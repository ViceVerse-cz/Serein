# Hardware video encoding

Serein already encodes H.264 on the GPU where the platform offers it. This note records what the
official Discord client does, what the current encoder path can and cannot reach, and the plan for
a direct NVENC backend that exposes the quality controls of the RTX 40/50 (Ada and Blackwell) NVENC
generation. **Everything under "Plan" is intended work, not shipped behavior.**

## What Serein sends today

Go Live video is H.264 only: `crates/discord-voice/src/video_codec.rs` assigns payload 101 to H.264
and the transport advertises `encode: true` for it alone, while H.265 (103) and AV1 (105) are
receive-only and only on Linux (`available_h265`/`available_av1`). `crates/discord-voice/src/video.rs`
is a small Annex-B H.264 packetizer (RFC 6184 single NAL and FU-A); there is no AV1 or H.265
packetizer for sending.

Screen sharing and the camera share one hardware encoder seam, `crates/discord-voice/src/video_encode.rs`
(`Config`, `Profile`, `hardware::Encoder`). A `ScreenEncoder` prefers it and keeps an openh264 encoder
for machines and pictures the hardware refuses, handing over to openh264 mid-stream when the hardware
fails.

- **Windows**: a Media Foundation H.264 transform (`crates/discord-voice/src/video_encode_windows.rs`)
  with the driver's encoder, peak-constrained VBR, no B-frames, `CODECAPI_AVLowLatencyMode`, a keyframe
  period from the profile, and NV12 input converted on the CPU.
- **macOS**: VideoToolbox H.264 (`crates/discord-voice/src/video_encode_macos.rs`).
- **Linux**: `vah264enc` or `nvh264enc` inside the GStreamer capture pipeline, with a software
  fallback.

So on a machine whose driver ships an H.264 transform — the RTX 5090 with driver 617.42 this note was
written on — the existing Windows path encodes H.264 in hardware already; nothing in the UI or the
diagnostics says which transform was chosen, which is why P0 adds that line. What the path cannot
reach is everything below: Media Foundation's codec API exposes a bitrate, a GOP and a quality-vs-speed
knob, not NVENC's rate-control and tuning fields.

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
  tuning, so the heavy NVENC tuning discussed for Blackwell is not something the official client
  turns on.
- Rate control and quality surveillance per codec: `Encoded.Qp.Vp9`, `Encoded.Qp.H265`, `Encoded.Qp.Av1`
  with `.S0`, `.S1`, `.S2` spatial layers, `WebRTC-QCM-Static-AV1`, `WebRTC-QCM-Dynamic-AV1`,
  `WebRTC-AV1-OverridePriorityBitrate`, `av1_kbps`, `av1_qp_low`, `av1_qp_high`.
- RTP: `rtp_packetizer_h265.cc`, `rtp_packetizer_av1.cc`,
  `aomediacodec.github.io/av1-rtp-spec`, `dav1d_decoder.cc`, `WebRTC-H265-QualityScaling` — the same
  AV1 dependency-descriptor layout the receive path in this repository already parses.
- Capture: `graphics_capture_window_source`, `graphics_capture_display_source`, `dxgi_window_source`,
  `dxgi_display_source`, `gdi_window_source` (`BitBlt` and `PrintWindow`), `video_hook`,
  `capture backend in use:`, and `hdr_tonemapped` on the captured surface.

The picture that emerges: the official client picks a codec after probing the GPU vendor's encoder
(NVENC, AMF, QSV, Media Foundation), configures a preset, a rate-control mode, a tuning, an optional
multi-pass and a VBV buffer, and keeps per-codec QP statistics for its quality controller. It does
**not** appear to use the aggressive AV1 tuning presets that Blackwell enables.

## What a direct NVENC backend adds

- Rate control the Media Foundation transform cannot express: VBV with separate average and maximum
  rates, CQ/target-quality mode, lookahead, adaptive quantization including temporal AQ, and
  multi-pass.
- Tuning info, including ultra-high quality for AV1 and HEVC on Ada and Blackwell (H.264 keeps the
  high-quality tuning).
- Encode for H.265 and AV1 on the same session type, which is the prerequisite for sending those
  codecs at all.
- One less media-session hop: the frame goes from our NV12 buffer into a D3D11 texture owned by this
  process, not through a Media Foundation transform graph.

It does not remove the H.264 hardware encoder that already works, and it does not by itself change
what the stream carries: payload 101 and H.264 remain the only negotiated send format until the
protocol side below moves.

## Plan

**P0 — capability probe, no behavior change.** A new `crates/discord-voice/src/video_encode_nvenc.rs`
loads `nvEncodeAPI64.dll` from the driver, reads the API version with
`NvEncodeAPIGetMaxSupportedVersion`, creates an instance, and records which encode GUIDs the machine
offers (`NV_ENC_CODEC_H264_GUID`, `HEVC`, `AV1`), the maximum dimensions, and the driver version.
The probe runs once per process, caches its answer, and prints one line under
`SEREIN_VOICE_DIAGNOSTICS=1` in the shape the official client uses (`nvenc: h264=yes hevc=yes av1=yes`).
The same commit names the Media Foundation transform the current path activates, so the existing
hardware path becomes measurable instead of assumed. Pure parts (capability record, preset and
rate-control mapping) get unit tests; the probe itself is verified by running it on the RTX 5090 here.

**P1 — NVENC H.264 for screen sharing.** `Encoder` gains the NVENC backend behind the existing
`video_encode::Config` interface, so the call sites in `crates/discord-voice/src/screen.rs` and
`crates/discord-voice/src/camera.rs` do not change. Order of preference becomes NVENC, Media
Foundation, openh264. Input stays NV12 through `crates/discord-voice/src/screen/convert.rs`, uploaded
to one reusable D3D11 texture; the transport-feedback target becomes the VBV maximum and the average
stays below it, so congestion control keeps working. Keyframes stay on the two-second period the
software encoder uses, B-frames stay off, and `NV_ENC_RECONFIGURE_PARAMS` applies bitrate changes
without a new IDR where the driver allows it.

**P2 — camera.** The same backend with the Baseline profile the camera sends today, and the same
fallback chain. This is a configuration change once P1 lands.

**P3 — H.265 and AV1 sending.** This is protocol work, not encoder work: `stream_codecs` must advertise
`encode: true` for the chosen codec, `crates/discord-voice/src/video.rs` needs H.265 and AV1
packetizers (the AV1 dependency descriptor is the same layout `video_av1.rs` and the receive path
already understand), and keyframe and parameter-set detection already exists in `video_codec.rs`.
It also needs **decoders on Windows and macOS**, because `available_av1` and `available_h265` are
Linux-only today: Serein clients would not be able to decode each other's H.265 or AV1. Interoperation
with the official client must be verified live before this ships; it is the one part of the plan that
cannot be established synthetically.

**P4 — quality controls.** Presets that expand into encoder settings rather than only a bitrate:
a latency-first setting (no B-frames, no lookahead, CQ off) and a quality setting (lookahead, temporal
AQ, multi-pass) per codec, with the transport-feedback ceiling still enforced. UI work stays inside
`crates/ui/src/screen.rs` and the settings catalogs.

## Tuning mapping

The NVENC settings discussed for Blackwell, as ffmpeg spells them, map onto `nvEncodeAPI.h` fields.
Field names are quoted from the SDK header as of NVENC 12.x and are confirmed against the installed
header during P0.

| ffmpeg option | NVENC API | Note |
| --- | --- | --- |
| `-tune uhq` | `NV_ENC_INITIALIZE_PARAMS::tuningInfo = NV_ENC_TUNING_INFO_ULTRA_HIGH_QUALITY` | Ada and Blackwell, AV1 and HEVC |
| `-temporal-aq 1` | `NV_ENC_CONFIG::rcParams.enableTemporalAQ` | needs lookahead to be useful |
| `-rc-lookahead 32` | `rcParams.lookaheadDepth` | about half a second at 60 fps; quality mode only |
| `-multipass fullres` | `rcParams.multiPass = NV_ENC_MULTI_PASS_ENABLED` | more encoder load, better rate accuracy |
| `-b_adapt 1` | codec config B-frame adaptivity | only meaningful with B-frames |
| `-rc vbr -cq 35` | `rateControlMode = NV_ENC_PARAMS_RC_VBR`, `rcParams.targetQuality` | 0–51 scale, lower is better |
| `-minrate 0 -maxrate 3M` | `rcParams.averageBitRate`, `rcParams.maxBitRate`, `rcParams.vbvBufferSize` | maximum must stay under the transport ceiling |

## Risks and evidence needed

- **Unsafe code.** `crates/discord-voice` denies `unsafe_code`; the NVENC module needs a module-level
  `#![allow(unsafe_code)]` with a safety comment, the same shape `video_encode_windows.rs` uses today.
  The vtable is loaded by hand, so a wrong struct layout faults on the first call — P0 must verify the
  instance against `NvEncodeAPIGetMaxSupportedVersion` before any session opens.
- **Driver variance.** Older drivers lack AV1 or HEVC encode and some lack the tuning enums; every
  setting must be requested through a capability check and degrade to the next backend, never fail a
  share.
- **NVENC session limits** on consumer drivers restrict concurrent sessions. One share is one session,
  but camera plus screen plus a second call must not fight for it silently.
- **Rate-control conflict.** The transport feedback controller currently owns the bitrate target. A CQ
  mode that ignores `maxBitRate` would break congestion response, so CQ stays inside the ceiling.
- **Latency.** Lookahead and multi-pass trade latency for rate accuracy; a 32-frame lookahead is about
  0.5 s at 60 fps, unacceptable as a default for interactive sharing. Quality mode must be an explicit
  choice, not the default.
- **Measurement.** Claims of "better quality at the same bitrate" need a comparison against the
  current Media Foundation path and openh264 on the same synthetic content, at the same bitrate and
  frame rate, with file size, PSNR/SSIM and encode time reported in `docs/performance.md` when the
  feature ships.
- **Interoperability.** H.264 over payload 101 is already the working send format; H.265 and AV1
  sending depend on the official client accepting them from an unofficial sender and on receivers
  decoding them, which only a live call can establish.
