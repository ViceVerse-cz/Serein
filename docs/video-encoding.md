# Hardware video encoding

Serein sends H.264 on every platform, and on Windows it now encodes it in the driver's own NVENC
session where one exists, keeping the Media Foundation transform and the openh264 software encoder as
fallbacks. This note records the encoders the official Discord client ships, what the NVENC backend
configures, and what is still missing before the stream itself can carry H.265 or AV1.

## What Serein sends today

Go Live video is H.264 only: `crates/discord-voice/src/video_codec.rs` assigns payload 101 to H.264
and the transport advertises `encode: true` for it alone, while H.265 (103) and AV1 (105) are
receive-only and only on Linux (`available_h265`/`available_av1`). `crates/discord-voice/src/video.rs`
is a small Annex-B H.264 packetizer (RFC 6184 single NAL and FU-A); there is no AV1 or H.265
packetizer for sending.

Screen sharing and the camera share one encoder seam, `crates/discord-voice/src/video_encode.rs`
(`Config`, `Profile`, `hardware::Encoder`). On Windows that seam prefers the direct NVENC session
(`crates/discord-voice/src/video_encode_nvenc.rs`), then the Media Foundation transform
(`crates/discord-voice/src/video_encode_windows.rs`), and the caller keeps an openh264 encoder for
everything both refuse and for mid-stream hand-over. On macOS the seam is VideoToolbox
(`crates/discord-voice/src/video_encode_macos.rs`); on Linux the GStreamer pipeline encodes with
`vah264enc` or `nvh264enc` and falls back to software.

What the NVENC backend configures, and why it is not just "hardware H.264":

- The P5 preset tier with the low-latency tuning, which is the pair the official client exposes as a
  preset and a latency setting.
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

**H.265 and AV1 sending.** This is protocol work, not encoder work: `stream_codecs` must advertise
`encode: true` for the chosen codec, `crates/discord-voice/src/video.rs` needs H.265 and AV1
packetizers (the AV1 dependency descriptor is the same layout `video_av1.rs` and the receive path
already understand), and keyframe and parameter-set detection already exists in `video_codec.rs`. It
also needs **decoders on Windows and macOS**, because `available_av1` and `available_h265` are
Linux-only today: Serein clients would not be able to decode each other. Interoperation with the
official client must be verified live; it cannot be established synthetically.

**Quality controls.** The probe already reports HEVC and AV1 support, so a quality setting per codec
(lookahead, temporal AQ and multi-pass for a quality-first mode, or NVENC's ultra-high-quality tuning
where the driver advertises it) is a configuration change on top of this backend, gated by the
transport ceiling. It should stay an explicit choice: a 32-frame lookahead is about half a second of
latency at 60 fps.

**Other vendors.** AMD's AMF and Intel's QSV expose the same shape of controls. The seam added here
takes one more backend each; neither is planned yet.
