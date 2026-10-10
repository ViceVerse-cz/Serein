#!/usr/bin/env python3
"""Build Serein's LGPL-only shared video encoders; never capture or open media.

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
    "ffmpeg": {
        "file": "ffmpeg-7.1.5.tar.xz",
        "url": "https://ffmpeg.org/releases/ffmpeg-7.1.5.tar.xz",
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
    unsigned int driver_depth, unsigned int driver_rate_control)
{
    if (requested_b == 0 && reference_distance != 1)
        return 0;
    if (requested_b > 0 && (!reference_distance || reference_distance > (unsigned int)requested_b + 1))
        return 0;
    if (requested_depth > 0 && (requested_extbrc <= 0 || driver_extbrc != MFX_CODINGOPTION_ON ||
        driver_depth != (unsigned int)requested_depth || driver_rate_control != MFX_RATECONTROL_CBR))
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
    return options


def patch_ffmpeg(tree):
    patches = []
    originals = {}
    replacements = [("libavcodec/libavcodec.v", "LIBAVCODEC_MAJOR", "SEREIN_LIBAVCODEC_MAJOR"),
                    ("libavutil/libavutil.v", "LIBAVUTIL_MAJOR", "SEREIN_LIBAVUTIL_MAJOR"),
                    ("libavcodec/qsvenc.c", "ret = av_new_packet(&pkt.pkt, q->packet_size);",
                     "ret = ff_get_encode_buffer(avctx, &pkt.pkt, q->packet_size, 0);"),
                    ("configure", 'hevc_qsv_encoder_select="hevcparse qsvenc"',
                     'hevc_qsv_encoder_select="hevcparse hevc_sei qsvenc"')]
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
        ("libavutil/hwcontext_vulkan.c", "    if (select->has_uuid) {\n",
         "    /* Explicit DRM selection may never fall through to vendor/model matching. */\n"
         "    if (select->has_drm && !(p->vkctx.extensions & FF_VK_EXT_DEVICE_DRM)) {\n"
         "        av_log(ctx, AV_LOG_ERROR, \"Exact DRM adapter selection requires VK_EXT_physical_device_drm.\\n\");\n"
         "        err = AVERROR(ENOSYS);\n"
         "        goto end;\n"
         "    }\n\n"
         "    if (select->has_uuid) {\n"),
        ("libavcodec/qsvenc.c", "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n",
         "    /* Never silently add reordered pictures or drop requested quality features.\n"
         "     * Startup can retry conservative settings on this same physical adapter. */\n"
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, q->extco2.ExtBRC,\n"
         "        q->extco2.LookAheadDepth, q->param.mfx.RateControlMethod))\n"
         "        return AVERROR(ENOSYS);\n"
         "    if (q->param.mfx.GopRefDist > 0)\n"
         "        avctx->max_b_frames = q->param.mfx.GopRefDist - 1;\n\n"
         "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n"),
        ("libavcodec/qsvenc.c", "    q->packet_size = q->param.mfx.BufferSizeInKB * q->param.mfx.BRCParamMultiplier * 1000;\n    dump_video_av1_param(avctx, q, ext_buffers);\n",
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, co2.ExtBRC,\n"
         "        co2.LookAheadDepth, q->param.mfx.RateControlMethod))\n"
         "        return AVERROR(ENOSYS);\n"
         "    if (q->param.mfx.GopRefDist > 0)\n"
         "        avctx->max_b_frames = q->param.mfx.GopRefDist - 1;\n"
         "    q->packet_size = q->param.mfx.BufferSizeInKB * q->param.mfx.BRCParamMultiplier * 1000;\n    dump_video_av1_param(avctx, q, ext_buffers);\n"),
        ("libavcodec/qsvenc.c", "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n",
         "    if (!serein_qsv_quality_matches(avctx->max_b_frames, q->param.mfx.GopRefDist,\n"
         "        q->look_ahead_depth, q->extbrc, co2.ExtBRC,\n"
         "        co2.LookAheadDepth, q->param.mfx.RateControlMethod))\n"
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
        "libavutil/hwcontext.h", "libavutil/mem.h", "libavutil/opt.h")]
    required += ["lib/pkgconfig/libavcodec-serein.pc", "lib/pkgconfig/libavutil-serein.pc"]
    required += {
        "Linux": ["lib/libavcodec-serein.so", "lib/libavcodec-serein.so.61",
                  "lib/libavutil-serein.so", "lib/libavutil-serein.so.59", "lib/libopenh264-serein.so.8"],
        "Darwin": ["lib/libavcodec-serein.dylib", "lib/libavcodec-serein.61.dylib",
                   "lib/libavutil-serein.dylib", "lib/libavutil-serein.59.dylib", "lib/libopenh264.8.dylib"],
        "Windows": ["bin/avcodec-serein-61.dll", "bin/avutil-serein-59.dll", "bin/openh264.dll",
                    "lib/avcodec-serein.lib", "lib/avutil-serein.lib"],
    }[system]
    provenance = ["build-ffmpeg.py", "configure.json", "serein-ffmpeg.patch", "COPYING.LGPLv2.1", "OpenH264-LICENSE",
                  "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2"]
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
              **backends, "encoders": sorted(encoder_names(backends)), "toolchain": toolchain, "recipe_sha256":
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
    required = ["ffmpeg"] + (["nv-codec-headers"] if nvenc else []) + (["onevpl"] if backends["qsv"] else [])
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
    configure = [
        f"--prefix={posix(prefix)}", "--build-suffix=-serein", "--disable-autodetect", "--disable-everything",
        "--disable-programs", "--disable-doc", "--disable-network", "--disable-static", "--enable-shared",
        "--disable-gpl", "--disable-nonfree", "--disable-version3", "--disable-avdevice", "--disable-avformat",
        "--disable-avfilter", "--disable-swscale", "--disable-swresample", "--disable-postproc",
        "--disable-vdpau", "--disable-cuvid", "--disable-nvdec", "--disable-cuda-llvm",
        "--enable-avcodec", "--enable-avutil", "--enable-libopenh264", "--enable-encoder=libopenh264",
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
    if "#define CONFIG_GPL 0" not in configuration or "#define CONFIG_NONFREE 0" not in configuration:
        raise ValueError("FFmpeg build must remain LGPL without GPL/nonfree components")
    component_config = (trees["ffmpeg"] / "config_components.h").read_text()
    expected = {name.upper() for name in encoder_names(backends)}
    actual = {line.split()[1].removeprefix("CONFIG_").removesuffix("_ENCODER")
              for line in component_config.splitlines() if line.startswith("#define CONFIG_")
              and "_ENCODER 1" in line}
    if actual != expected:
        raise ValueError(f"Unexpected FFmpeg encoder allowlist: {sorted(actual)}, expected {sorted(expected)}")
    if any(line.startswith("#define CONFIG_") and "_DECODER 1" in line for line in component_config.splitlines()):
        raise ValueError("FFmpeg outgoing encoder build must not include decoders")
    run("make", f"-j{args.jobs}", cwd=trees["ffmpeg"], env=env)
    run("make", "install", cwd=trees["ffmpeg"], env=env)
    if system == "Windows":
        # FFmpeg installs MSVC import libraries with DLLs in bin; Cargo searches lib.
        for name in ("avcodec-serein.lib", "avutil-serein.lib"):
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
        # Apple Silicon rejects modified Mach-O files whose linker signatures
        # were invalidated above. Sign the source-run/Nix libraries after every
        # install-name edit; packaging signs its staged copies again afterward.
        for library in (prefix / "lib").glob("*.dylib"):
            if not library.is_symlink():
                run("codesign", "--force", "--sign", "-", library)
    notices = stamp.parent
    (notices / "source").mkdir(parents=True)
    # The LGPL corresponding FFmpeg source travels with every official binary.
    for name, archive in archives.items():
        shutil.copyfile(archive, notices / "source" / SOURCES[name]["file"])
    shutil.copyfile(Path(__file__), notices / "build-ffmpeg.py")
    (notices / "configure.json").write_text(json.dumps(configure, indent=2) + "\n")
    (notices / "serein-ffmpeg.patch").write_text(source_patch)
    if openh264_patch is not None:
        (notices / "serein-openh264.patch").write_text(openh264_patch)
    shutil.copyfile(trees["ffmpeg"] / "COPYING.LGPLv2.1", notices / "COPYING.LGPLv2.1")
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
