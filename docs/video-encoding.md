# Hardware video encoding

Serein sends H.264 on every platform, encodes it in the driver's own NVENC session where one exists,
and can also send **AV1** when the driver offers an AV1 encoder. This note records the encoders the
official Discord client ships, what the two send formats configure, and what is still missing.

## What Serein sends today

Go Live video is H.264 by default and AV1 on request. `crates/discord-voice/src/video_codec.rs`
assigns payload 101 to H.264, 103 to H.265 and 105 to AV1 (RTX 102/104/106); H.265 stays receive-only
and only on Linux, AV1 is sent when `SEREIN_STREAM_CODEC=av1` selects it and the driver has an AV1
encoder. The stream's only encode-capable entry is the codec it sends, and it refuses a session that
negotiates another one.

Screen sharing and the camera share one encoder seam, `crates/discord-voice/src/video_encode.rs`
(`Config`, `Profile`, `Codec`, `hardware::Encoder`). On Windows that seam prefers the direct NVENC
session (`crates/discord-voice/src/video_encode_nvenc.rs`), then the Media Foundation transform
(`crates/discord-voice/src/video_encode_windows.rs`), and the caller keeps an openh264 encoder for
everything both refuse and for mid-stream hand-over. On macOS the seam is VideoToolbox
(`crates/discord-voice/src/video_encode_macos.rs`); on Linux the GStreamer pipeline encodes with
`vah264enc` or `nvh264enc` and falls back to software. The camera keeps H.264: every viewer decodes
it, and the camera has no AV1 packetizer.

What the NVENC backend configures, and why it is not just "hardware H.264":

- H.264 uses the P5 preset with the low-latency tuning; AV1 uses the P6 preset with the high-quality
  tuning. The encoder's `ULTRA_HIGH_QUALITY` tuning rejects a picture structure without B-frames on
  this driver (617.42, NVENC 13.1), so the quality tier is a follow-up, not a silent downgrade.
- Peak-constrained VBR: the transport feedback keeps owning the mean bitrate, the peak stays at 1.5×
  the mean, and the VBV buffer is 250 ms at the mean rate (NVENC measures it in bits, so the value is
  the bitrate quartered), mirroring the Media Foundation backend so congestion control behaves the
  same whichever encoder wins.
- Adaptive quantization on, driven by the preset's auto strength. Lookahead, temporal AQ and
  multi-pass stay off: each trades a picture of latency for rate accuracy, which an interactive
  screen share cannot spend.
- Main profile for screen sharing and Baseline for the camera, I/P pictures only, a keyframe every 10
  seconds while sharing and every 2 seconds on the camera, and SPS/PPS repeated with every IDR so a
  viewer that joined or lost packets can decode from any keyframe. AV1 repeats the sequence header on
  every IDR (`repeatSeqHdr`) for the same reason, and for one more: libwebrtc marks an AV1 packet as a
  keyframe only when the frame is a key frame **and** its first OBU is a sequence header, so a keyframe
  that repeats no header travels as a delta frame and a viewer that joined since the last one never
  starts — which is exactly what a viewer's connection error looks like.
- A live rate change through `nvEncReconfigureEncoder`, so a congestion cut does not cost an IDR.
- A device picture whose luma pitch is the visible width rounded up to four bytes, with the picture
  uploaded as two two-dimensional copies. NVENC refuses to register a CUDA picture with a pitch it
  does not like, and the screen settings round a shared source to an even width only, so without
  this a 1366×768 display — or any other source two bytes past a multiple of four — would have no
  hardware encoder at all.

## The AV1 send path

AV1 changes the framing around the encoder, not just the codec:

- The encoder emits an OBU stream. `crates/discord-voice/src/video.rs` (`av1_sender_frame`) rewrites it
  into the framing the receiving side rebuilds: every OBU keeps its size field except the last, whose
  size the frame boundary implies. DAVE encrypts exactly those bytes, so sender and receiver must
  agree byte for byte.
- That framing drops the OBUs DAVE drops before encrypting (temporal delimiter, tile list, padding).
  Left in, a trailing padding OBU would mean the last coded OBU keeps its size field, and the
  authentication data DAVE appends behind it would be parsed as one more OBU.
- `packetize_av1` produces AOM AV1 RTP v1.0.0 payloads: one aggregation header per packet, then
  size-prefixed elements, `Z`/`Y` marking an OBU that continues across packets, `N` on the packet that
  carries the sequence header, and the marker bit on the final packet. Payload type 105, RTX 106. A
  payload never carries more than 64 elements, the receiver's own budget, even when the bytes would
  still fit.
- `stream_codecs` advertises AV1 with `encode: true` and lists it first so the server picks it;
  `stream_codec` accepts the answer and the sender refuses a session that negotiates a different codec
  instead of sending mislabeled media.
- DAVE encrypts with `davey::Codec::AV1`, which keeps the OBU headers and size fields in the clear and
  encrypts the payloads, exactly as the receive path expects.

The receive side of this repository still decodes AV1 only on Linux (`available_av1`), so another
Serein client on Windows or macOS has no AV1 decoder. A live test against an official viewer showed
that the official client does not watch this AV1 stream either, while it watches the same share over
H.264 without trouble; `docs/video-encoding.md`'s "Interoperation" section records the evidence. That is
why AV1 is an opt-in rather than the default, and why the next step is a Windows AV1 decoder through the
Media Foundation transform — a decoder here would at least make a Serein-to-Serein share work.

An AV1 share has no software substitute. The transport labels every frame with the negotiated codec,
so `ScreenEncoder` refuses to start an AV1 share without a hardware encoder and stops the share if the
encoder dies mid-stream, instead of handing openh264 pictures to an AV1 stream. H.264 keeps the
mid-stream hand-over.

## What the official client does

Discord 1.0.9270 splits encoding across two processes: `discord_voice.node` owns capture, encoder,
encryption and packetization, and `gpu_encoder_helper.exe` runs only to answer whether a given GPU
vendor encodes a given codec from a given kind of source. It names eighteen hardware encoders —
NVIDIA CUDA/DX11 × H.264/H.265/AV1, AMD DX11, Intel QSV, and Media Foundation — and pins encoding to
the capture adapter by LUID.

`docs/discord-video-encoding.md` reads both binaries component by component: the probe contract, the
NVENC and AMF and QSV and Media Foundation backends, the shared D3D11 device and bounded surface
pool, the capture backends, and the per-stage encoder statistics. Two facts from it shape this
repository's work:

- The Media Foundation backend is driven **asynchronously**, enumerates transforms with `MFTEnum2`,
  filters them by adapter LUID, rejects candidates without dynamic format change or async mode, drains
  with a deadline on shutdown, and detects keyframes by parsing the bitstream (`Parse bitstream to
  detect IDR`).
- NVENC codec availability is gated per codec with a readable refusal, not by the API version alone
  (`nvenc: AV1 codec not supported on this driver version`), and the encoder session is modelled as a
  configuration of codec, preset, rate-control mode, tuning, multi-pass, VBV size and IDR period.

The strings establish which controls exist in the client; they do not establish which values a Go
Live stream receives, because the configuration object is shared with the camera and the codec choice
itself lives in a web bundle fetched at run time. Where this document says the client differs from
Serein, it compares designs, not measured streams.

Serein's NVENC backend follows that shape for the codec it sends: a per-codec capability check,
preset and tuning, peak-constrained VBR with a VBV buffer, and a live reconfiguration instead of a new
session. It does not follow the eighteen-name inventory (one backend per vendor, NVENC → Media
Foundation → software) and does not do the zero-copy CUDA/D3D11 hand-off yet, both deliberately. The
adoption list and its reasoning are at the end of `docs/discord-video-encoding.md`.

## What the backend needs to load

The crate links nothing at build time: `mini-nvcodec 0.2.0` (MIT OR Apache-2.0) declares the SDK 13.1
layouts and loads the driver's `nvEncodeAPI64.dll` at run time, and `mini-cuda 0.2.1` (MIT OR
Apache-2.0) supplies the CUDA driver table the session binds to. A machine without the library,
without an NVIDIA GPU, or below the NVENC 12.0 floor reports no encoder, and the Windows seam falls
through to Media Foundation and then to openh264 — the same behavior a machine with neither had
before. The probing line is printed once per process under `SEREIN_VOICE_DIAGNOSTICS=1`:

```
[Serein voice Video] nvenc: api=13.1 h264=true hevc=true av1=true
```

Measured on an RTX 5090 with driver 617.42: NVENC 13.1, H.264, HEVC and AV1 encode available. A
release-build measurement of one frame per call, including the NV12 upload, and the software encoder
for comparison on the same content:

| Picture | NVENC | openh264 |
| --- | --- | --- |
| 1920×1080 | 6.6 ms | 4.4 ms |
| 2560×1440 | 9.3 ms | 8.0 ms |
| 3840×2160 | 15.0 ms | 20.4 ms |

The hardware figure is wall-clock time on the encoder thread, mostly the two synchronous copies and
the driver wait, not CPU. At 60 fps it takes about 40% of the frame budget at 1080p; at 30 fps, a
fifth. The software numbers move a couple of milliseconds between runs on this machine, so read the
two columns as the same order of magnitude below 4K and a clear hardware win at 4K.

## Remaining work

**A Windows AV1 decoder.** The "AV1 Video Extension" transform is enumerable through
`MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, …, MFVideoFormat_AV1)` and activates, but it refuses every
output type with `MF_E_INVALIDMEDIATYPE` until its input type carries the stream's
`AV1CodecConfigurationRecord` in `MF_MT_USER_DATA`, which only exists after the sequence header
arrives — so negotiation has to be deferred to the first access unit. The transform also reports
`MF_TRANSFORM_ASYNC`, so an event-driven output loop is likely needed on top. A portable alternative
is `rav1d`, the Rust port of the dav1d decoder the official client ships: one dependency for Windows,
macOS and Linux instead of a per-platform transform, at the cost of a large pure-Rust build. macOS has
no AV1 receive path at all today.

**H.265 sending.** The same packetization work as AV1 (H.265 RTP follows RFC 7798, not the AV1 payload
format) plus a decoder on every platform.

**Quality mode.** The encoder's `ULTRA_HIGH_QUALITY` tuning is documented for AV1 from the NVENC 13.0
headers and rejects a picture structure without B-frames on this driver (617.42, NVENC 13.1), so
turning it on needs B-frames — and B-frames need an output queue on the sender and the reorder buffer
on the receiver. The other rate-accuracy controls carry their own limits: `lookaheadDepth` accepts at
most `31 - B frames` and only with `enableLookahead` set; multi-pass means
`NV_ENC_TWO_PASS_QUARTER_RESOLUTION` or `NV_ENC_TWO_PASS_FULL_RESOLUTION`; `targetQuality` is `0` for
automatic, with `0-51` above it for H.264 and H.265 and `0-63` for AV1. All of them trade latency for
rate accuracy, so they belong to an explicit quality setting rather than the default.

**Interoperation.** An AV1 share against an **official Discord viewer** does not work, and the failure
is not on the sending side. With `SEREIN_VOICE_DIAGNOSTICS=1` the sender reports
`[Serein voice Stream] codec=AV1 payload=105 rtx=106`, then
`[Serein voice Stream] first frame bytes=112738 keyframe=true packets=99`, and its per-window counters
keep `session_av1=1` with `dave_ready`, `group_ready`, `announced` and `capture_ready` all advancing for
the whole share — the session is negotiated as AV1, DAVE is ready and frames go out. The viewer still
never draws a picture and shows Discord's error **2012**, which Discord documents as a video viewer
timeout: it receives nothing it can decode. Sharing the same screen, at the same quality, over H.264
from the same machine is watched by that viewer normally.

So AV1 stays an opt-in and is not a supported way to share. The wire format itself is not the
problem: `the_reference_depacketizer_accepts_the_av1_packets_we_send` ports libwebrtc's
`video_rtp_depacketizer_av1.cc` and shows it accepts these payloads — for a keyframe carrying a
sequence header and for a delta frame — and assembles the same OBUs the sender framed. What remains
unseparated is whether the official client advertises AV1 for a Go Live stream at all, and whether
Discord relays AV1 from a sender that is not its own client; both are decisions on its side, and
telling them apart needs that client's own log.

**Other vendors.** AMD's AMF and Intel's QSV expose the same shape of controls. The seam added here
takes one more backend each; neither is planned yet.
