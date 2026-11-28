# Hardware video encoding

Serein sends H.264 on every platform, encodes it in the driver's own NVENC session where one exists,
and can also send **AV1** when the driver offers an AV1 encoder. This note records the encoders the
official Discord client ships, what the two send formats configure, and what is still missing.

## What Serein sends today

Go Live video is H.264 by default and AV1 on request. `crates/discord-voice/src/video_codec.rs`
assigns payload 101 to H.264, 103 to H.265 and 105 to AV1 (RTX 102/104/106); H.265 stays receive-only
and only on Linux, AV1 is sent when `SEREIN_STREAM_CODEC=av1` selects it and the driver has an AV1
encoder. The stream advertises exactly the codec it encodes and refuses a session that negotiates
another one.

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
  the mean, and the VBV buffer is 250 ms at the mean rate, mirroring the Media Foundation backend so
  congestion control behaves the same whichever encoder wins.
- Adaptive quantization on, driven by the preset's auto strength. Lookahead, temporal AQ and
  multi-pass stay off: each trades a picture of latency for rate accuracy, which an interactive
  screen share cannot spend.
- Main profile for screen sharing and Baseline for the camera, I/P pictures only, a keyframe every 10
  seconds while sharing and every 2 seconds on the camera, and SPS/PPS repeated with every IDR so a
  viewer that joined or lost packets can decode from any keyframe.
- A live rate change through `nvEncReconfigureEncoder`, so a congestion cut does not cost an IDR.

## The AV1 send path

AV1 changes the framing around the encoder, not just the codec:

- The encoder emits an OBU stream. `crates/discord-voice/src/video.rs` (`av1_sender_frame`) rewrites it
  into the framing the receiving side rebuilds: every OBU keeps its size field except the last, whose
  size the frame boundary implies. DAVE encrypts exactly those bytes, so sender and receiver must
  agree byte for byte.
- `packetize_av1` produces AOM AV1 RTP v1.0.0 payloads: one aggregation header per packet, then
  size-prefixed elements, `Z`/`Y` marking an OBU that continues across packets, and the marker bit on
  the final packet. Payload type 105, RTX 106.
- `stream_codecs` advertises AV1 with `encode: true` and lists it first so the server picks it;
  `stream_codec` accepts the answer and the sender refuses a session that negotiates a different codec
  instead of sending mislabeled media.
- DAVE encrypts with `davey::Codec::AV1`, which keeps the OBU headers and size fields in the clear and
  encrypts the payloads, exactly as the receive path expects.

The receive side of this repository still decodes AV1 only on Linux (`available_av1`), so another
Serein client on Windows or macOS has no AV1 decoder. Whether Discord's SFU transcodes per viewer or
rejects those viewers is unverified; that is why AV1 is an opt-in rather than the default, and why the
next step is a Windows AV1 decoder through the Media Foundation transform.

## What the official client does

Evidence is limited to the shipped client, Discord 1.0.9270 on Windows. `modules/discord_voice-1/discord_voice/discord_voice.node`
contains a Rust video stack, and its strings name the components. The web bundle that decides which
codec a stream advertises is fetched at run time and its cache files were locked by the running
client, so the protocol-level codec choice is **unverified** here.

Encoders and codecs, from the module's strings:

- Backends: `Nvidia/NVENC CUDA H.264 encoder (HW)`, `Nvidia/NVENC DX11 H.264 encoder (HW)`,
  the same pair for H.265 and AV1, `AMD/AMF DX11 H.264/H.265/AV1 encoder (HW)`,
  `Intel/QSV AV1 encoder (HW)`, `MediaFoundation H.264/H.265/AV1 encoder (HW)`, plus
  `MediaFoundation SW` and CPU paths.
- Codec set: `H264H265AV1` appears as the encoder selection and as
  `DimensionswidthheightH264H265AV1`; `Failed to create video encoder because we only suppport H.264, H.265 and AV1`.
- Capability probing per vendor: `GpuEncoderSupport`, `NvidiaGpuEncoderProbeResult`,
  `AmdGpuEncoderProbeResult`, `IntelGpuEncoderProbeResult`,
  `WindowsMediaFoundationGpuEncoderProbeResult`, `Enumerating hardware encoders for codec:`, and
  per-driver refusals `nvenc: AV1 codec not supported on this driver version` and
  `nvenc: H.265 codec not supported on this driver version`.
- NVENC knobs the client itself keeps: `NVENC config, codec:`, `latency:`, `preset:`, `vbv buffer size:`,
  and the tokens `Preset rc_mode tuning multi_pass` with `Low Medium High Unlimited` and `UltraLow`.
  There is no reference to lookahead depth, temporal AQ, multipass full-res or ultra-high-quality
  tuning, so the heavy NVENC tuning that Blackwell enables is not something the official client turns
  on.
- Rate control and quality surveillance per codec: `Encoded.Qp.Vp9`, `Encoded.Qp.H265`, `Encoded.Qp.Av1`
  with `.S0`, `.S1`, `.S2` spatial layers, `WebRTC-QCM-Static-AV1`, `WebRTC-QCM-Dynamic-AV1`,
  `WebRTC-AV1-OverridePriorityBitrate`, `av1_kbps`, `av1_qp_low`, `av1_qp_high`.
- RTP: `rtp_packetizer_h265.cc`, `rtp_packetizer_av1.cc`,
  `aomediacodec.github.io/av1-rtp-spec`, `dav1d_decoder.cc`, `WebRTC-H265-QualityScaling` — the same
  AV1 dependency-descriptor layout the receive path in this repository already parses.
- Capture: `graphics_capture_window_source`, `graphics_capture_display_source`, `dxgi_window_source`,
  `dxgi_display_source`, `gdi_window_source` (`BitBlt` and `PrintWindow`), `video_hook`,
  `capture backend in use:`, and `hdr_tonemapped` on the captured surface.

The official client picks a codec after probing the GPU vendor's encoder, configures a preset, a
rate-control mode, a tuning, an optional multi-pass and a VBV buffer, and keeps per-codec QP
statistics for its quality controller. Serein's NVENC backend follows that shape for the codec it
sends: probe, preset and tuning, peak-constrained VBR with a VBV buffer, and a live reconfiguration
instead of a new session.

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
| 1920×1080 | 7.5 ms | 4.0 ms |
| 2560×1440 | 9.6 ms | 7.3 ms |
| 3840×2160 | 15.7 ms | 17.5 ms |

The hardware figure is wall-clock time on the encoder thread, mostly the synchronous copy and the
driver wait, not CPU. At 60 fps it takes about half the frame budget at 1080p; at 30 fps a third.

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

**Quality mode.** The encoder's `ULTRA_HIGH_QUALITY` tuning needs B-frames, and B-frames need an
output queue on the sender and the reorder buffer on the receiver before they can be turned on. The
same applies to lookahead and multi-pass: they trade latency for rate accuracy, so they belong to an
explicit quality setting rather than the default.

**Interoperation.** AV1 sending has been verified synthetically only: the sender's packets round-trip
through this repository's own receiver, and the encoder's output is OBU-framed with a keyframe first.
A live call against an official client is required before the opt-in becomes a default.

**Other vendors.** AMD's AMF and Intel's QSV expose the same shape of controls. The seam added here
takes one more backend each; neither is planned yet.
