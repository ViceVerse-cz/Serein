#!/usr/bin/env python3
"""Build Serein's pinned shared FFmpeg libraries; never capture or open media.

Linux/macOS: Python 3.12+, make, pkg-config, C/C++ compiler, nasm; Linux also
needs cmake, patchelf, libva and libdrm development files. Windows: run from
MSYS2 bash with make/pkgconf/nasm, native cmake/nmake and the
MSVC developer environment (x64 or arm64). Sources are checksum verified.
"""

import argparse
import difflib
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import urllib.request


SOURCES = {
    "dav1d": {
        "file": "dav1d-1.5.1.tar.xz",
        "url": "https://downloads.videolan.org/pub/videolan/dav1d/1.5.1/dav1d-1.5.1.tar.xz",
        "sha256": "401813f1f89fa8fd4295805aa5284d9aed9bc7fc1fdbe554af4292f64cbabe21",
    },
    "ffmpeg": {
        "file": "ffmpeg-7.1.5.tar.xz",
        "url": "https://distfiles.macports.org/ffmpeg/ffmpeg-7.1.5.tar.xz",
        "sha256": "de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f",
    },
    "openh264": {
        "file": "openh264-2.6.0.tar.gz",
        "url": "https://codeload.github.com/cisco/openh264/tar.gz/refs/tags/v2.6.0",
        "sha256": "558544ad358283a7ab2930d69a9ceddf913f4a51ee9bf1bfb9e377322af81a69",
    },
    "openh264-source": {
        "file": "openh264-2.6.0-source.tar.bz2",
        "sha256": "783c8cdede353f0b0c77784a505438a32454917e9a95338b3b8d30a01f8ce0e5",
        "generated_from": "openh264",
        "omitted_paths": ["openh264-2.6.0/res/"],
    },
    "nv-codec-headers": {
        "file": "nv-codec-headers-12.2.72.0.tar.gz",
        "url": "https://codeload.github.com/FFmpeg/nv-codec-headers/tar.gz/refs/tags/n12.2.72.0",
        "sha256": "dbeaec433d93b850714760282f1d0992b1254fc3b5a6cb7d76fc1340a1e47563",
    },
    "amf-sdk": {
        "file": "AMF-1.4.36.tar.gz",
        "url": "https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AMF/tar.gz/refs/tags/v1.4.36",
        "sha256": "240a42033babc7920e5476506d5ac0c5628f67908833168e746406808d0ef146",
        "max_bytes": 192 * 1024 * 1024,
    },
    "amf-headers": {
        "file": "AMF-1.4.36-headers.tar",
        "sha256": "eb1a8cf31da12bcc4613f188809e7cc74d2582387f1c3346c0f541fb0e1dd21e",
        "generated_from": "amf-sdk",
        "paths": ["LICENSE.txt", "amf/public/include/"],
    },
    "onevpl": {
        "file": "libvpl-2.14.0.tar.gz",
        "url": "https://codeload.github.com/intel/libvpl/tar.gz/refs/tags/v2.14.0",
        "sha256": "7c6bff1c1708d910032c2e6c44998ffff3f5fdbf06b00972bc48bf2dd9e5ac06",
    },
    "vulkan-headers": {
        "file": "Vulkan-Headers-1.3.290.tar.gz",
        "url": "https://codeload.github.com/KhronosGroup/Vulkan-Headers/tar.gz/refs/tags/v1.3.290",
        "sha256": "f38a653bf93cab7a2a229a53d2d53b1cba9a2819e4c0a7de13c54085bde9bcf5",
    },
}

OPENH264_APIS = (
    "WelsCreateSVCEncoder", "WelsDestroySVCEncoder", "WelsGetDecoderCapability",
    "WelsCreateDecoder", "WelsDestroyDecoder", "WelsGetCodecVersion", "WelsGetCodecVersionEx",
)

QSV_FEATURE_VALIDATION = """/* Serein's negotiated quality contract; no frame submission or allocation. */
#ifndef SEREIN_QSV_FEATURE_VALIDATION_H
#define SEREIN_QSV_FEATURE_VALIDATION_H
static int serein_qsv_quality_matches(int requested_b, unsigned int reference_distance,
    int requested_depth, int requested_extbrc, unsigned int driver_extbrc,
    unsigned int driver_depth, unsigned int driver_rate_control, unsigned int requested_rate_control)
{
    if ((requested_rate_control == MFX_RATECONTROL_CBR || requested_rate_control == MFX_RATECONTROL_VBR) &&
        driver_rate_control != requested_rate_control)
        return 0;
    if (requested_b == 0 && reference_distance != 1)
        return 0;
    if (requested_b > 0 && (!reference_distance || reference_distance > (unsigned int)requested_b + 1))
        return 0;
    if (requested_depth > 0 && (requested_extbrc <= 0 || driver_extbrc != MFX_CODINGOPTION_ON ||
        driver_depth != (unsigned int)requested_depth ||
        (driver_rate_control != MFX_RATECONTROL_CBR && driver_rate_control != MFX_RATECONTROL_VBR)))
        return 0;
    return 1;
}
static int serein_qsv_tiles_match(int requested_columns, unsigned int driver_columns)
{
    /* A driver may decline parallelism and return one column, but may never
     * expand our request beyond the two-engine ceiling. Zero is inconclusive. */
    return requested_columns <= 0 ||
        (driver_columns > 0 && driver_columns <= (unsigned int)requested_columns);
}
#endif
"""

AMF_SPLIT_ENCODING = """/* Optional split-frame hint, bounded to this GPU's two codec engines. */
#ifndef SEREIN_AMF_SPLIT_ENCODING_H
#define SEREIN_AMF_SPLIT_ENCODING_H
#include <AMF/components/ComponentCaps.h>
static int serein_amf_split_eligible(AMFComponent *encoder,
    const wchar_t *count_name, const wchar_t *split_name)
{
    AMFCaps *caps = NULL;
    AMFVariantStruct count = {0}, flag = {0};
    int eligible = 0;
    /* An old runtime may advertise two engines but lack this optional flag. */
    if (encoder->pVtbl->GetProperty(encoder, split_name, &flag) != AMF_OK ||
        flag.type != AMF_VARIANT_BOOL)
        goto cleanup;
    if (encoder->pVtbl->GetCaps(encoder, &caps) == AMF_OK && caps &&
        caps->pVtbl->GetProperty(caps, count_name, &count) == AMF_OK &&
        count.type == AMF_VARIANT_INT64)
        eligible = count.int64Value == 2;
cleanup:
    /* GetProperty owns its returned variant, including unexpected types and
     * values populated before an error. Release interfaces/strings as well. */
    AMFVariantClear(&count);
    AMFVariantClear(&flag);
    if (caps)
        caps->pVtbl->Release(caps);
    /* AMF has no numeric engine limit. Do not enable its boolean multi-engine
     * hint on devices advertising >2, or borrow engines from another codec. */
    return eligible;
}
static int serein_amf_request_split(AMFComponent *encoder,
    const wchar_t *count_name, const wchar_t *split_name)
{
    AMFVariantStruct request = {0};
    if (!serein_amf_split_eligible(encoder, count_name, split_name))
        return 0;
    AMFVariantAssignBool(&request, true);
    return encoder->pVtbl->SetProperty(encoder, split_name, request) == AMF_OK;
}
#endif
"""

AMF_CONTROL = """/* Private Serein AMF control and ownership; retained with the LGPL source patch. */
#ifndef SEREIN_AMF_CONTROL_H
#define SEREIN_AMF_CONTROL_H
#include <stdint.h>
#include <string.h>
#include "libavutil/time.h"

/* av_opt_set_bin copies this record. The caller retains opaque until close
 * returns; the callback only reads cancellation and must never unwind. */
typedef struct SereinAmfInterruptV1 {
    uint32_t version, size;
    int (*cancelled)(void *opaque);
    void *opaque;
} SereinAmfInterruptV1;

static int serein_amf_interrupt(AmfContext *ctx, int waiting)
{
    SereinAmfInterruptV1 control;
    int64_t now;
    if (ctx->serein_interrupt_size) {
        if (!ctx->serein_interrupt || (size_t)ctx->serein_interrupt_size != sizeof(control))
            return AVERROR(EINVAL);
        memcpy(&control, ctx->serein_interrupt, sizeof(control));
        if (control.version != 1 || control.size != sizeof(control) ||
            !control.cancelled || !control.opaque)
            return AVERROR(EINVAL);
        if (control.cancelled(control.opaque))
            return AVERROR_EXIT;
    }
    if (!waiting)
        return 0;
    if (!ctx->delayed_surface && !ctx->delayed_drain && !ctx->eof &&
        (!ctx->timestamp_list || !av_fifo_can_read(ctx->timestamp_list))) {
        ctx->serein_progress_deadline = 0;
        return 0;
    }
    now = av_gettime_relative();
    if (!ctx->serein_progress_deadline)
        ctx->serein_progress_deadline = now + 15000000;
    return now >= ctx->serein_progress_deadline ? AVERROR(ETIMEDOUT) : 0;
}

/* An AMFBuffer containing a raw AVFrame pointer has no AVFrame destructor.
 * Keep every clone owned here until output consumes it or Terminate returns.
 * Forty-eight entries match the caller's existing in-flight picture limit. */
static int serein_amf_retain(AmfContext *ctx, AVFrame *frame)
{
    for (unsigned i = 0; i < 48; ++i) {
        if (!ctx->serein_retained[i]) {
            ctx->serein_retained[i] = frame;
            return 0;
        }
    }
    return AVERROR(ENOSPC);
}

static int serein_amf_release(AmfContext *ctx, AVFrame *frame)
{
    if (!frame)
        return AVERROR(EINVAL);
    for (unsigned i = 0; i < 48; ++i) {
        if (ctx->serein_retained[i] == frame) {
            av_frame_free(&ctx->serein_retained[i]);
            return 0;
        }
    }
    return AVERROR(EINVAL);
}

static void serein_amf_release_all(AmfContext *ctx)
{
    for (unsigned i = 0; i < 48; ++i)
        av_frame_free(&ctx->serein_retained[i]);
    ctx->hwsurfaces_in_queue = 0;
    ctx->serein_progress_deadline = 0;
}
#endif
"""


def amf_control_patches():
    """Patch the actual AMF loop, preserving input acceptance and encoder quality."""
    replacements = [
        ("libavcodec/amfenc.h", "    int                 usage;\n",
         "    uint8_t            *serein_interrupt;\n"
         "    int                 serein_interrupt_size;\n"
         "    int64_t             serein_progress_deadline;\n"
         "    AVFrame            *serein_retained[48];\n"
         "    int                 usage;\n"),
        ("libavcodec/amfenc.c", '#include "amfenc.h"\n',
         '#include "amfenc.h"\n#include "serein_amf_control.h"\n'),
        ("libavcodec/amfenc.c", "    if (ctx->context) {\n        ctx->context->pVtbl->Terminate(ctx->context);",
         "    /* The driver no longer owns queued surfaces. Release clones even\n"
         "     * when cancellation discarded their output or a submit failed. */\n"
         "    serein_amf_release_all(ctx);\n\n"
         "    if (ctx->context) {\n        ctx->context->pVtbl->Terminate(ctx->context);"),
        ("libavcodec/amfenc.c", "static AMFBuffer *amf_create_buffer_with_frame_ref(const AVFrame *frame, AMFContext *context)",
         "static AMFBuffer *amf_create_buffer_with_frame_ref(const AVFrame *frame, AmfContext *ctx)"),
        ("libavcodec/amfenc.c", "    res = context->pVtbl->AllocBuffer(context, AMF_MEMORY_HOST, sizeof(frame_ref), &frame_ref_storage_buffer);",
         "    AMFContext *context = ctx->context;\n"
         "    res = context->pVtbl->AllocBuffer(context, AMF_MEMORY_HOST, sizeof(frame_ref), &frame_ref_storage_buffer);"),
        ("libavcodec/amfenc.c", "        if (frame_ref) {\n            memcpy(frame_ref_storage_buffer->pVtbl->GetNative(frame_ref_storage_buffer), &frame_ref, sizeof(frame_ref));",
         "        if (frame_ref && serein_amf_retain(ctx, frame_ref) == 0) {\n"
         "            memcpy(frame_ref_storage_buffer->pVtbl->GetNative(frame_ref_storage_buffer), &frame_ref, sizeof(frame_ref));"),
        ("libavcodec/amfenc.c", "        } else {\n            frame_ref_storage_buffer->pVtbl->Release(frame_ref_storage_buffer);\n            frame_ref_storage_buffer = NULL;",
         "        } else {\n            av_frame_free(&frame_ref);\n"
         "            frame_ref_storage_buffer->pVtbl->Release(frame_ref_storage_buffer);\n            frame_ref_storage_buffer = NULL;"),
        ("libavcodec/amfenc.c", "static void amf_release_buffer_with_frame_ref(AMFBuffer *frame_ref_storage_buffer)\n{\n    AVFrame *frame_ref;\n    memcpy(&frame_ref, frame_ref_storage_buffer->pVtbl->GetNative(frame_ref_storage_buffer), sizeof(frame_ref));\n    av_frame_free(&frame_ref);\n    frame_ref_storage_buffer->pVtbl->Release(frame_ref_storage_buffer);\n}",
         "static int amf_release_buffer_with_frame_ref(AmfContext *ctx, AMFBuffer *frame_ref_storage_buffer)\n"
         "{\n    AVFrame *frame_ref = NULL;\n    int ret = AVERROR(EINVAL);\n"
         "    if (frame_ref_storage_buffer->pVtbl->GetSize(frame_ref_storage_buffer) == sizeof(frame_ref) &&\n"
         "        frame_ref_storage_buffer->pVtbl->GetNative(frame_ref_storage_buffer)) {\n"
         "        memcpy(&frame_ref, frame_ref_storage_buffer->pVtbl->GetNative(frame_ref_storage_buffer), sizeof(frame_ref));\n"
         "        ret = serein_amf_release(ctx, frame_ref);\n    }\n"
         "    frame_ref_storage_buffer->pVtbl->Release(frame_ref_storage_buffer);\n    return ret;\n}"),
        ("libavcodec/amfenc.c", "    if (!ctx->encoder)\n        return AVERROR(EINVAL);\n",
         "    if (!ctx->encoder)\n        return AVERROR(EINVAL);\n"
         "    if ((ret = serein_amf_interrupt(ctx, 0)) < 0)\n        return ret;\n"
         "    /* Bound this native call, not time spent idle outside FFmpeg.\n"
         "     * The caller separately bounds an outstanding fresh picture. */\n"
         "    ctx->serein_progress_deadline = 0;\n"),
        ("libavcodec/amfenc.c", "int ff_amf_receive_packet(AVCodecContext *avctx, AVPacket *avpkt)\n",
         "static int serein_amf_attach_frame(AmfContext *ctx, AMFSurface *surface, const AVFrame *frame)\n"
         "{\n"
         "    AMFBuffer *reference = amf_create_buffer_with_frame_ref(frame, ctx);\n"
         "    if (!reference)\n        return AVERROR(ENOMEM);\n"
         '    if (amf_set_property_buffer(surface, L"av_frame_ref", reference) != AMF_OK) {\n'
         "        amf_release_buffer_with_frame_ref(ctx, reference);\n"
         "        return AVERROR_UNKNOWN;\n    }\n"
         "    ctx->hwsurfaces_in_queue++;\n"
         "    reference->pVtbl->Release(reference);\n"
         "    return 0;\n}\n\n"
         "int ff_amf_receive_packet(AVCodecContext *avctx, AVPacket *avpkt)\n"),
        ("libavcodec/amfenc.c", '''            AMFBuffer *frame_ref_storage_buffer;

            // input HW surfaces can be vertically aligned by 16; tell AMF the real size
            surface->pVtbl->SetCrop(surface, 0, 0, frame->width, frame->height);

            frame_ref_storage_buffer = amf_create_buffer_with_frame_ref(frame, ctx->context);
            AMF_RETURN_IF_FALSE(ctx, frame_ref_storage_buffer != NULL, AVERROR(ENOMEM), "create_buffer_with_frame_ref() returned NULL\\n");

            res = amf_set_property_buffer(surface, L"av_frame_ref", frame_ref_storage_buffer);
            AMF_RETURN_IF_FALSE(ctx, res == AMF_OK, AVERROR_UNKNOWN, "SetProperty failed for \\"av_frame_ref\\" with error %d\\n", res);
            ctx->hwsurfaces_in_queue++;
            frame_ref_storage_buffer->pVtbl->Release(frame_ref_storage_buffer);''',
         '''            // input HW surfaces can be vertically aligned by 16; tell AMF the real size
            surface->pVtbl->SetCrop(surface, 0, 0, frame->width, frame->height);
            ret = serein_amf_attach_frame(ctx, surface, frame);
            if (ret < 0) {
                surface->pVtbl->Release(surface);
                return ret;
            }'''),
        ("libavcodec/amfenc.c", "        block_and_wait = 0;\n        // poll data\n",
         "        block_and_wait = 0;\n"
         "        if ((ret = serein_amf_interrupt(ctx, 1)) < 0)\n            return ret;\n"
         "        // poll data\n"),
        ("libavcodec/amfenc.c", "                    amf_release_buffer_with_frame_ref(frame_ref_storage_buffer);\n                    ctx->hwsurfaces_in_queue--;",
         "                    int released = amf_release_buffer_with_frame_ref(ctx, frame_ref_storage_buffer);\n"
         "                    if (released < 0) { data->pVtbl->Release(data); return released; }\n"
         "                    ctx->hwsurfaces_in_queue--;"),
        ("libavcodec/amfenc.c", '                AMF_RETURN_IF_FALSE(ctx, ret >= 0, ret, "amf_copy_buffer() failed with error %d\\n", ret);',
         '                AMF_RETURN_IF_FALSE(ctx, ret >= 0, ret, "amf_copy_buffer() failed with error %d\\n", ret);\n'
         '                /* Only a real encoded packet resets the no-progress budget. */\n'
         '                ctx->serein_progress_deadline = 0;'),
        ("libavcodec/amfenc.c", "                block_and_wait = 1;\n                av_usleep(1000);",
         "                block_and_wait = 1;\n"
         "                if ((ret = serein_amf_interrupt(ctx, 1)) < 0)\n                    return ret;\n"
         "                av_usleep(1000);"),
    ]
    for codec in ("h264", "hevc", "av1"):
        replacements.append((f"libavcodec/amfenc_{codec}.c", "static const AVOption options[] = {\n",
            "static const AVOption options[] = {\n"
            '    { "serein_interrupt", "Private versioned cancellation callback", OFFSET(serein_interrupt), AV_OPT_TYPE_BINARY, {.str = NULL}, 0, 0, VE },\n'))
    return replacements


def run(*args, cwd=None, env=None):
    subprocess.run([str(arg) for arg in args], cwd=cwd, env=env, check=True)


def posix(path):
    """MSVC is driven by MSYS make/configure; use its paths in build recipes."""
    if os.name == "nt":
        return subprocess.check_output(["cygpath", "-u", str(path)], text=True).strip()
    return str(path)


def configure_shell(system, env):
    if system != "Windows":
        return "bash"
    # Native Windows Python's executable search may find System32's WSL bash
    # before MSYS2, even when this script was launched from an MSYS2 shell.
    # cygpath resolves /usr within that MSYS installation; use its absolute
    # native path as one subprocess argument, including when it contains spaces.
    try:
        shell = subprocess.check_output(["cygpath", "-w", "/usr/bin/bash.exe"],
                                        text=True, env=env).strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError("Windows FFmpeg configure requires MSYS2 cygpath and Bash; "
                         "run the builder from an MSYS2 shell") from error
    if not shell or not Path(shell).is_absolute() or not Path(shell).is_file():
        raise ValueError("MSYS2 cygpath did not resolve an existing absolute Bash executable")
    return shell


def meson_environment(system, env):
    result = dict(env)
    if system == "Windows":
        compiler = shutil.which("cl.exe", path=env.get("PATH"))
        if not compiler or not Path(compiler).with_name("link.exe").is_file():
            raise ValueError("dav1d requires the MSVC compiler and matching linker; "
                             "run from the target architecture's Developer shell")
        # MSYS prepends its GNU link.exe. Meson invokes the linker directly,
        # unlike cl's own driver, so put the selected MSVC tools first for all
        # three Meson phases without changing make/configure's MSYS lookup.
        result["PATH"] = str(Path(compiler).parent) + os.pathsep + env.get("PATH", "")
        result["CC"] = result["CXX"] = "cl"
    return result


def fetch(source, cache, offline):
    destination = cache / source["file"]
    if not destination.exists():
        if offline:
            raise ValueError(f"Offline source missing: {destination}")
        temporary = destination.with_suffix(destination.suffix + ".download")
        request = urllib.request.Request(source["url"], headers={"User-Agent": "Serein-source-build"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response, temporary.open("wb") as output:
                total = 0
                while block := response.read(1024 * 1024):
                    total += len(block)
                    if total > source.get("max_bytes", 128 * 1024 * 1024):
                        raise ValueError(f"Dependency source exceeds its download bound: {source['file']}")
                    output.write(block)
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    with destination.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    if checksum != source["sha256"]:
        raise ValueError(f"Source SHA-256 mismatch: {destination}")
    return destination


def amf_headers(cache, offline):
    """Keep only the exact MIT public headers, and allow package-local rebuilds."""
    definition = SOURCES["amf-headers"]
    destination = cache / definition["file"]
    if not destination.exists():
        sdk = fetch(SOURCES["amf-sdk"], cache, offline)
        temporary = destination.with_suffix(".tar.generated")
        try:
            with tarfile.open(sdk) as source, tarfile.open(temporary, "w", format=tarfile.USTAR_FORMAT) as target:
                entries = []
                for member in source.getmembers():
                    if member.name == "AMF-1.4.36/LICENSE.txt":
                        name = "AMF-1.4.36-headers/LICENSE.txt"
                    elif member.isfile() and member.name.startswith("AMF-1.4.36/amf/public/include/"):
                        name = "AMF-1.4.36-headers/AMF/" + member.name.split("/amf/public/include/", 1)[1]
                    else:
                        continue
                    if not member.isfile() or member.size > 1024 * 1024:
                        raise ValueError("Unexpected AMF header archive member")
                    data = source.extractfile(member).read()
                    entries.append((name, data))
                # Plain USTAR, fixed metadata and sorted paths avoid gzip and
                # host-specific metadata changing the shipped subset's checksum.
                for name, data in sorted(entries):
                    entry = tarfile.TarInfo(name)
                    entry.mode, entry.size = 0o644, len(data)
                    target.addfile(entry, io.BytesIO(data))
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    return fetch(definition, cache, offline=True)


def write_openh264_subset(archive, destination):
    """Preserve every source/build file and its mode; omit only root test media."""
    with tarfile.open(archive) as source, tarfile.open(destination, "w:bz2", compresslevel=9,
                                                    format=tarfile.USTAR_FORMAT) as target:
        for member in sorted(source.getmembers(), key=lambda item: item.name):
            if member.name == "openh264-2.6.0/res" or member.name.startswith("openh264-2.6.0/res/"):
                continue
            if not (member.isfile() or member.isdir()):
                raise ValueError(f"Unexpected OpenH264 source member: {member.name}")
            entry = tarfile.TarInfo(member.name)
            entry.type, entry.mode, entry.size = member.type, member.mode & 0o777, member.size
            target.addfile(entry, source.extractfile(member) if member.isfile() else None)


def openh264_source(cache, offline):
    definition = SOURCES["openh264-source"]
    destination = cache / definition["file"]
    if not destination.exists():
        archive = fetch(SOURCES["openh264"], cache, offline)
        temporary = destination.with_suffix(".bz2.generated")
        try:
            write_openh264_subset(archive, temporary)
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    return fetch(definition, cache, offline=True)


def unpack(archive, destination):
    destination.mkdir(parents=True)
    with tarfile.open(archive) as source:
        members = source.getmembers()
        roots = {Path(item.name).parts[0] for item in members if Path(item.name).parts}
        if len(roots) != 1:
            raise ValueError(f"Expected one source root: {archive}")
        if sum(item.size for item in members) > 1024 * 1024 * 1024:
            raise ValueError("Expanded source exceeds 1 GiB")
        source.extractall(destination, filter="data")
    return destination / roots.pop()


def toolchain_options(system, arm64, env=None):
    if system != "Windows":
        # FFmpeg's configure ignores the conventional uppercase CC/CXX
        # variables and otherwise defaults to gcc/g++. In particular, Nix's
        # Darwin stdenv provides compiler wrappers as cc/c++, not gcc/g++.
        env = os.environ if env is None else env
        return [f"--cc={env.get('CC') or 'cc'}", f"--cxx={env.get('CXX') or 'c++'}"]
    options = ["--toolchain=msvc", "--target-os=win32", f"--arch={'aarch64' if arm64 else 'x86_64'}"]
    if arm64:
        # FFmpeg 7's MSVC ARM assembly needs gas-preprocessor.pl, which our
        # build tools do not include. Match OpenH264's C-only Windows ARM build.
        options.append("--disable-asm")
    return options


def encoder_backends(system, arm64):
    return {
        "nvenc": system in {"Linux", "Windows"} and not (system == "Windows" and arm64),
        "amf": system == "Linux" or (system == "Windows" and not arm64),
        "qsv": system in {"Linux", "Windows"} and not arm64,
        "videotoolbox": system == "Darwin",
    }


def encoder_names(backends):
    names = {"libopenh264"}
    for backend, enabled in backends.items():
        if enabled:
            codecs = ("h264", "hevc") if backend == "videotoolbox" else ("h264", "hevc", "av1")
            names.update(f"{codec}_{backend}" for codec in codecs)
    return names


def decoder_names(backends):
    names = {"h264", "hevc", "av1", "libdav1d", "aac", "opus", "vorbis", "mp3", "pcm_s16le", "pcm_s24le", "pcm_f32le"}
    if backends["qsv"]:
        names.update(["h264_qsv", "hevc_qsv", "av1_qsv"])
    return names


def hardware_options(system, backends):
    options = ["--disable-mediafoundation", "--disable-encoder=h264_vaapi,hevc_vaapi,av1_vaapi", "--disable-dxva2"]
    options += ["--enable-vulkan" if system == "Linux" and backends["amf"] else "--disable-vulkan"]
    options += ["--enable-vaapi" if system == "Linux" and backends["qsv"] else "--disable-vaapi"]
    options += ["--enable-libdrm" if system == "Linux" and (backends["qsv"] or backends["amf"]) else "--disable-libdrm"]
    options += ["--enable-d3d11va" if system == "Windows" and (backends["qsv"] or backends["amf"]) else "--disable-d3d11va"]
    options += (["--enable-ffnvcodec", "--enable-nvenc"] if backends["nvenc"] else
                ["--disable-ffnvcodec", "--disable-nvenc"])
    options += (["--enable-amf"] if backends["amf"] else ["--disable-amf"])
    options += (["--enable-libvpl"] if backends["qsv"] else ["--disable-libvpl"])
    options += (["--enable-videotoolbox", "--install-name-dir=@rpath"] if backends["videotoolbox"] else
                ["--disable-videotoolbox"])
    options += ["--enable-encoder=" + ",".join(sorted(encoder_names(backends) - {"libopenh264"}))] if any(backends.values()) else []
    if system == "Windows":
        options += ["--enable-d3d11va", "--enable-hwaccel=h264_d3d11va,hevc_d3d11va,av1_d3d11va"]
    if system == "Darwin":
        options += ["--enable-hwaccel=h264_videotoolbox,hevc_videotoolbox"]
    if backends["nvenc"]:
        options += ["--enable-nvdec", "--enable-hwaccel=h264_nvdec,hevc_nvdec,av1_nvdec"]
    if system == "Linux" and backends["qsv"]:
        # VA's driver interface supplies selected-render-node decoding, including
        # AMD GPUs. The h264/hevc/av1_vaapi encoders remain explicitly disabled.
        options += ["--enable-hwaccel=h264_vaapi,hevc_vaapi,av1_vaapi"]
    return options


def amf_vulkan_patches():
    source = "libavutil/hwcontext_vulkan.c"
    return [
        (source, "    FFVkQueueFamilyCtx transfer_qf;\n",
         "    FFVkQueueFamilyCtx transfer_qf;\n"
         "    VkPhysicalDeviceCoherentMemoryFeaturesAMD serein_amf_coherent;\n"),
        (source, "static int vulkan_device_create_internal(AVHWDeviceContext *ctx,\n",
         """/* Match AMF's external-device feature contract, using supported values only.
 * Keep the extension feature in the device owner, not a temporary pNext node. */
static void serein_vk_amf_features(AVHWDeviceContext *ctx,
                                   const VkPhysicalDeviceFeatures2 *supported,
                                   const VkPhysicalDeviceVulkan12Features *supported12)
{
    VulkanDevicePriv *p = ctx->hwctx;
    AVVulkanDeviceContext *hwctx = &p->p;
    FFVulkanFunctions *vk = &p->vkctx.vkfn;
    hwctx->device_features.features = supported->features;
#define AMF_COPY_FEATURE(name) p->device_features_1_2.name = supported12->name
    AMF_COPY_FEATURE(descriptorIndexing);
    AMF_COPY_FEATURE(descriptorBindingUniformBufferUpdateAfterBind);
    AMF_COPY_FEATURE(descriptorBindingSampledImageUpdateAfterBind);
    AMF_COPY_FEATURE(descriptorBindingStorageImageUpdateAfterBind);
    AMF_COPY_FEATURE(descriptorBindingStorageBufferUpdateAfterBind);
    AMF_COPY_FEATURE(descriptorBindingUniformTexelBufferUpdateAfterBind);
    AMF_COPY_FEATURE(descriptorBindingStorageTexelBufferUpdateAfterBind);
#undef AMF_COPY_FEATURE
    for (int i = 0; i < hwctx->nb_enabled_dev_extensions; i++) {
        if (!strcmp(hwctx->enabled_dev_extensions[i], VK_AMD_DEVICE_COHERENT_MEMORY_EXTENSION_NAME)) {
            VkPhysicalDeviceFeatures2 features = {
                .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                .pNext = &p->serein_amf_coherent,
            };
            p->serein_amf_coherent.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COHERENT_MEMORY_FEATURES_AMD;
            vk->GetPhysicalDeviceFeatures2(hwctx->phys_dev, &features);
            if (p->serein_amf_coherent.deviceCoherentMemory)
                ff_vk_link_struct(hwctx->device_features.pNext, &p->serein_amf_coherent);
            break;
        }
    }
}

static int vulkan_device_create_internal(AVHWDeviceContext *ctx,
"""),
        (source, "    /* Add the enabled features into the pnext chain of device creation */\n",
         """    if (av_dict_get(opts, "serein_amf", NULL, 0)) {
        /* Make the actual enabled list available to the AMF feature query. */
        hwctx->enabled_dev_extensions = dev_info.ppEnabledExtensionNames;
        hwctx->nb_enabled_dev_extensions = dev_info.enabledExtensionCount;
        serein_vk_amf_features(ctx, &dev_features, &dev_features_1_2);
        hwctx->enabled_dev_extensions = NULL;
        hwctx->nb_enabled_dev_extensions = 0;
    }

    /* Add the enabled features into the pnext chain of device creation */
"""),
        (source, """                ADD_VAL_TO_LIST(extension_names, extensions_found, token);
""", """                int duplicate = 0;
                for (int i = 0; i < extensions_found; i++)
                    duplicate |= !strcmp(extension_names[i], token);
                if (!duplicate)
                    ADD_VAL_TO_LIST(extension_names, extensions_found, token);
"""),
    ]


def patch_ffmpeg(tree):
    patches = []
    originals = {}
    replacements = [("libavcodec/qsvenc.h", "    int look_ahead_depth;\n", "    int look_ahead_depth;\n    int serein_requested_rate_control;\n"),
                    ("libavcodec/qsvenc.c", "    q->param.mfx.RateControlMethod = rc_mode;\n", "    q->param.mfx.RateControlMethod = rc_mode;\n    q->serein_requested_rate_control = rc_mode;\n"),
                    ("libavcodec/libavcodec.v", "LIBAVCODEC_MAJOR", "SEREIN_LIBAVCODEC_MAJOR"),
                    ("libavutil/libavutil.v", "LIBAVUTIL_MAJOR", "SEREIN_LIBAVUTIL_MAJOR"),
                    ("libavformat/libavformat.v", "LIBAVFORMAT_MAJOR", "SEREIN_LIBAVFORMAT_MAJOR"),
                    ("libavfilter/libavfilter.v", "LIBAVFILTER_MAJOR", "SEREIN_LIBAVFILTER_MAJOR"),
                    ("libswscale/libswscale.v", "LIBSWSCALE_MAJOR", "SEREIN_LIBSWSCALE_MAJOR"),
                    ("libswresample/libswresample.v", "LIBSWRESAMPLE_MAJOR", "SEREIN_LIBSWRESAMPLE_MAJOR"),
                    ("libavcodec/qsvenc.c", "ret = av_new_packet(&pkt.pkt, q->packet_size);",
                     "ret = ff_get_encode_buffer(avctx, &pkt.pkt, q->packet_size, 0);"),
                    ("configure", 'hevc_qsv_encoder_select="hevcparse qsvenc"',
                     'hevc_qsv_encoder_select="hevcparse hevc_sei qsvenc"')]
    # dav1d uses its own pool instead of AVCodecContext.get_buffer2. Reject
    # oversized or unsupported planes before that allocator obtains a surface.
    replacements += [("libavcodec/libdav1d.c", "    ret = av_image_get_buffer_size(format, w, h, DAV1D_PICTURE_ALIGNMENT);",
        """    if (p->p.bpc > 10 || (p->p.layout != DAV1D_PIXEL_LAYOUT_I420 && p->p.layout != DAV1D_PIXEL_LAYOUT_I400) ||
        p->p.w <= 0 || p->p.h <= 0 || p->p.w > 7680 || p->p.h > 7680 || (int64_t)w * h > (int64_t)7680 * 4352)
        return AVERROR(ENOSYS);
    ret = av_image_get_buffer_size(format, w, h, DAV1D_PICTURE_ALIGNMENT);""")]
    # Bind VT decoding through its public RequiredDecoderGPURegistryID key,
    # with a private device-context field preserved in the corresponding-source patch.
    replacements += [
        ("libavutil/hwcontext_videotoolbox.h", "typedef struct AVVTFramesContext {",
         "typedef struct AVVTDeviceContext { uint64_t gpu_registry_id; } AVVTDeviceContext;\n\ntypedef struct AVVTFramesContext {"),
        ("libavutil/hwcontext_videotoolbox.c", "    .type                 = AV_HWDEVICE_TYPE_VIDEOTOOLBOX,",
         "    .type                 = AV_HWDEVICE_TYPE_VIDEOTOOLBOX,\n    .device_hwctx_size     = sizeof(AVVTDeviceContext),"),
        ("libavcodec/videotoolbox.c", '#include "videotoolbox.h"',
         '#include "videotoolbox.h"\n#include <dlfcn.h>'),
        ("libavcodec/videotoolbox.c", "    avc_info = CFDictionaryCreateMutable(kCFAllocatorDefault,",
         """    if (avctx->hw_device_ctx) {
        AVHWDeviceContext *device = (AVHWDeviceContext *)avctx->hw_device_ctx->data;
        if (device->type == AV_HWDEVICE_TYPE_VIDEOTOOLBOX && device->hwctx) {
            AVVTDeviceContext *selected = device->hwctx;
            if (selected->gpu_registry_id) {
                CFStringRef *key = dlsym(RTLD_DEFAULT, "kVTVideoDecoderSpecification_RequiredDecoderGPURegistryID");
                if (!key || !*key) { CFRelease(config_info); return NULL; }
                CFNumberRef number = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt64Type, &selected->gpu_registry_id);
                if (!number) { CFRelease(config_info); return NULL; }
                CFDictionarySetValue(config_info, *key, number);
                CFDictionarySetValue(config_info, kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder, kCFBooleanTrue);
                CFRelease(number);
            }
        }
    }
    avc_info = CFDictionaryCreateMutable(kCFAllocatorDefault,"""),
    ]
    # FFmpeg7.1's VT encoder otherwise silently chooses an unrelated GPU.
    # This public VideoToolbox specification is resolved dynamically so an
    # older SDK can compile, while an OS lacking the key fails closed. Keep
    # these changes in the bundled LGPL source patch beside the pristine tar.
    replacements += [
        ("libavcodec/qsvenc.c", '#include "qsvenc.h"\n',
         '#include "qsvenc.h"\n#include "serein_qsv_feature_validation.h"\n'),
        ("libavcodec/videotoolboxenc.c", "    int allow_sw;\n",
         "    int64_t gpu_registry_id;\n    int allow_sw;\n"),
        ("libavcodec/videotoolboxenc.c", "#define COMMON_OPTIONS \\\n",
         "#define COMMON_OPTIONS \\\n"
         '    { "gpu_registry_id", "Require encoding on this Metal GPU registry ID", OFFSET(gpu_registry_id), AV_OPT_TYPE_INT64, \\\n'
         "        { .i64 = 0 }, INT64_MIN, INT64_MAX, VE }, \\\n"),
        ("libavcodec/videotoolboxenc.c", "    // low-latency mode: eliminate frame reordering, follow a one-in-one-out encoding mode\n",
         "    if (vtctx->gpu_registry_id) {\n"
         '        CFStringRef *key = (CFStringRef *)dlsym(RTLD_DEFAULT, "kVTVideoEncoderSpecification_RequiredEncoderGPURegistryID");\n'
         "        CFNumberRef number;\n"
         "        if (!key || !*key) {\n"
         "            CFRelease(enc_info);\n"
         "            return AVERROR(ENOSYS);\n"
         "        }\n"
         "        number = CFNumberCreate(NULL, kCFNumberSInt64Type, &vtctx->gpu_registry_id);\n"
         "        if (!number) {\n"
         "            CFRelease(enc_info);\n"
         "            return AVERROR(ENOMEM);\n"
         "        }\n"
         "        CFDictionarySetValue(enc_info, *key, number);\n"
         "        CFRelease(number);\n"
         "    }\n\n"
         "    // low-latency mode: eliminate frame reordering, follow a one-in-one-out encoding mode\n"),
    ]
    replacements += [("libavcodec/amfenc.h", "    int                 usage;\n",
                      "    int                 split_encode;\n"
                      "    int                 split_accepted;\n    int                 usage;\n")]
    for codec, prefix, boost, filler, preencode in [
        ("hevc", "AMF_VIDEO_ENCODER_HEVC_", "HIGH_MOTION_QUALITY_BOOST_ENABLE", "FILLER_DATA_ENABLE", "PREENCODE_ENABLE"),
        ("av1", "AMF_VIDEO_ENCODER_AV1_", "HIGH_MOTION_QUALITY_BOOST", "FILLER_DATA", "RATE_CONTROL_PREENCODE"),
    ]:
        relative = f"libavcodec/amfenc_{codec}.c"
        replacements += [
            (relative, '#include "amfenc.h"\n',
             '#include "amfenc.h"\n#include "serein_amf_split_encoding.h"\n'),
            (relative, "static const AVOption options[] = {\n",
             "static const AVOption options[] = {\n"
             '    { "split_encode", "Request at most two codec engines on this DX11 GPU", OFFSET(split_encode), AV_OPT_TYPE_BOOL, {.i64 = 0}, 0, 1, VE},\n'
             '    { "split_accepted", "AMF accepted the optional split request", OFFSET(split_accepted), AV_OPT_TYPE_BOOL, {.i64 = 0}, 0, 1, VE | AV_OPT_FLAG_READONLY},\n'),
            (relative, "    // init encoder\n",
             "    /* The driver may decline this hint, including below its resolution\n"
             "     * threshold. Ineligible sessions retain their ordinary quality. */\n"
             "    ctx->split_accepted = ctx->split_encode &&\n"
             "        ctx->context->pVtbl->GetDX11Device(ctx->context, AMF_DX11_1) &&\n"
             f"        serein_amf_request_split(ctx->encoder, {prefix}CAP_NUM_OF_HW_INSTANCES,\n"
             f"                                 {prefix}MULTI_HW_INSTANCE_ENCODE);\n"
             "    if (ctx->split_accepted) {\n"
             f"        AMF_ASSIGN_PROPERTY_BOOL(res, ctx->encoder, {prefix}PRE_ANALYSIS_ENABLE, false);\n"
             f"        AMF_ASSIGN_PROPERTY_BOOL(res, ctx->encoder, {prefix}{preencode}, false);\n"
             f"        AMF_ASSIGN_PROPERTY_BOOL(res, ctx->encoder, {prefix}{filler}, false);\n"
             f"        AMF_ASSIGN_PROPERTY_BOOL(res, ctx->encoder, {prefix}{boost}, false);\n"
             "    } else {\n"
             f"        AMF_ASSIGN_PROPERTY_BOOL(res, ctx->encoder, {prefix}MULTI_HW_INSTANCE_ENCODE, false);\n"
             "    }\n\n    // init encoder\n"),
        ]
    replacements += [
        ("libavcodec/qsvenc.c", "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n",
         "    if ((avctx->codec_id == AV_CODEC_ID_HEVC &&\n"
         "         !serein_qsv_tiles_match(q->tile_cols, q->exthevctiles.NumTileColumns)) ||\n"
         "        (avctx->codec_id == AV_CODEC_ID_AV1 &&\n"
         "         !serein_qsv_tiles_match(q->tile_cols, q->extav1tileparam.NumTileColumns)))\n"
         "        return AVERROR(ENOSYS);\n\n"
         "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n"),
        ("libavcodec/qsvenc.c", "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n",
         "    if (avctx->codec_id == AV_CODEC_ID_HEVC &&\n"
         "        !serein_qsv_tiles_match(q->tile_cols, hevc_tile_buf.NumTileColumns))\n"
         "        return AVERROR(ENOSYS);\n\n"
         "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n"),
    ]
    replacements += [
        ("libavcodec/amfenc.c", '#include "libavutil/hwcontext.h"\n',
         '#include "libavutil/hwcontext.h"\n'
         '#if CONFIG_VULKAN\n#include "libavutil/hwcontext_vulkan.h"\n'
         '#include <AMF/core/VulkanAMF.h>\n#endif\n'),
        ("libavcodec/amfenc.c", "#if CONFIG_D3D11VA\nstatic int amf_init_from_d3d11_device",
         "#if CONFIG_VULKAN\n"
         "static int amf_init_from_vulkan_device(AVCodecContext *avctx, AVVulkanDeviceContext *hwctx)\n"
         "{\n"
         "    AmfContext *ctx = avctx->priv_data;\n"
         "    AMFContext1 *context1 = NULL;\n"
         "    AMFGuid guid = IID_AMFContext1();\n"
         "    AMFVulkanDevice device = {0};\n"
         "    AMF_RESULT result;\n"
         "    if (!hwctx->inst || !hwctx->phys_dev || !hwctx->act_dev)\n"
         "        return AVERROR(EINVAL);\n"
         "    result = ctx->context->pVtbl->QueryInterface(ctx->context, &guid, (void **)&context1);\n"
         "    if (result != AMF_OK || !context1)\n"
         "        return AVERROR(ENOSYS);\n"
         "    device.cbSizeof = sizeof(device);\n"
         "    device.hInstance = hwctx->inst;\n"
         "    device.hPhysicalDevice = hwctx->phys_dev;\n"
         "    device.hDevice = hwctx->act_dev;\n"
         "    result = context1->pVtbl->InitVulkan(context1, &device);\n"
         "    context1->pVtbl->Release(context1);\n"
         "    if (result != AMF_OK)\n"
         "        av_log(avctx, AV_LOG_ERROR, \"AMF initialization on the selected Vulkan device failed (result=%d).\\n\", result);\n"
         "    return result == AMF_OK ? 0 : AVERROR(ENODEV);\n"
         "}\n#endif\n\n"
         "#if CONFIG_D3D11VA\nstatic int amf_init_from_d3d11_device"),
        ("libavcodec/amfenc.c", "        switch (device_ctx->type) {\n",
         "        switch (device_ctx->type) {\n"
         "#if CONFIG_VULKAN\n"
         "        case AV_HWDEVICE_TYPE_VULKAN:\n"
         "            ret = amf_init_from_vulkan_device(avctx, device_ctx->hwctx);\n"
         "            if (ret < 0)\n"
         "                return ret;\n"
         "            break;\n"
         "#endif\n"),
        ("libavutil/hwcontext_vulkan.c", "        dev_select.drm_major = major(drm_node_info.st_dev);\n",
         "        dev_select.drm_major = major(drm_node_info.st_rdev);\n"),
        ("libavutil/hwcontext_vulkan.c", "        dev_select.drm_minor = minor(drm_node_info.st_dev);\n",
         "        dev_select.drm_minor = minor(drm_node_info.st_rdev);\n"),
        ("libavutil/hwcontext_vulkan.c", "/* Finds a device */\n",
         """/* Device extensions are known only after a device is selected. Query each
 * physical device here instead of testing the as-yet-unset device flag. */
static int serein_vk_has_drm_extension(AVHWDeviceContext *ctx, VkPhysicalDevice device)
{
    VulkanDevicePriv *p = ctx->hwctx;
    FFVulkanFunctions *vk = &p->vkctx.vkfn;

    for (int attempt = 0; attempt < 3; attempt++) {
        VkExtensionProperties *extensions;
        uint32_t count = 0;
        VkResult ret = vk->EnumerateDeviceExtensionProperties(device, NULL, &count, NULL);
        if (ret != VK_SUCCESS)
            return AVERROR_EXTERNAL;
        if (!count)
            return 0;
        if (count > 4096)
            return AVERROR(E2BIG);
        extensions = av_malloc_array(count, sizeof(*extensions));
        if (!extensions)
            return AVERROR(ENOMEM);
        ret = vk->EnumerateDeviceExtensionProperties(device, NULL, &count, extensions);
        if (ret == VK_INCOMPLETE) {
            av_free(extensions);
            continue;
        }
        if (ret != VK_SUCCESS) {
            av_free(extensions);
            return AVERROR_EXTERNAL;
        }
        for (uint32_t i = 0; i < count; i++) {
            if (!strcmp(extensions[i].extensionName, VK_EXT_PHYSICAL_DEVICE_DRM_EXTENSION_NAME)) {
                av_free(extensions);
                return 1;
            }
        }
        av_free(extensions);
        return 0;
    }
    return AVERROR_EXTERNAL;
}

/* Finds a device */
"""),
        ("libavutil/hwcontext_vulkan.c", "    int err = 0, choice = -1;\n",
         "    int err = 0, choice = -1, drm_query_error = 0;\n"),
        ("libavutil/hwcontext_vulkan.c",
         "    if (p->vkctx.extensions & FF_VK_EXT_DEVICE_DRM) {\n        drm_prop = av_calloc(num, sizeof(*drm_prop));\n",
         "    if (select->has_drm) {\n        drm_prop = av_calloc(num, sizeof(*drm_prop));\n"),
        ("libavutil/hwcontext_vulkan.c",
         """        if (p->vkctx.extensions & FF_VK_EXT_DEVICE_DRM) {
            drm_prop[i].sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT;
            idp[i].pNext = &drm_prop[i];
        }
""",
         """        if (drm_prop) {
            int supported = serein_vk_has_drm_extension(ctx, devices[i]);
            if (supported < 0) {
                av_log(ctx, AV_LOG_ERROR, "Unable to query Vulkan device extensions for GPU %d.\\n", i);
                if (!drm_query_error)
                    drm_query_error = supported;
            } else if (supported) {
                drm_prop[i].sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT;
                idp[i].pNext = &drm_prop[i];
            }
        }
"""),
        ("libavutil/hwcontext_vulkan.c",
         "    } else if ((p->vkctx.extensions & FF_VK_EXT_DEVICE_DRM) && select->has_drm) {\n",
         "    } else if (select->has_drm) {\n"),
        ("libavutil/hwcontext_vulkan.c",
         """            if ((select->drm_major == drm_prop[i].primaryMajor &&
                 select->drm_minor == drm_prop[i].primaryMinor) ||
                (select->drm_major == drm_prop[i].renderMajor &&
                 select->drm_minor == drm_prop[i].renderMinor)) {
""",
         """            if ((drm_prop[i].hasPrimary && select->drm_major == drm_prop[i].primaryMajor &&
                 select->drm_minor == drm_prop[i].primaryMinor) ||
                (drm_prop[i].hasRender && select->drm_major == drm_prop[i].renderMajor &&
                 select->drm_minor == drm_prop[i].renderMinor)) {
"""),
        ("libavutil/hwcontext_vulkan.c",
         """        av_log(ctx, AV_LOG_ERROR, "Unable to find device by given DRM node numbers %i:%i!\\n",
               select->drm_major, select->drm_minor);
        err = AVERROR(ENODEV);
""",
         """        av_log(ctx, AV_LOG_ERROR, "Unable to find device by given DRM node numbers %i:%i!\\n",
               select->drm_major, select->drm_minor);
        err = drm_query_error ? drm_query_error : AVERROR(ENODEV);
"""),
        ("libavcodec/qsvenc.c", "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n",
         "    /* Never silently add reordered pictures or drop requested quality features.\n"
         "     * Startup can retry conservative settings on this same physical adapter. */\n"
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, q->extco2.ExtBRC,\n"
         "        q->extco2.LookAheadDepth, q->param.mfx.RateControlMethod, q->serein_requested_rate_control) ||\n"
         "        q->preset != q->param.mfx.TargetUsage)\n"
         "        return AVERROR(ENOSYS);\n"
         "    if (q->param.mfx.GopRefDist > 0)\n"
         "        avctx->max_b_frames = q->param.mfx.GopRefDist - 1;\n\n"
         "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n"),
        ("libavcodec/qsvenc.c", "    q->packet_size = q->param.mfx.BufferSizeInKB * q->param.mfx.BRCParamMultiplier * 1000;\n    dump_video_av1_param(avctx, q, ext_buffers);\n",
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, co2.ExtBRC,\n"
         "        co2.LookAheadDepth, q->param.mfx.RateControlMethod, q->serein_requested_rate_control) ||\n"
         "        q->preset != q->param.mfx.TargetUsage)\n"
         "        return AVERROR(ENOSYS);\n"
         "    if (q->param.mfx.GopRefDist > 0)\n"
         "        avctx->max_b_frames = q->param.mfx.GopRefDist - 1;\n"
         "    q->packet_size = q->param.mfx.BufferSizeInKB * q->param.mfx.BRCParamMultiplier * 1000;\n    dump_video_av1_param(avctx, q, ext_buffers);\n"),
        ("libavcodec/qsvenc.c", "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n",
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, co2.ExtBRC,\n"
         "        co2.LookAheadDepth, q->param.mfx.RateControlMethod, q->serein_requested_rate_control) ||\n"
         "        q->preset != q->param.mfx.TargetUsage)\n"
         "        return AVERROR(ENOSYS);\n"
         "    if (q->param.mfx.GopRefDist > 0)\n"
         "        avctx->max_b_frames = q->param.mfx.GopRefDist - 1;\n\n"
         "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n"),
    ]
    replacements += [
        ("libavcodec/qsvenc.c", "    dump_video_av1_param(avctx, q, ext_buffers);\n",
         "    if (!serein_qsv_tiles_match(q->tile_cols, av1_extend_tile_buf.NumTileColumns))\n"
         "        return AVERROR(ENOSYS);\n"
         "    dump_video_av1_param(avctx, q, ext_buffers);\n"),
    ]
    replacements += amf_control_patches()
    replacements += amf_vulkan_patches()
    for relative, old, new in replacements:
        path = tree / relative
        before = path.read_text()
        originals.setdefault(relative, before)
        if before.count(old) != 1:
            raise ValueError(f"Expected exactly one FFmpeg source patch target: {relative}")
        after = before.replace(old, new)
        path.write_text(after)
    # Multiple edits to the same upstream file must become one coherent diff.
    # Sequential overlapping diffs apply forward but cannot reverse reliably.
    for relative, original in originals.items():
        patches.extend(difflib.unified_diff(original.splitlines(keepends=True),
                                           (tree / relative).read_text().splitlines(keepends=True),
                                           fromfile="a/" + relative, tofile="b/" + relative))
    for relative, contents in [
        ("libavcodec/serein_qsv_feature_validation.h", QSV_FEATURE_VALIDATION),
        ("libavcodec/serein_amf_split_encoding.h", AMF_SPLIT_ENCODING),
        ("libavcodec/serein_amf_control.h", AMF_CONTROL),
    ]:
        if (tree / relative).exists():
            raise ValueError(f"FFmpeg generated header already exists: {relative}")
        (tree / relative).write_text(contents)
        patches.extend(difflib.unified_diff([], contents.splitlines(keepends=True),
                                           fromfile="/dev/null", tofile="b/" + relative))
    return "".join(patches)


def patch_openh264(tree):
    """Isolate the Linux encoder from host GStreamer's incompatible OpenH264 ABI."""
    patches = []
    api_guard = "#define WELS_VIDEO_CODEC_SVC_API_H__\n"
    macros = "\n/* Private Serein ABI; never satisfy a host plugin's Wels imports. */\n" + "".join(
        f"#define {name} serein_{name}\n" for name in OPENH264_APIS)
    soname = "SHLDFLAGS = -Wl,-soname,$(LIBPREFIX)$(PROJECT_NAME).$(SHAREDLIBSUFFIXMAJORVER)"
    replacements = [
        ("codec/api/wels/codec_api.h", api_guard, api_guard + macros),
        ("build/platform-gnu-chain.mk", soname,
         soname.replace("$(PROJECT_NAME).", "$(PROJECT_NAME)-serein.") +
         "\nSHLDFLAGS += -Wl,--version-script,$(SRC_PATH)serein-openh264.map"),
    ]
    for relative, old, new in replacements:
        path = tree / relative
        before = path.read_text()
        if before.count(old) != 1 or "serein_" in before:
            raise ValueError(f"Expected exactly one unpatched OpenH264 source target: {relative}")
        after = before.replace(old, new)
        patches.extend(difflib.unified_diff(before.splitlines(keepends=True), after.splitlines(keepends=True),
                                           fromfile="a/" + relative, tofile="b/" + relative))
        path.write_text(after)
    # Hide internal C++ and assembly symbols too; they otherwise interpose even
    # when the seven public entry points have distinct names and a private SONAME.
    exports = "SEREIN_OPENH264_8 {\n  global:\n" + "".join(
        f"    serein_{name};\n" for name in OPENH264_APIS) + "  local: *;\n};\n"
    relative = "serein-openh264.map"
    (tree / relative).write_text(exports)
    patches.extend(difflib.unified_diff([], exports.splitlines(keepends=True),
                                       fromfile="/dev/null", tofile="b/" + relative))
    return "".join(patches)


def validate_completed_build(prefix, system, backends):
    """A recipe stamp is insufficient if a restored build lost required files."""
    required = [f"include/{name}" for name in (
        "libavcodec/avcodec.h", "libavutil/avutil.h", "libavutil/error.h", "libavutil/frame.h",
        "libavutil/hwcontext.h", "libavutil/mem.h", "libavutil/opt.h",
        "libavformat/avformat.h", "libavfilter/avfilter.h", "libavfilter/buffersrc.h", "libavfilter/buffersink.h",
        "libswscale/swscale.h", "libswresample/swresample.h")]
    required += ["lib/pkgconfig/libavcodec-serein.pc", "lib/pkgconfig/libavutil-serein.pc"]
    required += {
        "Linux": ["lib/libavcodec-serein.so", "lib/libavcodec-serein.so.61",
                  "lib/libavutil-serein.so", "lib/libavutil-serein.so.59", "lib/libopenh264-serein.so.8"],
        "Darwin": ["lib/libavcodec-serein.dylib", "lib/libavcodec-serein.61.dylib",
                   "lib/libavutil-serein.dylib", "lib/libavutil-serein.59.dylib", "lib/libopenh264.8.dylib"],
        "Windows": ["bin/avcodec-serein-61.dll", "bin/avutil-serein-59.dll", "bin/openh264.dll",
                    "lib/avcodec-serein.lib", "lib/avutil-serein.lib"],
    }[system]
    for name, version in (("avformat", "61"), ("avfilter", "10"), ("swscale", "8"), ("swresample", "5")):
        required.append(f"lib/pkgconfig/lib{name}-serein.pc")
        if system == "Linux":
            required += [f"lib/lib{name}-serein.so", f"lib/lib{name}-serein.so.{version}"]
        elif system == "Darwin":
            required += [f"lib/lib{name}-serein.dylib", f"lib/lib{name}-serein.{version}.dylib"]
        else:
            required += [f"bin/{name}-serein-{version}.dll", f"lib/{name}-serein.lib"]
    provenance = ["build-ffmpeg.py", "configure.json", "serein-ffmpeg.patch", "COPYING.LGPLv2.1", "COPYING.GPLv2", "OpenH264-LICENSE",
                  "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2", "source/dav1d-1.5.1.tar.xz", "dav1d-COPYING"]
    if system == "Linux":
        provenance.append("serein-openh264.patch")
    if backends["nvenc"]:
        required += ["include/ffnvcodec/nvEncodeAPI.h", "include/ffnvcodec/dynlink_cuda.h"]
        provenance += ["nv-codec-headers-README", "source/nv-codec-headers-12.2.72.0.tar.gz"]
    if backends["amf"]:
        required += ["include/AMF/core/Factory.h", "include/AMF/components/ComponentCaps.h",
                     "include/AMF/components/VideoEncoderVCE.h", "include/AMF/components/VideoEncoderHEVC.h",
                     "include/AMF/components/VideoEncoderAV1.h"]
        provenance += ["AMF-LICENSE", "source/AMF-1.4.36-headers.tar"]
        if system == "Linux":
            required += ["include/vulkan/vulkan.h", "include/vulkan/vulkan_core.h"]
            provenance += ["Vulkan-Headers-LICENSE.md", "Vulkan-Headers-LICENSES/Apache-2.0.txt",
                           "Vulkan-Headers-LICENSES/MIT.txt", "source/Vulkan-Headers-1.3.290.tar.gz"]
    if backends["qsv"]:
        required += ["include/vpl/mfxdispatcher.h", "include/vpl/mfxstructures.h", "lib/pkgconfig/vpl.pc",
                     "lib/vpl.lib" if system == "Windows" else "lib/libvpl.a"]
        provenance += ["oneVPL-LICENSE", "oneVPL-third-party-programs.txt", "source/libvpl-2.14.0.tar.gz"]
    required += ["share/serein-ffmpeg/" + name for name in provenance]
    for name in required:
        path = prefix / name
        # Installed library aliases are symlinks; is_file/stat validate their
        # actual targets too, including a broken alias after partial cache restore.
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Incomplete FFmpeg prefix (missing/empty {name}); "
                             "choose fresh --prefix and --work-dir paths")


def build(args):
    system = platform.system()
    if system.startswith(("MSYS", "MINGW", "CYGWIN")):
        system = "Windows"
    if system not in {"Linux", "Darwin", "Windows"}:
        raise ValueError(f"Unsupported native FFmpeg build host: {system}")
    architecture = os.environ.get("VSCMD_ARG_TGT_ARCH", platform.machine()).lower()
    arm64 = architecture in {"arm64", "aarch64"}
    if architecture not in {"x86_64", "amd64", "x64", "arm64", "aarch64"}:
        raise ValueError(f"Unsupported native FFmpeg architecture: {architecture}")
    backends = encoder_backends(system, arm64)
    toolchain = toolchain_options(system, arm64)
    nvenc = backends["nvenc"]
    prefix = args.prefix.resolve()
    cache = args.cache_dir.resolve()
    work = args.work_dir.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    recipe = {"sources": SOURCES, "system": system, "architecture": architecture,
              "license": "GPL-2.0-or-later", "filters": ["atadenoise", "hqdn3d", "unsharp"],
              **backends, "encoders": sorted(encoder_names(backends)), "decoders": sorted(decoder_names(backends)), "toolchain": toolchain, "recipe_sha256":
              hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    stamp = prefix / "share/serein-ffmpeg/build.json"
    if stamp.is_file() and json.loads(stamp.read_text()) == recipe:
        validate_completed_build(prefix, system, backends)
        print(f"Reusing verified FFmpeg build: {prefix}")
        return prefix
    if prefix.exists() and any(prefix.iterdir()):
        raise ValueError(f"FFmpeg prefix contains another build; choose a fresh prefix: {prefix}")
    if work.exists() and any(work.iterdir()):
        raise ValueError(f"FFmpeg work directory contains an incomplete build; choose a fresh work directory: {work}")
    work.mkdir(parents=True, exist_ok=True)
    required = ["ffmpeg", "dav1d"] + (["nv-codec-headers"] if nvenc else []) + (["onevpl"] if backends["qsv"] else [])
    archives = {name: fetch(SOURCES[name], cache, args.offline) for name in required}
    archives["openh264-source"] = openh264_source(cache, args.offline)
    if backends["amf"]:
        archives["amf-headers"] = amf_headers(cache, args.offline)
        if system == "Linux":
            archives["vulkan-headers"] = fetch(SOURCES["vulkan-headers"], cache, args.offline)
    trees = {name: unpack(archive, work / name) for name, archive in archives.items()}
    # Keep host GStreamer's FFmpeg in its own ELF symbol namespace. A distinct
    # SONAME alone cannot prevent LIBAVCODEC_61/LIBAVUTIL_59 interposition.
    source_patch = patch_ffmpeg(trees["ffmpeg"])
    openh264_patch = patch_openh264(trees["openh264-source"]) if system == "Linux" else None
    env = dict(os.environ)
    shell = configure_shell(system, env)
    env["PKG_CONFIG_PATH"] = posix(prefix / "lib/pkgconfig") + os.pathsep + env.get("PKG_CONFIG_PATH", "")
    # MSYS pkgconf uses ':' even when driven by Windows Python.
    if system == "Windows":
        env["PKG_CONFIG_PATH"] = posix(prefix / "lib/pkgconfig")
    openh264_args = [f"PREFIX={posix(prefix)}", f"ARCH={'arm64' if arm64 else 'x86_64'}"]
    if system == "Windows":
        openh264_args += ["OS=msvc", "USE_ASM=No" if arm64 else "USE_ASM=Yes"]
    else:
        # Command-line assignments override GNU make's built-in CXX=g++, and
        # the same environment selects oneVPL's CMake compiler wrappers.
        env["CC"] = env.get("CC") or "cc"
        env["CXX"] = env.get("CXX") or "c++"
        openh264_args += [f"CC={env['CC']}", f"CXX={env['CXX']}"]
    run("make", f"-j{args.jobs}", *openh264_args, "install-shared", cwd=trees["openh264-source"], env=env)
    if system == "Linux":
        # Retain the upstream development linker alias/pkg-config name, but give
        # the runtime dependency its own SONAME and packaged filename. A host
        # OpenH264 2.6 plugin must still load its own libopenh264.so.8.
        library = prefix / "lib/libopenh264.so.8"
        (prefix / "lib/libopenh264-serein.so.8").symlink_to(library.resolve().name)
    if system == "Windows":
        # FFmpeg/pkgconf's -lopenh264 must select the shared import library.
        shutil.copyfile(prefix / "lib/openh264_dll.lib", prefix / "lib/openh264.lib")
    if "vulkan-headers" in trees:
        shutil.copytree(trees["vulkan-headers"] / "include", prefix / "include", dirs_exist_ok=True)
    if nvenc:
        run("make", f"PREFIX={posix(prefix)}", "install", cwd=trees["nv-codec-headers"], env=env)
    if backends["amf"]:
        shutil.copytree(trees["amf-headers"] / "AMF", prefix / "include/AMF")
    if backends["qsv"]:
        vpl_build = work / "onevpl-cmake"
        # vpl.pc advertises dl/pthread/C++ dependencies in Libs on Linux.
        # Build only the PIC static dispatcher, which loads the system GPU runtime.
        # oneVPL 2.14 checks MSVC before its first project(), so its CMP0091
        # setup misses fresh builds. Select NEW before compiler detection to
        # make the upstream static-runtime option produce /MT instead of /MD.
        run("cmake", "-S", trees["onevpl"], "-B", vpl_build,
            "-G", "NMake Makefiles" if system == "Windows" else "Unix Makefiles",
            f"-DCMAKE_INSTALL_PREFIX={prefix.as_posix()}", "-DCMAKE_INSTALL_LIBDIR=lib",
            "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF", "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            "-DBUILD_TESTS=OFF", "-DBUILD_EXAMPLES=OFF", "-DINSTALL_DEV=ON", "-DINSTALL_LIB=ON",
            "-DCMAKE_INSTALL_SYSTEM_RUNTIME_LIBS_SKIP=ON",
            f"-DUSE_MSVC_STATIC_RUNTIME={'ON' if system == 'Windows' else 'OFF'}",
            *(["-DCMAKE_POLICY_DEFAULT_CMP0091=NEW"] if system == "Windows" else []), env=env)
        run("cmake", "--build", vpl_build, "--parallel", str(args.jobs), env=env)
        run("cmake", "--install", vpl_build, env=env)
        if system == "Windows":
            # Upstream's MSVC pkg-config file omits static Win32 dependencies.
            # The dispatcher uses registry APIs, StringFromGUID2 and device GUIDs.
            pc = prefix / "lib/pkgconfig/vpl.pc"
            content = pc.read_text()
            content = content.replace("Libs: ", "Libs: -ladvapi32 -lole32 -luuid ", 1)
            pc.write_text(content)
    dav1d_build = work / "dav1d-meson"
    meson_env = meson_environment(system, env)
    run("meson", "setup", dav1d_build, trees["dav1d"], f"--prefix={prefix.as_posix()}",
        "--libdir=lib", "--buildtype=release", "--default-library=static",
        "-Db_staticpic=true", "-Db_vscrt=mt", "-Denable_tools=false", "-Denable_tests=false",
        *(["-Denable_asm=false"] if system == "Windows" and arm64 else []), env=meson_env)
    run("meson", "compile", "-C", dav1d_build, "-j", str(args.jobs), env=meson_env)
    run("meson", "install", "-C", dav1d_build, env=meson_env)
    if system == "Windows":
        # dav1d overrides Meson's static archive name to libdav1d.a even with
        # MSVC. FFmpeg translates pkg-config's -ldav1d to dav1d.lib.
        archive = prefix / "lib/libdav1d.a"
        if archive.is_file():
            archive.replace(prefix / "lib/dav1d.lib")
        if not (prefix / "lib/dav1d.lib").is_file():
            raise ValueError("dav1d's MSVC static archive was not installed")
    configure = [
        f"--prefix={posix(prefix)}", "--build-suffix=-serein", "--disable-autodetect", "--disable-everything",
        "--disable-programs", "--disable-doc", "--disable-network", "--disable-static", "--enable-shared",
        "--enable-gpl", "--disable-nonfree", "--disable-version3", "--disable-avdevice", "--enable-avformat",
        "--enable-avfilter", "--enable-filter=buffer,buffersink,atadenoise,hqdn3d,unsharp", "--enable-swscale", "--enable-swresample", "--disable-postproc",
        "--disable-vdpau", "--disable-cuvid", "--disable-cuda-llvm",
        "--enable-avcodec", "--enable-avutil", "--enable-libopenh264", "--enable-encoder=libopenh264",
        "--enable-libdav1d", "--enable-decoder=" + ",".join(sorted(decoder_names(backends))),
        "--enable-demuxer=mov,matroska", "--enable-parser=h264,hevc,av1,aac,opus,vorbis,mpegaudio",

        f"--extra-cflags=-I{posix(prefix / 'include')}",
        f"--extra-ldflags=-L{posix(prefix / 'lib')}" + (" -Wl,-z,defs" if system == "Linux" else ""),
    ]
    configure += hardware_options(system, backends)
    configure += toolchain
    if system == "Windows":
        configure = [arg for arg in configure if not arg.startswith(("--extra-ldflags=", "--extra-cflags="))]
        # Match OpenH264's -MT and oneVPL's internal allocation/free ownership;
        # no additional Visual C++ redistributable is needed for these DLLs.
        configure += [f"--extra-cflags=-I{posix(prefix / 'include')} -MT",
                      f"--extra-ldflags=-libpath:{(prefix / 'lib').as_posix()}"]
    run(shell, "configure", *configure, cwd=trees["ffmpeg"], env=env)
    configuration = (trees["ffmpeg"] / "config.h").read_text()
    if ("#define CONFIG_GPL 1" not in configuration or "#define CONFIG_NONFREE 0" not in configuration
            or "#define CONFIG_VERSION3 0" not in configuration):
        raise ValueError("FFmpeg build must be GPLv2-or-later without nonfree or version-3-only components")
    component_config = (trees["ffmpeg"] / "config_components.h").read_text()
    expected = {name.upper() for name in encoder_names(backends)}
    actual = {line.split()[1].removeprefix("CONFIG_").removesuffix("_ENCODER")
              for line in component_config.splitlines() if line.startswith("#define CONFIG_")
              and "_ENCODER 1" in line}
    if actual != expected:
        raise ValueError(f"Unexpected FFmpeg encoder allowlist: {sorted(actual)}, expected {sorted(expected)}")
    expected_decoders = {name.upper() for name in decoder_names(backends)}
    actual_decoders = {line.split()[1].removeprefix("CONFIG_").removesuffix("_DECODER")
                       for line in component_config.splitlines() if line.startswith("#define CONFIG_")
                       and "_DECODER 1" in line}
    if actual_decoders != expected_decoders:
        raise ValueError(f"Unexpected FFmpeg decoder allowlist: {sorted(actual_decoders)}")
    # FFmpeg always compiles its four audio/video buffer source/sink filters;
    # they are not CONFIG_*_FILTER entries. Enable only the webcam filter chain.
    expected_filters = {"ATADENOISE", "HQDN3D", "UNSHARP"}
    actual_filters = {line.split()[1].removeprefix("CONFIG_").removesuffix("_FILTER")
                      for line in component_config.splitlines() if line.startswith("#define CONFIG_")
                      and "_FILTER 1" in line}
    if actual_filters != expected_filters:
        raise ValueError(f"Unexpected FFmpeg filter allowlist: {sorted(actual_filters)}, expected {sorted(expected_filters)}")
    if "#define CONFIG_NETWORK 0" not in configuration:
        raise ValueError("Media decoders must never perform network I/O")
    run("make", f"-j{args.jobs}", cwd=trees["ffmpeg"], env=env)
    run("make", "install", cwd=trees["ffmpeg"], env=env)
    if system == "Windows":
        # FFmpeg installs MSVC import libraries with DLLs in bin; Cargo searches lib.
        for name in ("avcodec-serein.lib", "avutil-serein.lib", "avformat-serein.lib", "avfilter-serein.lib", "swscale-serein.lib", "swresample-serein.lib"):
            shutil.copyfile(prefix / "bin" / name, prefix / "lib" / name)
    if system == "Linux":
        for library in (prefix / "lib").glob("*.so.*"):
            if not library.is_symlink():
                # Keep compiler-runtime store paths in Nix builds, while making
                # the replaceable sibling codecs resolve before those fallbacks.
                original = subprocess.check_output(["patchelf", "--print-rpath", str(library)], text=True).strip()
                paths = [path for path in original.split(":") if path and path != str(prefix / "lib") and path != "$ORIGIN"]
                run("patchelf", "--set-rpath", ":".join(["$ORIGIN", *paths]), library)
    if system == "Darwin":
        for library in (prefix / "lib").glob("*.dylib"):
            if library.is_symlink():
                continue
            # FFmpeg already uses @rpath and its major-version IDs. The physical
            # filenames include minor versions and are not shipped as aliases.
            if library.name.startswith("libopenh264"):
                run("install_name_tool", "-id", "@rpath/libopenh264.8.dylib", library)
            dependencies = subprocess.check_output(["otool", "-L", str(library)], text=True).splitlines()[1:]
            for line in dependencies:
                name = line.strip().split(" (", 1)[0]
                if name.startswith(str(prefix / "lib") + "/"):
                    run("install_name_tool", "-change", name, f"@rpath/{Path(name).name}", library)
            # Direct native consumers also load these dylibs outside the app.
            # Resolve private dependencies beside each library in both the
            # development prefix and the packaged Frameworks directory.
            run("install_name_tool", "-add_rpath", "@loader_path", library)
        # Apple Silicon rejects modified Mach-O files whose linker signatures
        # were invalidated above. Sign the source-run/Nix libraries after every
        # install-name edit; packaging signs its staged copies again afterward.
        for library in (prefix / "lib").glob("*.dylib"):
            if not library.is_symlink():
                run("codesign", "--force", "--sign", "-", library)
    notices = stamp.parent
    (notices / "source").mkdir(parents=True)
    # Corresponding FFmpeg source and license texts travel with every official binary.
    for name, archive in archives.items():
        shutil.copyfile(archive, notices / "source" / SOURCES[name]["file"])
    shutil.copyfile(Path(__file__), notices / "build-ffmpeg.py")
    (notices / "configure.json").write_text(json.dumps(configure, indent=2) + "\n")
    (notices / "serein-ffmpeg.patch").write_text(source_patch)
    if openh264_patch is not None:
        (notices / "serein-openh264.patch").write_text(openh264_patch)
    shutil.copyfile(trees["ffmpeg"] / "COPYING.LGPLv2.1", notices / "COPYING.LGPLv2.1")
    shutil.copyfile(trees["ffmpeg"] / "COPYING.GPLv2", notices / "COPYING.GPLv2")
    shutil.copyfile(trees["dav1d"] / "COPYING", notices / "dav1d-COPYING")
    shutil.copyfile(trees["openh264-source"] / "LICENSE", notices / "OpenH264-LICENSE")
    if nvenc:
        shutil.copyfile(trees["nv-codec-headers"] / "README", notices / "nv-codec-headers-README")
    if backends["amf"]:
        shutil.copyfile(trees["amf-headers"] / "LICENSE.txt", notices / "AMF-LICENSE")
    if "vulkan-headers" in trees:
        shutil.copyfile(trees["vulkan-headers"] / "LICENSE.md", notices / "Vulkan-Headers-LICENSE.md")
        shutil.copytree(trees["vulkan-headers"] / "LICENSES", notices / "Vulkan-Headers-LICENSES")
    if backends["qsv"]:
        shutil.copyfile(trees["onevpl"] / "LICENSE", notices / "oneVPL-LICENSE")
        shutil.copyfile(trees["onevpl"] / "third-party-programs.txt", notices / "oneVPL-third-party-programs.txt")
    stamp.write_text(json.dumps(recipe, indent=2) + "\n")
    print(f"FFMPEG_DIR={prefix}")
    return prefix


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=Path("target/ffmpeg/prefix"))
    parser.add_argument("--work-dir", type=Path, default=Path("target/ffmpeg/build"))
    parser.add_argument("--cache-dir", type=Path, default=Path("target/ffmpeg/sources"))
    parser.add_argument("--jobs", type=int, default=min(os.cpu_count() or 1, 8))
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.jobs <= 256:
        parser.error("--jobs must be between 1 and 256")
    build(args)


if __name__ == "__main__":
    main()
