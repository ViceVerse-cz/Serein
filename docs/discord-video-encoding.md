# How the official client encodes screen video

This note describes how the shipped Discord client turns a captured screen into an encoded video
stream, so that `docs/video-encoding.md` can say what Serein shares with it and where Serein is
deliberately different. Nothing here is copied from the client: it is a reading of the two Windows
binaries that implement the path, written for someone who has to make the same decisions.

## Scope and how the claims were obtained

- **What was read.** `modules/discord_voice-1/discord_voice/discord_voice.node` (22.6 MB, the Node
  addon that carries Discord's internal `discord_media` Rust video stack) and
  `gpu_encoder_helper.exe` (0.93 MB, the out-of-process encoder probe that ships beside it), from
  Discord **1.0.9270** on Windows. Both are unsigned-by-this-repository artifacts of a third-party
  product; treat them as evidence, not as a specification.
- **How.** Import and export tables, section layout, the string table (which in both binaries
  retains Rust crate paths such as `components/video/coding/nvidia-media-encoding/src/native/config.rs`),
  the `clap` argument definitions of the helper, and targeted disassembly of the frame-submission
  path. The Rust stack is not stripped enough to hide its module names, which is why the component
  layout below is readable at all.
- **Confidence labels.** *Observed* means a string, symbol or table entry in the binary states it.
  *Inferred* means the behavior follows from those strings plus the published API contract of the
  vendor library involved, without a runtime trace. *Unverified* means the binary is consistent with
  it but nothing here proves it.
- **What is not claimed.** Which settings the web bundle asks for (that code is fetched at run time
  and its cache is locked by the running client), what the SFU does with each codec, and any
  behavioral comparison against another Discord build.

## The two processes

Encoding is split across a process boundary on purpose:

| Process | Role |
| --- | --- |
| `discord_voice.node` | Owns capture, the encoder factory, frame encryption and packetization. Loads the vendor API and runs the encoder in-process. |
| `gpu_encoder_helper.exe` | Runs *only* to answer "does this GPU vendor encode this codec from this kind of source?" and then exits. |

The helper is a Rust binary built with `clap`, version `0.1.0`, whose own `--version` metadata carries
`chao.chen@discordapp.com`, and whose embedded source path is
`discord_desktop\native_modules\discord_voice\gpu_encoder_helper\src\main_win.rs`. Its `about` text
is the clearest sentence in either binary about its job: *"A helper process to probe the supported
codecs of a gpu vendor"*.

The reason to pay for a second process is fault isolation, and the binary supports the reading
*inferred*: the helper does not merely inspect a driver version, it constructs a real encoder. Its
strings are the log lines of a full initialization — `Creating MediaFoundationGpuVideoEncoder, size:,
fps:, bitrate:, format:, codec:`, `MediaFoundationGpuVideoEncoder initialized`,
`Creating MediaFoundationDx11VideoEncoder`, `MediaFoundationDx11VideoEncoder initialized` — and it
carries the same AMF, QSV and CUDA loaders the node has. A driver that hangs, crashes or leaks inside
a vendor transform therefore costs the helper process, not the call.

Helper command line, *observed* through the `clap` definitions:

- required `vendor`, one of `nvidia`, `intel`, `amd`, `windows-media-foundation`;
- the codec set `h264`, `h265`, `av1`;
- a source mode, one of `cuda`, `dx11`, `basic` — the three ways the client can hand a frame to an
  encoder: a CUDA buffer, a D3D11 texture, or a plain CPU surface;
- a result reported as `Success` or `Error`.

The probe answers per tuple, not per vendor: `Nvidia/NVENC CUDA H.264` succeeding does not imply
`Nvidia/NVENC DX11 AV1`, which is why the factory in the node is keyed on all three components. The
adapter identity it reports is DXGI's — `vendor_id`, `device_id`, `sub_system_id`, `revision`, with
`IDXGIAdapter1` and `ID3D11Device` in the same module — so a probe is tied to one physical adapter
in a multi-GPU machine rather than to "the GPU".

## Encoder inventory

The node names eighteen hardware encoders. The names are format strings consumed by the settings UI,
and they encode the tuple the factory dispatches on — vendor, transport, codec:

| Vendor | Transports | Codecs |
| --- | --- | --- |
| NVIDIA | `CUDA`, `DX11` | H.264, H.265, AV1 |
| AMD | `DX11` | H.264, H.265, AV1 |
| Intel | default, `Dx11` | H.264, H.265, AV1 |
| Microsoft | Media Foundation | H.264, H.265, AV1 |

The vendor column is not cosmetic: NVENC and AMF and QSV are three different APIs with three
different lifecycles, and the fourth row is not a vendor at all but the generic Windows transform
path that any driver can register. The `(HW)` suffix on every name implies a software counterpart
exists in the same selection; the strings of this build name none of the software encoders the same
way, so their set is unverified here.

Two consequences worth stating for our own design:

- Only NVIDIA offers a CUDA transport. Everyone else is D3D11, so the D3D11 path is the one that has
  to work everywhere and the CUDA path is an optimization.
- Media Foundation is a *fallback vendor*. It is the only row that can produce an encoder on a
  machine whose vendor library is missing, and it is the only row where "the driver registered a
  transform" rather than "the vendor runtime answered" is the availability signal.

## The NVIDIA path

Two layers, both visible in the node:

**Driver load, once per process** (`nvidia-media-encoding/src/native/library.rs`). The module resolves
`NvEncodeAPIGetMaxSupportedVersion` and `NvEncodeAPICreateInstance` from `nvencodeapi64.dll` (or the
32-bit `nvencodeapi.dll`), behind a `lazy_static`. It keeps an error taxonomy that names the failure
modes we also have to distinguish: `UnsupportedDriverVersion`, `GpuTexturesUnsupported`,
`TextureOnWrongDevice`, `EventCreationFailed`, `WaitFailed`, `EncodeSessionClosed`,
`SurfaceCreationFailed`, `InvalidSurfacePool`, `InvalidNativeEncoder`, plus `EncodeDimension*` for
unsupported picture sizes.

**Codec availability is version-gated, and says so.** The node contains
`nvenc: H.265 codec not supported on this driver version` and
`nvenc: AV1 codec not supported on this driver version`. So the client does not infer "this driver
has AV1" from the API version alone; it has a per-codec floor and reports the refusal in words a user
can read. That is a cheaper and clearer contract than a generic `ERR_UNSUPPORTED`.

**Configuration** (`nvidia-media-encoding/src/native/config.rs`). The one configuration log line in
the binary is `NVENC config, codec: <c>, preset: <p>, vbv buffer size: <n>`, and the string table
around it carries `Preset`, `rc_mode`, `tuning`, `multi_pass` as separate tokens, plus the values
`Low`, `Medium`, `High`, `Unlimited`, `UltraLow`, `None`, `Some`. `IDRPeriod` appears as a field of
the same configuration, next to per-codec override names.

*Inferred* from that shape: the client models the NVENC session as a small configuration object
(codec, preset, rate-control mode, tuning, multi-pass, VBV size, IDR period, and per-codec
overrides) and applies it at session creation.

**Preset tiers** (file offsets `0xe5e388`–`0xe5e3e8`, six 16-byte GUIDs, *observed*). The image
carries a table of `NV_ENC_PRESET_LOW_LATENCY_HP`, `P1`, `NV_ENC_PRESET_LOW_LATENCY_HQ`,
`NV_ENC_PRESET_HQ`, `P4` and `P6`, in that order, followed directly by the capture-source names. It
mixes the legacy `hq`/`ll*` presets with the modern P-series, and `P2`, `P3`, `P5` and `P7` are absent
from the whole file. The quality tokens visible in the strings are five (`Low`, `Medium`, `High`,
`Unlimited`, `UltraLow`), so either the table is not that enum or one tier has no name in this build;
the strings do not say which.

*Unverified*: which entry a Go Live share selects, and whether this table is what the configuration
indexes at all. The configuration object is shared with the camera and other consumers, so the
presence of a tuning token does not mean the stream path sets it, and the table has no direct code
reference to follow (see "Open questions").

**Frame submission** (`nvidia-media-encoding/src/native/input_frame.rs`). The failure string
`couldn't register input frame with nvenc` shows the texture is registered as an input resource per
frame rather than copied through system memory, which is the same zero-copy intent as our
`nvEncRegisterResource` + CUDA picture path. The CUDA dependency list in the node is the tell for
*how* the transport can be either CUDA or D3D11: `cuMemAllocPitch_v2`, `cuMemcpy2D_v2`/`...Async`,
`cuGraphicsD3D11RegisterResource`, `cuGraphicsMapResources`,
`cuGraphicsResourceGetMappedPointer`, `cuGraphicsSubResourceGetMappedArray`,
`cuImportExternalMemory`, `cuExternalMemoryGetMappedBuffer`/`...MipmappedArray`,
`cuImportExternalSemaphore`, `cuSignalExternalSemaphoresAsync`,
`cuWaitExternalSemaphoresAsync`, all from `nvcuda.dll`. The external-memory and external-semaphore
entries are the Vulkan-style hand-off; a D3D11 texture becomes either a registered graphics resource
or an imported external buffer, and synchronization is by semaphore rather than by a full context
sync.

The tokens `nvRelaxRc` sits in the loader's string block. Its meaning is *unverified*; it is not an
NVENC API symbol, and reading it as a release-control relaxation switch is a guess, so this repository
does not imitate it.

## The AMD path

`components/video/sys/amf/src/lib.rs` binds `amfrt64.dll` and resolves `AMFInit` and
`AMFQueryVersion`. The encoder components are the public AMF IDs, *observed* verbatim:
`AMFVideoEncoderVCE_AVC`, `AMFVideoEncoderHW_HEVC`, `AMFVideoEncoderHW_AV1`; H.264 goes through the
VCE (video-conversion-engine) component while H.265 and AV1 have dedicated hardware components, which
matches AMF's own naming and means the H.264 path has an extra conversion stage the others do not.

The most useful part of the binary for us is the property table: AMF is configured by *string*
property names, so the client's setting surface is readable directly. Grouped as they appear:

- Rate control: `RateControlMethod`, `TargetBitrate`, `VBVBufferSize`, `EnforceHRD`,
  `FillerDataEnable`, `RateControlPreanalysisEnable`, `RateControlSkipFrameEnable`,
  `EncodingLatencyMode`, `QueryTimeout`, per-codec variants `HevcRateControlMethod`,
  `HevcRateControlPreAnalysisEnable`, `HevcTargetBitrate`, `HevcVBVBufferSize`, `HevcEnforceHRD`,
  and the AV1 set `Av1RateControlMethod`, `Av1TargetBitrate`, `Av1VBVBufferSize`, `Av1EnforceHRD`,
  `Av1EncodingLatencyMode`, `Av1QueryTimeout`, `Av1RateControlSkipFrameEnable`.
- Picture structure: `BPicturesPattern`, `BReferenceEnable`, `MaxNumRefFrames`, `ForcePictureType`,
  `MaxAUSize`, with per-codec `Hevc*` and `Av1*` variants alongside the base names.
- Headers and format: `HeaderInsertionSpacing`, `ProfileLevel`, `OutputDataType`, `OutputFormat`,
  `FrameRate`, `AlignmentMode`, `WidthAlignmentFactor`, `HeightAlignmentFactor`,
  `ForceInsertSequenceHeader`, `HevcInsertHeader`, `HevcMaxAUSize`, `HevcProfileLevel`,
  `Av1FrameRate`, `Av1OutputFrameType`, `Av1ForceFrameType`, `Av1ForceInsertSequenceHeader`.
- Quantizer floor: `HevcMinQP_I`, `HevcMinQP_P` and the base `MinQP_*` family.

Two behaviors are asserted by the code itself rather than by a property:

- `AMD encoder generated a show existing frame, this should not happen`
  (`amd-media-encoding/src/codec/av1.rs`) — a low-latency screen share must emit one output per
  input, and AMF's "repeat the previous picture" mode breaks that invariant. Whoever wires AMF in
  has to check for it rather than trust the submit call.
- `AMFVideoConverter` plus `Dropping QsvD3d11AMFVideoConverter` — AMD's path includes a converter
  component, and the name collision with QSV suggests a shared D3D11-to-AMF-surface converter type in
  `discord_media`, not two independent implementations.

The surface is the same as NVIDIA's in shape: the encoder consumes a D3D11 texture, and AMF's
`InvalidSurfacePool`/`UnsupportedHardware`/`UnsupportedConfiguration`/`EncoderSubmitFailed`/
`DxEncoderStalled` errors are mirrored by QSV's identically named set, which is why the client can
report one "hardware encoder failed" message for three vendors.

## The Intel path

`components/video/sys/qsv/src/d3d11.rs` binds the Media SDK into a D3D11 device and logs `Device:`
and `Adapter:` when it picks one — the same adapter pinning as the NVIDIA path, for the same reason:
a texture captured on one adapter cannot be encoded on another without a copy.

The QSV row in the inventory is doubled (default and `Dx11`), which likely distinguishes a
D3D11-surface encoder from a texture-free one; *unverified*. The lifecycle is the HF-managed MFX
session, whose proxy functions are loaded at run time — `LibraryVersionTooOld` and
`NoVirtualFunctionPointer` are the two errors a Media SDK build can hit, so the client checks the
runtime library version and the dispatch table before creating a session, exactly as the NVENC
loader checks the API version.

## The Media Foundation path

This is the most interesting backend for Serein, because it is the generic one and the only row that
can exist on any machine. The client implements **two** encoders over it:

- `MediaFoundationGpuVideoEncoder` (`windows-media-foundation/src/gpu_encoder.rs`) — an MFT that
  accepts GPU surfaces; its log line names size, fps, bitrate, input format and codec.
- `MediaFoundationDx11VideoEncoder` (`windows-media-foundation/src/dx11_encoder.rs`) — a D3D11-aware
  encoder that reports `MediaFoundationDx11VideoEncoder does not support input format <f>` when the
  transform refuses the surface format it was given.

Their common layer (`src/common.rs`) shows how an **asynchronous** MFT is driven, and this is the
part that matters for any client that wants Media Foundation to be a real backend:

- Enumeration and eligibility (`src/util.rs`): `Enumerating hardware encoders for codec: <c>, input:
  <f>`, `Found N encoders`, `After filtering by LUID N encoders`, `Failed to set adapter LUID blob`,
  `Failed to enumerate MFTs`, then per candidate `Encoder idx:, guid:, name:, luid:` and the two
  filters `Encoder N does not support dynamic format change, trying next` and
  `Encoder N does not support async mode, trying next`, followed by
  `Failed to unlock async mode for encoder` and `Async encoder activated successfully:`. In other
  words the client enumerates *all* hardware transforms for the subtype, pins them to the capture
  adapter by LUID blob, then rejects candidates that cannot renegotiate a format mid-stream or run
  asynchronously, and unlocks async mode on the one it keeps.
- Event loop: `Unknown event type:`, `MEError event received`, `Error handling event:`,
  `Failed to re-register for async events:`. An async MFT delivers `METransformNeedInput` and
  `METransformHaveOutput` through an event generator, and re-arming it is a step that can itself
  fail, which is the failure this string reports.
- Shutdown: `Drain complete: all frames processed`, `Timeout waiting for drain complete (pending: N)`,
  `Dropped N input frames and M pending frames on encoder shutdown`, `Encoder dropping:` — draining
  the transform with a deadline and *counting* what was lost, instead of dropping the COM object and
  hoping.
- Keyframes are detected by parsing the output: `Parse bitstream to detect IDR`. The client does not
  trust the "I asked for a keyframe" flag; it reads the bitstream and knows.
- Accounting: a struct whose fields are `enc_time`, `wait_time`, `conversion_time`,
  `input_processing_time`, `output_processing_time`, `input_count`, `output_count`,
  `key_frame_count`, filled from `MF_EVENT_TYPE` timestamps. That is the data behind a "why is my
  stream late" diagnostic.

The API surface it binds — `MFStartup`, `MFShutdown`, `MFCreateAttributes`, `MFCreateSample`,
`MFCreateMemoryBuffer`, `MFFrameRateToAverageTimePerFrame`, `MFCreateDXGIDeviceManager`,
`MFCreateDXGISurfaceBuffer`, `MFTEnum2` from `mfplat.dll` — is the standard set for a
device-manager-driven GPU encode. Note `MFTEnum2` rather than `MFTEnumEx`: the newer entry point is
what takes the adapter LUID attribute used above.

## Frame hand-off, surfaces and the shared device

Both `discord_media` and this repository have to answer the same question: who owns the D3D11 device
the capture produces into and the encoder consumes from. The client's answer is visible in
`media_encoding/windows/utils`:

- `device_reuse_check: reference device is multi-threaded and device reuse enabled, reusing device`
  and `Encode reusing capture device` — the encoder runs on the capture device when that device was
  created multithreaded, so no cross-device copy happens.
- `device_reuse_check: device reuse disabled due to experiment override, setting encode device to
  multithreaded`, `Encoder device set to multithreaded`, `Encoder device failed to set multithreading:`
  — the alternative, behind a server-side experiment flag, is a separate encoder device created
  multithreaded.
- `Encoder device got ~*~ boosted ~*~` — a log line from the same path that names no API; *unverified*
  what it adjusts (thread priority is the obvious candidate).

Surface lifetime is bounded by a pool (`components/video/util/surface-pool/src/lib.rs`) with a
non-zero, fixed capacity — `capacity must be non-zero`, `couldn't acquire surface: no frame was
available`, `InvalidSurfacePool`, plus `couldn't create intermediate surface:` on the capture side.
A bounded pool with an explicit "no frame available" path is what keeps a slow encoder from growing
the capture queue without limit.

## Where frames come from

The capture side names six backends: `DXGI`, `GDI (BitBlt)`, `GDI (PrintWindow)`, `GDIW`, `WGC`,
`external`, `video hook`, with sources `dxgi_window_source`, `dxgi_display_source`,
`gdi_window_source`, `graphics_capture_window_source`, `graphics_capture_display_source`,
`video_hook_source`, `external_texture_source`. The switch between them is logged as
`capture backend in use: <new> (previous: <old>)`, i.e. the client degrades *while sharing* and
records the transition. `GDI Window Source prefers using PrintWindow` over `BitBlt` is a per-window
quality decision: `PrintWindow` renders occluded windows, `BitBlt` copies what is on screen.

Only the WGC and DXGI paths hand over a D3D11 texture natively; GDI produces a system-memory bitmap
that has to be uploaded, which is the same split our `capture_windows.rs` has between DXGI desktop
duplication and the GDI fallback.

## What this suggests for Serein

Adopt, in rough order of value:

1. **Probe out of process.** Our `nvenc::support()` runs inside the voice process. A vendor runtime
   that faults while probing should not take a call with it; a tiny probe executable that answers
   `vendor × codec × source` is the cheap version of what the client does, and it also gives us the
   adapter identity to pin encoding to the capture adapter.
2. **Per-codec driver floors with readable refusals.** We already refuse AV1 when the driver lacks
   it (`AV1_UNAVAILABLE`); the client's phrasing per codec is a better message than one shared
   string.
3. **Parse keyframes instead of trusting flags.** `Parse bitstream to detect IDR` is a stronger
   invariant than our `force_key_frame` bookkeeping, and it makes the receiver's "can I start here"
   decision testable against real encoder output.
4. **Drain with a deadline.** Our Media Foundation backend is synchronous today, so this does not
   bite yet; the moment anything async is added, the drain-and-count pattern is the difference
   between a clean stop and a stuck process.
5. **Bounded surfaces with an explicit miss.** We bound fragments and frame queues by bytes; the
   surface pool is the same discipline one layer lower.
6. **Encoder statistics.** Per-stage timings (submit, wait, convert, lock) turn "the stream is late"
   into a number, and we already have a diagnostics switch to print it under.

Deliberately not adopted:

- The **eighteen-name inventory**. Serein sends H.264 today and AV1 opt-in; the matrix exists in the
  client because its settings UI exposes every vendor × transport × codec combination. One backend
  per vendor is enough for us, in the order NVENC → Media Foundation → software.
- **Zero-copy CUDA/D3D11 hand-off.** Our capture path already produces a CPU-visible picture and the
  measured encoder time is dominated by the upload at 4K. It is the right optimization for a client
  that captures into GPU memory; it is a rewrite of the capture seam for us, not an encoder change.
- **Vendor experiment switches.** `device_reuse_check`'s override and `nvRelaxRc` are server-driven
  knobs of a product with a fleet; a local client needs configuration it can explain in a document.

## Open questions

- Which values the Go Live path sets for `preset`, `rc_mode`, `tuning`, `multi_pass` and the VBV size,
  and which of the six preset tiers a quality level picks. Known: the preset table above, and that the
  configuration object is shared with the camera, so the tokens alone do not answer it.
  What blocks the rest: neither the configuration log strings nor the preset table is the target of a
  direct reference. A scan of the whole `.text` section for `lea r64, [rip + disp32]` finds 45,629
  destinations inside `.rdata` but neither of those two addresses, and no absolute pointer to either
  appears anywhere in the image, so the code reaches them through an indirection that only a control
  flow graph can resolve. A whole-image `angr` CFG did not finish on this 22.6 MB Rust binary (88,890
  functions discovered before it ran out of time). The remaining routes are a local CFG rooted at a
  candidate function found another way, or recording the client's own `nvEncInitializeEncoder` call at
  run time.
- Whether the client renegotiates the stream codec when a hardware encoder fails mid-share, or ends
  the share. There is a `Failed to create video encoder because we only support H.264, H.265 and AV1`
  path and a per-codec error callback exposed to JavaScript, but the policy above them is in the web
  bundle.
- What `Encoder device got ~*~ boosted ~*~` changes, and whether `nvRelaxRc` is a rate-control
  relaxation or an unrelated experiment tag.
