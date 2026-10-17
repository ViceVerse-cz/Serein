# Shared FFmpeg media libraries

The Experimental camera/screen-share engine links to a small FFmpeg 7.1.5
LGPL-2.1-or-later build of libavcodec, libavutil, libavformat, libswscale and
libswresample. The native bridge in `crates/platform` also serves incoming
streams and bounded video-attachment playback. Stable uses the original platform
H264 encoders: Media Foundation on Windows, VideoToolbox on macOS and GStreamer
VA-API/NVENC on Linux, with source-built Rust OpenH264 fallback.

Experimental compiles this exact encoder set; hardware capabilities and runtime
availability determine which can open.

Hardware selection follows the running renderer's physical GPU, including the
app's existing GPU preference. Native LUID, PCI bus address or Metal registry ID
binds the encoder and capability query to that device. Another GPU of the same
vendor/model cannot supply its results. An unidentified/unavailable target uses
H264 software fallback; HEVC/AV1 reports unavailable rather than changing codec.

Experimental settings initially query each vendor's driver capability API for
the compiled FFmpeg encoder paths, without initializing encoders or submitting
pictures. Recheck repeats those queries. The separate, optional **Test encoder**
action validates synthetic camera/screen output for the selected codec, without
software or another-vendor fallback; its results never replace the driver report.
Both use disposable Serein helpers, not an FFmpeg CLI, with a ten-second deadline
per backend/codec. The parent kills/reaps cancelled helpers; reopening restarts
cancelled discovery but never automatically repeats an encoder test. Reports are
process-local, with inconclusive APIs/timeouts shown as Unknown. Driver-reported
support is not a guarantee of a working encode or peer negotiation;
see [voice documentation](../../docs/voice.md).

| Platform | H264 | H265/HEVC | AV1 |
|---|---|---|---|
| Linux/Windows x64 | OpenH264, NVENC, AMF, Quick Sync | NVENC, AMF, Quick Sync | NVENC, AMF, Quick Sync |
| Linux ARM64 | OpenH264, NVENC, AMF | NVENC, AMF | NVENC, AMF |
| macOS | OpenH264, VideoToolbox | VideoToolbox | Unavailable |
| Windows ARM64 | OpenH264 | Unavailable | Unavailable |

OpenH264 2.6.0 supplies Experimental's software H264 encoder. There are no
software HEVC/AV1 encoders: these codecs require compatible hardware. Software
fallback applies only to H264 selection/negotiation. An explicit HEVC/AV1 selection
cannot send H264 bytes
under that codec's transport metadata. FFmpeg Media Foundation and all three
`h264_vaapi`/`hevc_vaapi`/`av1_vaapi` encoders,
GPL/nonfree components, command-line programs, networking and avfilter are
disabled. Decoder allowlists include H.264/HEVC, AV1/dav1d and supported
hardware paths, plus AAC, Opus, Vorbis, MP3 and required PCM audio. MOV/MP4,
Matroska/WebM demuxers, scaling and audio resampling serve attachment
playback. `build.json` records exact encoder and decoder names; native registry
checks reject additions outside those allowlists. Decoders receive an anonymous
reader bounded to 100 MiB, never a service URL or credentials.

Incoming streams admit visible pictures through 7680×4320, including portrait
equivalents and 6144×2560. Admission checks coded geometry, reference counts and
8–10-bit gray/4:2:0 profiles before picture allocation. Media working-set
reservations are shared across players and decoders; they are not an OS RSS or
GPU-memory measurement. Only the latest display picture is retained.

PQ/HLG pictures keep P010 pixels and color metadata until presentation. The
renderer uses an advertised HDR surface on an active HDR display, otherwise
tone maps to SDR. HDR capture is capability gated and defaults to SDR streaming
when receiver ten-bit compatibility is unknown. Codec availability alone does
not establish HDR negotiation. H.264 remains the compatible SDR sender path.

On Linux/macOS install Python 3.12+, make, pkg-config, a C/C++ compiler, nasm,
Meson 1.9.2 and Ninja 1.13.0;
Linux also needs CMake, patchelf, libva and libdrm development packages. Then,
from the repository root:

```sh
python3 scripts/build-ffmpeg.py --jobs 8
export FFMPEG_DIR="$PWD/target/ffmpeg/prefix"
if [ "$(uname -s)" = Darwin ]; then
  export DYLD_LIBRARY_PATH="$FFMPEG_DIR/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
else
  export LD_LIBRARY_PATH="$FFMPEG_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
cargo xtask check
cargo xtask package
```

The loader path is for development and unit-test binaries. Release packages use
their own bundled library paths and must also be checked without this environment.

Windows uses MSYS2 bash with make, pkgconf and nasm, Meson/Ninja installed for
Windows Python, native CMake/NMake, and an initialized MSVC
developer environment matching the Rust target. Run the same builder through
Windows Python, then set `FFMPEG_DIR` to its absolute native prefix for Cargo.
The Windows CI jobs show the complete setup. Source archives have exact SHA-256
pins. Builds reuse a matching completed prefix after checking required SDK headers,
link/runtime libraries and bundled source/provenance files are present and nonempty.
Missing files, empty files and broken library aliases reject reuse; a failed/incompatible build
requires fresh `--prefix` and `--work-dir` paths. `--offline` consumes previously
downloaded archives in `--cache-dir` without network access.

On Linux/macOS, `CC` and `CXX` select the compiler for all native dependencies;
when unset, the recipe uses `cc` and `c++`. FFmpeg receives these choices through
its explicit configure options, since it otherwise defaults to `gcc`/`g++` and
does not use those environment variables. Nix selects its stdenv compiler wrappers
by absolute path. Windows retains the explicit MSVC toolchain.
The builder hashes its complete file, so recipe changes require a fresh prefix
and work directory rather than relabelling previously built libraries.

Packages contain replaceable shared libraries and the complete corresponding
FFmpeg source archive, LGPL text, build recipe, source hashes, source patch and configure
arguments in `ffmpeg-source` (Linux: `share/doc/serein/ffmpeg-source`). dav1d 1.5.1
is statically linked with its unmodified BSD-2-Clause notice and full source archive.
OpenH264
and NVENC headers retain their BSD/MIT notices. AMF 1.4.36 public headers retain
AMD's MIT license and standards/patent notice. Intel oneVPL 2.14.0's MIT license,
third-party notice and complete source archive accompany its statically linked
PIC dispatcher, now also linked into the application for hardware capability
enumeration. Linux AMD device binding additionally builds with pinned
Vulkan-Headers 1.3.290; its MIT/Apache license texts and complete header source
archive accompany the recipe. No Intel GPU runtime, AMD driver or Vulkan loader
is redistributed. OpenH264 is compiled from source,
without assuming Cisco binary-download patent coverage. Rebuilding a library
requires the compiler/tool versions recorded by the distributor's build logs.
Serein's MIT/Apache application sources remain available in the repository.
The FFmpeg build uses the `-serein` library suffix and `SEREIN_LIBAVCODEC_61` /
`SEREIN_LIBAVUTIL_59` ELF symbol versions so it can coexist with the host
FFmpeg used by GStreamer's incoming-video plugins. The other three FFmpeg libraries
use their corresponding private symbol versions. The version-script changes,
QSV allocation/dependency fixes, dav1d allocation limits, checked QSV quality negotiation, explicit
VideoToolbox GPU registry selection and AMF external Vulkan-device support are retained as
`serein-ffmpeg.patch` with the original source archive. QSV requests packet allocation through the app's capped buffer
callback, so an excessive driver-advised packet size fails before allocation.
The pinned HEVC-QSV encoder also selects its internal HEVC SEI helpers, which
upstream 7.1.5 omits from that encoder's configure dependencies; the shared decoders are
enabled. Linux shared-library linking uses `-z defs`, rejecting unresolved
symbols before installation.

Linux also isolates bundled OpenH264 from host GStreamer plugins. Its runtime
SONAME is `libopenh264-serein.so.8`; its seven public entry points use the
`serein_Wels` prefix and all internal symbols are hidden. The installed headers
retain the original source API through macros. `serein-openh264.patch` accompanies
the original source subset and is applied automatically by the shipped recipe.
Desktop and voice-test executables hide native static archive symbols with
`--exclude-libs,ALL`, including OpenH264 bundled in Rust's hashed archives.
Both steps are required: Ubuntu's OpenH264 2.4 GStreamer plugin otherwise binds
to our 2.6 implementation and overwrites its smaller encoder parameter structure.
A private SONAME also lets newer host plugins load their own ABI-major-8 library.
Windows and macOS retain their existing library names and linking rules.

Packages include FFmpeg, NVENC-header and oneVPL archives for the enabled
platform, the pinned Vulkan-Headers archive on Linux, a deterministic 1,198,914-byte OpenH264 source archive, and a
deterministic 620 KiB AMF public-header archive. The OpenH264 subset omits only
the upstream root `openh264-2.6.0/res/` directory of test media. Every other
source/build/test/docs file, license, executable mode and nested Android
`res/` directory is preserved. Sorted USTAR entries use fixed ownership and
timestamps with bzip2 level 9 compression; the original and subset hashes are
recorded separately. The shared-library build uses this same subset, and tests
needing the omitted media require the original upstream archive.
The AMF subset contains only upstream `LICENSE.txt` and `amf/public/include/` from
the checked 1.4.36 SDK archive; it is not the complete SDK. `build.json` records
both original SDK and subset SHA-256 hashes. The builder regenerates the same
plain USTAR subset from the pinned SDK when needed, with sorted paths and fixed
metadata. To rebuild from a package without downloading the 171 MiB SDK, copy
its `ffmpeg-source` directory to a writable location, then run there:

```sh
python3 build-ffmpeg.py --offline --cache-dir source \
  --prefix rebuilt/prefix --work-dir rebuilt/work --jobs 8
```

Use the original package's OS/architecture and install the build prerequisites.
The recipe applies its recorded FFmpeg changes automatically. The static oneVPL
dispatcher's pkg-config file provides the required Linux C++/thread/dl link
flags and Windows registry/COM/GUID libraries without statically linking libva.

On Linux installed libraries live in `lib/serein`, with executable-relative
lookup; the portable staging tree uses `lib`. macOS uses `Contents/Frameworks`
with `@rpath` install names and signs each library before the app bundle.
Windows places the five FFmpeg DLLs and OpenH264 alongside `serein.exe`. OpenH264's MSVC recipe
uses `-MT`; oneVPL's Windows build selects its static CRT, and FFmpeg explicitly
uses `-MT` too. Dispatcher-owned objects are released by the matching SDK APIs;
FFmpeg owns its frame/packet buffers. This avoids adding a Visual C++ runtime or
dispatcher DLL requirement for software fallback. Windows CI checks actual DLL
imports for those dependencies; that binary check still requires a Windows
runner. FFmpeg loads NVIDIA/AMD driver libraries only when attempting those
encoders, and the oneVPL dispatcher loads a system-provided compatible Intel
GPU runtime (`libmfx-gen.so.1.2` or
`libmfxhw64.so.1` on Linux). Linux QSV uses libva/libva-drm/libdrm to create the
Intel driver device; VA-API encoders remain disabled in Experimental. Stable's
Linux VA-API/NVENC path uses system GStreamer plugins independently. Windows AMF/QSV use
D3D11 devices bound by the renderer's LUID. Linux AMF derives a Vulkan device
from that physical GPU's explicit DRM render node and gives its native handles
to AMF; encoding uses no AMD VA-API driver interface or Vulkan video encoder. The
bundled patch checks DRM character-device numbers, requires exact Vulkan DRM
device matching, and rejects unavailable identity rather than selecting another
GPU by vendor/model. macOS Stable and Experimental require the renderer GPU's
Metal registry ID in VideoToolbox's encoder specification. QSV's frame-free
startup Query must retain requested lookahead and CBR; corrected B-frame counts
are propagated before input surfaces are initialized. Driver or sandbox access
failures select OpenH264 for H264, or report unavailable for HEVC/AV1.
No broader Flatpak permissions are added. Hardware availability and
live Discord delivery still require validation on the actual platform.
