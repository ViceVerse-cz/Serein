# Native microphone DSP ABI (preview)

Custom suppression can select **Plugin**. This is an owner-installed native
library, separate from sandboxed Wasm extensions. Serein includes only this open
interface and its loader, under the repository's MIT OR Apache-2.0 terms. No
proprietary SDK, model, binary, license key or vendor implementation is included.
The seam does not establish compatibility with any vendor SDK.

**Draft limitation:** Echo bypasses a failed plugin, but the unchanged audio
worker treats every returned DSP error as fatal and stops the call/preview.
Continuous Off fallback therefore requires a plugin-only recoverable error
route through the worker and desktop warning handling. That change is pending
resolution of the requirement to leave the worker unchanged.

## Exports

Export these exact unmangled C symbols, using the host architecture's C calling
convention (`extern "C"` in Rust; `extern "C"` and export visibility in C++):

```c
#include <stdint.h>
#include <stddef.h>

void *serein_dsp_create(uint32_t sample_rate);
int32_t serein_dsp_process(void *ctx, int16_t *pcm, size_t frames);
void serein_dsp_destroy(void *ctx);
```

`create(48000)` returns a uniquely owned session, or NULL on failure. It must
clean up any partial allocation before returning NULL. The host resolves all
three exports before calling create. `process` receives exactly 480 signed
16-bit mono samples (10 ms at 48 kHz) in writable memory. Process in place and
return 0 on success or a negative code on failure; positive codes are reserved
and also rejected. Never retain the PCM pointer. `destroy` frees the session
exactly once, before the library is unloaded. All calls are serial on the same
audio worker; there are no device or render-thread calls and no concurrent
session access. There are no Rust layouts, strings or allocation ownership
transfers across the ABI; the loader's function table is `#[repr(C)]`.

Every function and library initializer/finalizer must return normally: catch
vendor exceptions/panics inside the addon and translate failures to NULL or a
negative status. Do not unwind, abort, open devices, access credentials, record
or transmit audio. Processing must finish within its 10 ms budget and bound its
own allocations/history. The host owns one session and one 960-byte conversion
buffer, with no new queues. Native library memory and execution time cannot be
bounded or sandboxed by this ABI. A broken or malicious native library can crash
or compromise the process; graceful degradation covers loader errors and ABI
error returns, not native faults. Install only trusted code.

## Location and loading

The loader reuses `local_store::data_dir()` without creating directories. With
the packaged build and no override:

| OS | Path |
| --- | --- |
| Windows | `%LOCALAPPDATA%\serein\plugins\serein_dsp.dll` |
| macOS | `~/Library/Application Support/serein/plugins/libserein_dsp.dylib` |
| Linux | `$XDG_DATA_HOME/serein/plugins/libserein_dsp.so`, or `~/.local/share/serein/plugins/libserein_dsp.so` |

Default desktop source builds use `serein-development` in place of `serein`, matching
existing data isolation. The existing absolute `SEREIN_DATA_DIR` override also
applies; relative/empty overrides fail without fallback. The plugin filename is
fixed for each OS. There is no working-directory/PATH search for the plugin,
download, settings field or migration. OS loading of the library's dependencies
still follows the native loader's rules.

Loading is lazy when the worker first switches to Plugin. Existing Off, RNNoise
and WebRTC paths do not load or inspect the library. RNNoise remains the default;
Voice Isolation, Studio and Custom retain their existing semantics. Missing
stored suppression uses RNNoise; unknown enum strings still reject as before.

The selected plugin replaces only suppression, after AEC and before digital
AGC. Samples use RNNoise's 32768 scaling, saturate to i16, then scale back and
clamp to [-1, 1]. The existing `noise` diagnostic timing slot includes plugin
processing; no diagnostic field names change. Switching away destroys the
session. A history reset destroys and recreates an active plugin session; a
failed recreation is reported on the next configure call.

Missing libraries/exports, loader rejection, NULL creation and nonzero process
status produce the fixed `Microphone processing failed: native DSP plugin
unavailable` error. Echo retains the selected settings and bypasses suppression
after failure; AEC, AGC, manual gain and sensitivity retain their independent
settings. Failed process output is discarded. Unchanged settings do not retry
or repeatedly report the failure; switch away and back to retry installation.
