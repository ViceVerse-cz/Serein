#!/usr/bin/env python3
"""Offline driver-query regression fixtures. Never load real GPU drivers.

Usage: python3 crates/discord-voice/tests/native/test_video_queries.py --prefix "$FFMPEG_DIR"
Linux loader fixtures complement the native Windows/macOS compilation checks.
"""
import argparse
import hashlib
import importlib.util
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tarfile
import tempfile


def run_queries(prefix):
    if sys.platform != "linux":
        print("Linux driver-loader fixtures skipped; native platform builds still required")
        return
    here = Path(__file__).resolve().parent
    source = here.parents[3] / "crates/platform/src/video"
    include = prefix / "include"
    compiler = shlex.split(os.environ.get("CC", "cc"))
    flags = ["-std=c11", "-Wall", "-Wextra", "-Werror", "-ffunction-sections", "-fdata-sections",
             "-Wl,--gc-sections", "-I" + str(include), "-I" + str(source)]
    with tempfile.TemporaryDirectory(prefix="serein-driver-fixtures-") as directory:
        out = Path(directory)
        recipe_spec = importlib.util.spec_from_file_location(
            "ffmpeg_recipe", source.parents[3] / "scripts/build-ffmpeg.py")
        recipe = importlib.util.module_from_spec(recipe_spec)
        recipe_spec.loader.exec_module(recipe)
        (out / "serein_qsv_feature_validation.h").write_text(recipe.QSV_FEATURE_VALIDATION)
        if (include / "AMF/components/ComponentCaps.h").is_file():
            (out / "serein_amf_split_encoding.h").write_text(recipe.AMF_SPLIT_ENCODING)
            executable = out / "amf-split-test"
            subprocess.run([*compiler, *flags, "-I" + str(out), str(here / "video_amf_split_test.c"),
                            "-o", str(executable)], check=True)
            subprocess.run([str(executable)], check=True)
        if (include / "ffnvcodec/nvEncodeAPI.h").is_file():
            for variant, soname in [(1, "libcuda.so.1"), (2, "libnvidia-encode.so.1")]:
                subprocess.run([*compiler, *flags, "-fPIC", "-shared",
                                "-DSEREIN_NVENC_FIXTURE=" + str(variant), str(here / "video_query_nvenc_test.c"),
                                "-Wl,-soname," + soname, "-o", str(out / soname)], check=True)
            executable = out / "nvenc-test"
            subprocess.run([*compiler, *flags, "-DSEREIN_HAVE_NVENC_QUERY=1", "-DSEREIN_QUERY_PART=1",
                            str(here / "video_query_nvenc_test.c"), str(source / "video_query.c"),
                            str(source / "video_gpu.c"),
                            "-L" + str(out), "-Wl,--no-as-needed", "-l:libcuda.so.1",
                            "-l:libnvidia-encode.so.1", "-ldl", "-o", str(executable)], check=True)
            subprocess.run([str(executable)], env=dict(os.environ, LD_LIBRARY_PATH=str(out)), check=True)
        else:
            print("NVENC SDK absent: loader fixture skipped")
        if (include / "vpl/mfxdispatcher.h").is_file():
            executable = out / "qsv-test"
            subprocess.run([*compiler, *flags, "-DSEREIN_HAVE_QSV_QUERY=1", "-DSEREIN_QUERY_PART=2",
                            str(here / "video_query_qsv_test.c"), str(source / "video_query.c"),
                            str(source / "video_gpu.c"),
                            "-o", str(executable)], check=True)
            subprocess.run([str(executable)], check=True)
            executable = out / "qsv-quality-test"
            subprocess.run([*compiler, *flags, "-I" + str(out), "-DSEREIN_QSV_QUALITY_FIXTURE=1", str(here / "video_query_qsv_test.c"),
                            "-o", str(executable)], check=True)
            subprocess.run([str(executable)], check=True)
        else:
            print("QSV SDK absent: dispatcher fixture skipped")
        executable = out / "dispatcher-test"
        subprocess.run([*compiler, *flags, "-DSEREIN_HAVE_AMF_QUERY=1", "-DSEREIN_QUERY_PART=4",
                        str(here / "video_query_dispatcher_test.c"), str(source / "video_query.c"),
                        str(source / "video_gpu.c"),
                        "-o", str(executable)], check=True)
        subprocess.run([str(executable)], check=True)
        executable = out / "gpu-binding-test"
        subprocess.run([*compiler, *flags, "-DSEREIN_HAVE_VULKAN_GPU=1",
                        str(here / "video_gpu_binding_test.c"), str(source / "video_gpu.c"),
                        "-Wl,--wrap=readlink", "-o", str(executable)], check=True)
        subprocess.run([str(executable)], check=True)
        executable = out / "no-sdk-test"
        subprocess.run([*compiler, *flags, "-DSEREIN_NO_SDK_FIXTURE=1", str(here / "video_query_dispatcher_test.c"),
                        str(source / "video_query.c"),
                        str(source / "video_gpu.c"),
                        "-o", str(executable)], check=True)
        subprocess.run([str(executable)], check=True)
    if (include / "AMF/core/Factory.h").is_file():
        run_amf(include)
    else:
        print("AMF SDK absent: component fixture skipped")



def run_amf(include_path):
    if sys.platform != "linux":
        print("AMF native mock tests require Linux's dlopen wrapper; skipped.")
        return
    include = Path(include_path).resolve()
    if not (include / "AMF/core/Factory.h").is_file():
        raise RuntimeError(f"AMF SDK headers are missing from {include}")
    fixture_dir = Path(__file__).resolve().parent
    source_dir = fixture_dir.parents[3] / "crates/platform/src/video"
    c_compiler = shlex.split(os.environ.get("CC", "cc"))
    cpp_compiler = shlex.split(os.environ.get("CXX", "c++"))
    sdk_flags = ["-isystem", str(include)]
    warnings = ["-Wall", "-Wextra", "-Werror", "-ffunction-sections", "-fdata-sections", "-Wl,--gc-sections"]
    modes = {
        "support": 1,
        "software": 0,
        "gpu": 0,
        "unsupported": 0,
        "bad-accel": -1,
        "caps-failed": -1,
        "null-caps": -1,
        "bad-count": -1,
        "format-failed": -1,
        "no-nv12": 1,
        "no-420": 0,
        "absent": 0,
        "component-failed": -1,
        "null-component": -1,
        "init-failed": -1,
        "no-device": 0,
        "no-interface": -1,
    }
    with tempfile.TemporaryDirectory(prefix="serein-amf-query-") as temp:
        directory = Path(temp)
        library = directory / "mock-amf.so"
        executable = directory / "query-test"
        subprocess.run(
            c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-fPIC", "-shared", str(fixture_dir / "video_query_amf_mock.c"),
                "-o", str(library),
            ],
            check=True,
        )
        subprocess.run(
            cpp_compiler + ["-std=c++11"] + warnings + sdk_flags + [
                str(source_dir / "video_query_amf.cpp"),
                str(fixture_dir / "video_query_amf_test.cpp"),
                "-Wl,--wrap=dlopen", "-ldl", "-o", str(executable),
            ],
            check=True,
        )
        environment = {**os.environ, "SEREIN_AMF_TEST_RUNTIME": str(library)}
        for codec in range(3):
            for mode, expected in modes.items():
                subprocess.run(
                    [str(executable), str(codec), str(expected)],
                    env={**environment, "AMF_MOCK_MODE": mode},
                    check=True,
                    timeout=5,
                )
        for codec in [-1, 3]:
            subprocess.run(
                [str(executable), str(codec), "-1"],
                env=environment,
                check=True,
                timeout=5,
            )
        missing_library = directory / "absent-amf.so"
        subprocess.run(
            [str(executable), "0", "0"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(missing_library)},
            check=True,
            timeout=5,
        )
        invalid_library = directory / "invalid-amf.so"
        invalid_library.write_text("This fixture is deliberately not a shared library.\n")
        subprocess.run(
            [str(executable), "0", "-1"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(invalid_library)},
            check=True,
            timeout=5,
        )
        no_api_source = directory / "no-api.c"
        no_api_source.write_text("int serein_mock_no_api(void) { return 0; }\n")
        no_api_library = directory / "no-api-amf.so"
        subprocess.run(
            c_compiler + ["-std=c11"] + warnings + [
                "-fPIC", "-shared", str(no_api_source), "-o", str(no_api_library),
            ],
            check=True,
        )
        subprocess.run(
            [str(executable), "0", "-1"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(no_api_library)},
            check=True,
            timeout=5,
        )
        if (include / "vulkan/vulkan.h").is_file():
            scoped_library = directory / "scoped-amf.so"
            scoped_executable = directory / "scoped-query-test"
            gpu_object = directory / "gpu.o"
            subprocess.run(c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-DSEREIN_AMF_SCOPED_FIXTURE=1", "-fPIC", "-shared",
                str(fixture_dir / "video_query_amf_mock.c"), "-o", str(scoped_library),
            ], check=True)
            subprocess.run(c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-c", str(source_dir / "video_gpu.c"), "-o", str(gpu_object),
            ], check=True)
            subprocess.run(cpp_compiler + ["-std=c++11"] + warnings + sdk_flags + [
                "-I" + str(source_dir), "-DSEREIN_HAVE_VULKAN_GPU=1",
                str(source_dir / "video_query_amf.cpp"),
                "-DSEREIN_AMF_SCOPED_FIXTURE=1", str(fixture_dir / "video_query_amf_test.cpp"), str(gpu_object),
                "-Wl,--wrap=dlopen", "-Wl,--wrap=serein_video_vulkan_device", "-ldl",
                "-o", str(scoped_executable),
            ], check=True)
            subprocess.run([str(scoped_executable)],
                env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(scoped_library), "AMF_MOCK_MODE": "scoped"},
                timeout=5, check=True)
            print("AMF scoped query: identical models retain exact Vulkan handles; missing targets stay unknown; all objects released")
        print(
            "AMF query: 51 capability cases, invalid codecs, missing/bad runtime "
            "and missing ABI entry point passed; no encoder Init or submitted frames."
        )



def run_encoding(prefix):
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="serein-encode-fixture-") as directory:
        executable = Path(directory) / "video-encoding-test"
        subprocess.run([
            *shlex.split(os.environ.get("CC", "cc")), "-std=c11", "-Wall", "-Wextra", "-Werror",
            "-I" + str(prefix / "include"), "-I" + str(here.parents[3] / "crates/platform/src/video"),
            str(here / "video_encode_timing_test.c"), "-L" + str(prefix / "lib"),
            "-Wl,-rpath," + str(prefix / "lib"), "-lavcodec-serein", "-lavutil-serein",
            "-o", str(executable),
        ], check=True)
        subprocess.run([str(executable)], check=True)


    # Exercise the actual decoder copy path: odd visible HDR crops need rounded
    # chroma strides, not an even-dimension rejection or a truncated allocation.
    with tempfile.TemporaryDirectory(prefix="serein-decode-fixture-") as directory:
        root = Path(directory)
        source = root / "cropped.c"
        source.write_text(r'''
#include <assert.h>
#include "video_decode_ffmpeg.c"
static void audio_resampler_transitions(void) {
    SereinMedia *media = calloc(1, sizeof(*media));
    assert(media);
    media->audio = avcodec_alloc_context3(NULL);
    media->audio_frame = av_frame_alloc();
    media->format = avformat_alloc_context();
    assert(media->audio && media->audio_frame && media->format);
    AVStream *stream = avformat_new_stream(media->format, NULL);
    assert(stream);
    stream->time_base = (AVRational){1, 48000};
    media->audio_stream = stream->index;
    float *guard = calloc(2 * 65536 + 2, sizeof(*guard));
    assert(guard);
    guard[0] = guard[2 * 65536 + 1] = 17.0f;
    const struct { enum AVSampleFormat format; uint64_t layout; int rate; } cases[] = {
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_STEREO, 48000},
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_MONO, 48000},
        {AV_SAMPLE_FMT_FLT, AV_CH_LAYOUT_MONO, 48000},
        {AV_SAMPLE_FMT_S16, AV_CH_LAYOUT_STEREO, 48000},
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_2POINT1, 48000},
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_SURROUND, 48000},
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_7POINT1, 44100},
        {AV_SAMPLE_FMT_FLTP, AV_CH_LAYOUT_7POINT1, 48000},
    };
    for (size_t step = 0; step < sizeof(cases) / sizeof(cases[0]); ++step) {
        AVFrame *frame = media->audio_frame;
        frame->format = cases[step].format;
        frame->sample_rate = media->audio->sample_rate = cases[step].rate;
        frame->nb_samples = 1024;
        frame->best_effort_timestamp = (int64_t)step * 48000;
        assert(av_channel_layout_from_mask(&frame->ch_layout, cases[step].layout) == 0);
        assert(av_frame_get_buffer(frame, 0) == 0);
        assert(av_samples_set_silence(frame->extended_data, 0, frame->nb_samples,
            frame->ch_layout.nb_channels, frame->format) == 0);
        media->audio_pending = 1;
        size_t written = 0;
        double pts = 0;
        assert(serein_media_audio(media, guard + 1, 65536, &written, &pts) == 1);
        assert(written > 0 && written <= 65536 && isfinite(pts));
        assert(media->resample && media->resample_layout.u.mask == cases[step].layout);
        assert(media->resample_format == cases[step].format && media->resample_rate == cases[step].rate);
        assert(guard[0] == 17.0f && guard[2 * 65536 + 1] == 17.0f);
        for (size_t sample = 0; sample < written * 2; ++sample) assert(guard[sample + 1] == 0.0f);
        assert(media->audio_pending == 0 && frame->ch_layout.nb_channels == 0);
    }
    // Seeking/fallback/close use this same reset; owned layout state is retired too.
    media_reset_resampler(media);
    assert(!media->resample && media->resample_layout.nb_channels == 0);
    assert(media->resample_format == AV_SAMPLE_FMT_NONE && media->resample_rate == 0);
    media_reset_resampler(media);
    // Invalid audio metadata must fail before any resampler conversion/allocation.
    media->audio_frame->nb_samples = 1024;
    media->audio_frame->sample_rate = media->audio->sample_rate = 48000;
    media->audio_frame->format = AV_SAMPLE_FMT_NONE;
    av_channel_layout_default(&media->audio_frame->ch_layout, 2);
    media->audio_pending = 1;
    size_t written = 0;
    double pts = 0;
    assert(serein_media_audio(media, guard + 1, 65536, &written, &pts) == -2);
    assert(!media->resample && media->resample_layout.nb_channels == 0);
    free(guard);
    serein_media_close(media);
    puts("Attachment audio safely rebuilds for channel layout, sample format and rate changes");
}
int main(void) {
    audio_resampler_transitions();
    SereinDecoder *d = serein_decode_open(0, NULL, 0);
    assert(d);
    const enum AVPixelFormat formats[] = {AV_PIX_FMT_YUV420P10LE, AV_PIX_FMT_P010LE,
        AV_PIX_FMT_YUV420P, AV_PIX_FMT_NV12, AV_PIX_FMT_GRAY10LE, AV_PIX_FMT_GRAY8};
    for (size_t variant = 0; variant < sizeof(formats) / sizeof(formats[0]); ++variant)
    for (int full = 0; full < 2; ++full) {
    AVFrame *f = d->frame;
    av_frame_unref(f);
    f->format = formats[variant];
    f->width = 33; f->height = 17;
    f->color_trc = AVCOL_TRC_SMPTE2084;
    f->color_primaries = AVCOL_PRI_BT2020;
    f->colorspace = AVCOL_SPC_BT2020_NCL;
    f->color_range = full ? AVCOL_RANGE_JPEG : AVCOL_RANGE_MPEG;
    f->chroma_location = (enum AVChromaLocation)(variant + 1);
    assert(av_frame_get_buffer(f, 32) == 0);
    const AVPixFmtDescriptor *description = av_pix_fmt_desc_get(f->format);
    for (int component = 0; component < description->nb_components; ++component) {
        const AVComponentDescriptor *c = &description->comp[component];
        int w = component ? 17 : 33, h = component ? 9 : 17;
        unsigned sample = component ? 1u << (c->depth - 1) : full ? 0 : 16u << (c->depth - 8);
        unsigned word = sample << c->shift;
        for (int y = 0; y < h; ++y) {
            for (int x = 0; x < w; ++x) {
                uint8_t *at = f->data[c->plane] + y * f->linesize[c->plane] + x * c->step + c->offset;
                at[0] = (uint8_t)word;
                if (c->depth + c->shift > 8) at[1] = (uint8_t)(word >> 8);
            }
        }
    }
    d->pending = 1;
    SereinPicture picture;
    assert(serein_decode_receive(d, &picture, NULL, 0) == 1);
    assert(picture.width == 33 && picture.height == 17 && picture.format == 1);
    assert(picture.bytes == 2 * 33 * 17 + 4 * 17 * 9);
    assert(picture.chroma_location == variant + 1);
    uint8_t *guard = malloc(picture.bytes + 2);
    assert(guard); memset(guard, 0x5a, picture.bytes + 2);
    assert(serein_decode_receive(d, &picture, guard + 1, picture.bytes) == 1);
    assert(guard[0] == 0x5a && guard[picture.bytes + 1] == 0x5a);
    for (size_t at = 0; at < picture.bytes; at += 2) {
        uint16_t word = (uint16_t)(guard[at + 1] | guard[at + 2] << 8);
        unsigned expected = at < 2 * 33 * 17 ? (full ? 0 : 64 << 6) : 512 << 6;
        if (word != expected)
            fprintf(stderr, "crop offset=%zu word=%u\n", at, (unsigned)word);
        assert(word == expected);
    }
    free(guard);
    }
    serein_decode_close(d);
    puts("Odd HDR crops preserve exact P010 strides, black levels and output bounds");
    return 0;
}
''')
        bridge = here.parents[3] / "crates/platform/src/video"
        executable = root / "cropped-test"
        subprocess.run([
            *shlex.split(os.environ.get("CC", "cc")), "-std=c11", "-Wall", "-Wextra", "-Werror",
            "-I" + str(prefix / "include"), "-I" + str(bridge), str(source),
            str(bridge / "video_gpu.c"), "-L" + str(prefix / "lib"),
            "-Wl,-rpath," + str(prefix / "lib"), "-lavcodec-serein", "-lavutil-serein",
            "-lavformat-serein", "-lswscale-serein", "-lswresample-serein", "-lm", "-ldl",
            "-o", str(executable),
        ], check=True)
        subprocess.run([str(executable)], check=True)



def run_patch(archive):
    repository = Path(__file__).resolve().parents[4]
    spec = importlib.util.spec_from_file_location("ffmpeg_recipe", repository / "scripts/build-ffmpeg.py")
    recipe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(recipe)
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == recipe.SOURCES["ffmpeg"]["sha256"]
    with tempfile.TemporaryDirectory(prefix="serein-ffmpeg-patch-") as directory:
        root = Path(directory)
        modified = root / "modified"
        reproduced = root / "reproduced"
        # Only these upstream files are involved; no large build/capture fixture.
        paths = ["configure", "libavcodec/libavcodec.v", "libavutil/libavutil.v", "libavformat/libavformat.v", "libavutil/hwcontext_videotoolbox.h", "libavutil/hwcontext_videotoolbox.c", "libavcodec/videotoolbox.c",
                 "libswscale/libswscale.v", "libswresample/libswresample.v", "libavcodec/qsvenc.c",
                 "libavcodec/amfenc.c", "libavcodec/amfenc.h", "libavcodec/amfenc_hevc.c",
                 "libavcodec/amfenc_av1.c", "libavcodec/videotoolboxenc.c", "libavutil/hwcontext_vulkan.c", "libavcodec/libdav1d.c"]
        with tarfile.open(archive) as source:
            for path in paths:
                content = source.extractfile("ffmpeg-7.1.5/" + path).read()
                for tree in [modified, reproduced]:
                    (tree / path).parent.mkdir(parents=True, exist_ok=True)
                    (tree / path).write_bytes(content)
        patch = recipe.patch_ffmpeg(modified)
        subprocess.run(["patch", "--batch", "--fuzz=0", "-p1"], input=patch, text=True,
                       cwd=reproduced, stdout=subprocess.DEVNULL, check=True)
        original_paths = paths.copy()
        generated_paths = ["libavcodec/serein_qsv_feature_validation.h", "libavcodec/serein_amf_split_encoding.h"]
        paths.extend(generated_paths)
        for path in paths:
            assert (modified / path).read_bytes() == (reproduced / path).read_bytes(), path
        subprocess.run(["patch", "--batch", "--fuzz=0", "--reverse", "-p1"], input=patch, text=True,
                       cwd=reproduced, stdout=subprocess.DEVNULL, check=True)
        with tarfile.open(archive) as source:
            for path in original_paths:
                assert (reproduced / path).read_bytes() == source.extractfile("ffmpeg-7.1.5/" + path).read()
        for path in generated_paths:
            assert not (reproduced / path).exists()
        try:
            recipe.patch_ffmpeg(modified)
        except ValueError:
            pass
        else:
            raise AssertionError("Already-patched source must reject a second application")
    print("Pinned FFmpeg GPU patches reproduce byte-for-byte, reverse cleanly and reject duplicate application")



if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Offline native media regressions; no GPU/capture access.")
    parser.add_argument("--suite", choices=["queries", "encoding", "patch", "all"], default="queries")
    parser.add_argument("--prefix", type=Path)
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    if args.suite in ["queries", "encoding", "all"] and args.prefix is None:
        parser.error("--prefix is required for query/encoding fixtures")
    if args.suite in ["patch", "all"] and args.archive is None:
        parser.error("--archive is required for patch fixtures")
    if args.suite in ["queries", "all"]:
        run_queries(args.prefix.resolve())
    if args.suite in ["encoding", "all"]:
        run_encoding(args.prefix.resolve())
    if args.suite in ["patch", "all"]:
        run_patch(args.archive.resolve())
