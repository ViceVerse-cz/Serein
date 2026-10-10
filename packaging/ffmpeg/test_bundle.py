"""Synthetic library/source packaging and ELF coexistence checks; no media is opened."""

import importlib.util
import ctypes
import json
import hashlib
import io
import mmap
import os
import re
from pathlib import Path
import tempfile
import subprocess
import tarfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import bundle

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("ffmpeg_builder", REPO / "scripts/build-ffmpeg.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def native_codec(prefix, system, mode=ctypes.DEFAULT_MODE):
    directory = prefix / ("bin" if system == "Windows" else "lib")
    # Loading the absolute dependencies also resolves @rpath IDs when macOS
    # Python itself has no LC_RPATH for a developer's private native prefix.
    dependencies = [ctypes.CDLL(str(directory / name), mode=mode) for name in reversed(bundle.LIBRARIES[system][1:])]
    codec = ctypes.CDLL(str(directory / bundle.LIBRARIES[system][0]), mode=mode)
    codec._serein_dependencies = dependencies
    return codec


class BundleTest(unittest.TestCase):
    @staticmethod
    def darwin_prefix(root):
        prefix = root / "prefix"
        libraries = prefix / "lib"
        libraries.mkdir(parents=True)
        for name in bundle.LIBRARIES["Darwin"]:
            (libraries / name).write_bytes(("old " + name).encode())
        provenance = prefix / "share/serein-ffmpeg"
        (provenance / "source").mkdir(parents=True)
        (provenance / "build.json").write_text(json.dumps({
            "system": "Darwin", "nvenc": False, "amf": False, "qsv": False,
            "sources": {"ffmpeg": builder.SOURCES["ffmpeg"]},
        }))
        for name in ("configure.json", "build-ffmpeg.py", "serein-ffmpeg.patch", "COPYING.LGPLv2.1",
                     "OpenH264-LICENSE", "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2"):
            (provenance / name).write_bytes(b"synthetic source/notice")
        return prefix

    @unittest.skipUnless(os.name == "posix", "Darwin-style source aliases require POSIX symlinks")
    def test_darwin_restage_replaces_library_inode_and_preserves_open_mapping(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = self.darwin_prefix(root)
            stage = root / "dist"
            frameworks = stage / "Serein.app/Contents/Frameworks"
            with patch.object(bundle.platform, "system", return_value="Darwin"):
                bundle.bundle(stage, prefix)
                for name in bundle.LIBRARIES["Darwin"]:
                    target = frameworks / name
                    target.chmod(0o644)
                    inode = target.stat().st_ino
                    old = target.read_bytes()
                    source = prefix / "lib" / name
                    actual = source.with_suffix(source.suffix + ".actual")
                    source.rename(actual)
                    source.symlink_to(actual.name)
                    actual.write_bytes(("new " + name).encode())
                    with target.open("rb") as opened, mmap.mmap(opened.fileno(), 0, access=mmap.ACCESS_READ) as mapped:
                        bundle.bundle(stage, prefix)
                        self.assertNotEqual(target.stat().st_ino, inode)
                        self.assertEqual(opened.read(), old)
                        self.assertEqual(mapped[:], old)
                    self.assertFalse(target.is_symlink())
                    self.assertEqual(target.read_bytes(), actual.read_bytes())
                    self.assertEqual(target.stat().st_mode & 0o777, 0o644)
                    self.assertEqual({path.name for path in frameworks.iterdir()}, set(bundle.LIBRARIES["Darwin"]))

    def test_darwin_failed_staging_preserves_library_and_removes_temporary(self):
        for operation in ("copy", "replace"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                prefix = self.darwin_prefix(root)
                stage = root / "dist"
                frameworks = stage / "Serein.app/Contents/Frameworks"
                with patch.object(bundle.platform, "system", return_value="Darwin"):
                    bundle.bundle(stage, prefix)
                    target = frameworks / bundle.LIBRARIES["Darwin"][0]
                    old = target.read_bytes()
                    inode = target.stat().st_ino
                    def fail_copy(source, destination):
                        Path(destination).write_bytes(b"partial staging")
                        raise OSError("synthetic copy failure")
                    failure = patch.object(bundle.shutil, "copyfile", side_effect=fail_copy) if operation == "copy" else \
                        patch.object(bundle.os, "replace", side_effect=OSError("synthetic replace failure"))
                    with failure, self.assertRaisesRegex(OSError, "synthetic .* failure"):
                        bundle.bundle(stage, prefix)
                    self.assertEqual(target.stat().st_ino, inode)
                    self.assertEqual(target.read_bytes(), old)
                    self.assertEqual({path.name for path in frameworks.iterdir()}, set(bundle.LIBRARIES["Darwin"]))

    def test_matching_stamp_rejects_incomplete_prefix_without_native_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = root / "prefix"
            notices = prefix / "share/serein-ffmpeg"
            notices.mkdir(parents=True)
            backends = builder.encoder_backends("Linux", False)
            recipe = {"sources": builder.SOURCES, "system": "Linux", "architecture": "x86_64", **backends,
                      "encoders": sorted(builder.encoder_names(backends)),
                      "toolchain": builder.toolchain_options("Linux", False),
                      "recipe_sha256": hashlib.sha256(Path(builder.__file__).read_bytes()).hexdigest()}
            (notices / "build.json").write_text(json.dumps(recipe))
            args = SimpleNamespace(prefix=prefix, cache_dir=root / "cache", work_dir=root / "work", jobs=1, offline=True)
            with patch.object(builder.platform, "system", return_value="Linux"), \
                    patch.dict(os.environ, {"VSCMD_ARG_TGT_ARCH": "x86_64"}), \
                    patch.object(builder, "fetch") as fetch, patch.object(builder, "run") as run:
                with self.assertRaisesRegex(ValueError, "Incomplete FFmpeg prefix"):
                    builder.build(args)
                fetch.assert_not_called()
                run.assert_not_called()

    def test_cached_build_requires_platform_artifacts_and_source_payload(self):
        headers = ("libavcodec/avcodec.h", "libavutil/avutil.h", "libavutil/error.h", "libavutil/frame.h",
                   "libavutil/hwcontext.h", "libavutil/mem.h", "libavutil/opt.h")
        provenance = ("build-ffmpeg.py", "configure.json", "serein-ffmpeg.patch", "COPYING.LGPLv2.1", "OpenH264-LICENSE",
                      "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2",
                      "nv-codec-headers-README", "source/nv-codec-headers-12.2.72.0.tar.gz", "AMF-LICENSE",
                      "source/AMF-1.4.36-headers.tar", "oneVPL-LICENSE", "oneVPL-third-party-programs.txt",
                      "source/libvpl-2.14.0.tar.gz")
        for system in bundle.LIBRARIES:
            with self.subTest(system=system), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                prefix = root / "prefix"
                relative_files = ["include/" + name for name in headers]
                relative_files += ["lib/pkgconfig/libavcodec-serein.pc", "lib/pkgconfig/libavutil-serein.pc"]
                libraries = [str(Path("bin" if system == "Windows" else "lib") / name)
                             for name in bundle.LIBRARIES[system]]
                relative_files += libraries
                aliases = {"Linux": ["lib/libavcodec-serein.so", "lib/libavutil-serein.so"],
                           "Darwin": ["lib/libavcodec-serein.dylib", "lib/libavutil-serein.dylib"],
                           "Windows": ["lib/avcodec-serein.lib", "lib/avutil-serein.lib"]}[system]
                relative_files += aliases + ["share/serein-ffmpeg/" + name for name in provenance]
                backends = builder.encoder_backends(system, False)
                query_files = []
                if backends["nvenc"]:
                    query_files += ["include/ffnvcodec/nvEncodeAPI.h", "include/ffnvcodec/dynlink_cuda.h"]
                if backends["amf"]:
                    query_files += ["include/AMF/core/Factory.h", "include/AMF/components/ComponentCaps.h",
                                    "include/AMF/components/VideoEncoderVCE.h", "include/AMF/components/VideoEncoderHEVC.h",
                                    "include/AMF/components/VideoEncoderAV1.h"]
                if backends["qsv"]:
                    query_files += ["include/vpl/mfxdispatcher.h", "include/vpl/mfxstructures.h", "lib/pkgconfig/vpl.pc",
                                    "lib/vpl.lib" if system == "Windows" else "lib/libvpl.a"]
                vulkan_provenance = []
                if backends["amf"] and system == "Linux":
                    query_files += ["include/vulkan/vulkan.h", "include/vulkan/vulkan_core.h"]
                    vulkan_provenance = ["share/serein-ffmpeg/" + name for name in (
                        "Vulkan-Headers-LICENSE.md", "Vulkan-Headers-LICENSES/Apache-2.0.txt",
                        "Vulkan-Headers-LICENSES/MIT.txt", "source/Vulkan-Headers-1.3.290.tar.gz")]
                relative_files += query_files + vulkan_provenance
                if system == "Linux":
                    relative_files.append("share/serein-ffmpeg/serein-openh264.patch")
                for name in relative_files:
                    target = prefix / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(b"synthetic required artifact")
                recipe = {"sources": builder.SOURCES, "system": system, "architecture": "x86_64", **backends,
                          "encoders": sorted(builder.encoder_names(backends)),
                          "toolchain": builder.toolchain_options(system, False),
                          "recipe_sha256": hashlib.sha256(Path(builder.__file__).read_bytes()).hexdigest()}
                (prefix / "share/serein-ffmpeg/build.json").write_text(json.dumps(recipe))
                args = SimpleNamespace(prefix=prefix, cache_dir=root / "cache", work_dir=root / "work", jobs=1, offline=True)
                with patch.object(builder.platform, "system", return_value=system), \
                        patch.dict(os.environ, {"VSCMD_ARG_TGT_ARCH": "x86_64"}), \
                        patch.object(builder, "fetch") as fetch, patch.object(builder, "run") as run:
                    self.assertEqual(builder.build(args), prefix.resolve())
                    if system != "Windows":
                        for compiler in ("CC", "CXX"):
                            with patch.dict(os.environ, {compiler: "/different/compiler-wrapper"}):
                                with self.assertRaisesRegex(ValueError, "prefix contains another build"):
                                    builder.build(args)
                    for name in ["include/libavcodec/avcodec.h", libraries[0], aliases[0],
                                 "share/serein-ffmpeg/source/ffmpeg-7.1.5.tar.xz", *query_files, *vulkan_provenance]:
                        target = prefix / name
                        original = target.read_bytes()
                        target.unlink()
                        with self.assertRaisesRegex(ValueError, "Incomplete FFmpeg prefix"):
                            builder.build(args)
                        target.write_bytes(b"")
                        with self.assertRaisesRegex(ValueError, "Incomplete FFmpeg prefix"):
                            builder.build(args)
                        target.write_bytes(original)
                    if system != "Windows" and os.name == "posix":
                        alias = prefix / aliases[0]
                        alias.unlink()
                        alias.symlink_to(Path(libraries[0]).name)
                        self.assertEqual(builder.build(args), prefix.resolve())
                        (prefix / libraries[0]).unlink()
                        with self.assertRaisesRegex(ValueError, "Incomplete FFmpeg prefix"):
                            builder.build(args)
                    fetch.assert_not_called()
                    run.assert_not_called()

    def test_openh264_subset_preserves_build_scripts_and_android_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = root / "raw.tar.gz"
            with tarfile.open(raw, "w:gz") as archive:
                for name, mode, content in (("codec/common/generate_version.sh", 0o775, b"#!/bin/sh\n"),
                                            ("LICENSE", 0o664, b"BSD license"),
                                            ("codec/build/android/dec/res/layout/main.xml", 0o664, b"Android resource"),
                                            ("res/test.yuv", 0o664, b"test media omitted")):
                    entry = tarfile.TarInfo("openh264-2.6.0/" + name)
                    entry.size, entry.mode, entry.mtime, entry.uid = len(content), mode, 123456, 1000
                    archive.addfile(entry, io.BytesIO(content))
            first, second = root / "first.tar.bz2", root / "second.tar.bz2"
            builder.write_openh264_subset(raw, first)
            builder.write_openh264_subset(raw, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with tarfile.open(first) as archive:
                self.assertNotIn("openh264-2.6.0/res/test.yuv", archive.getnames())
                retained = "openh264-2.6.0/codec/build/android/dec/res/layout/main.xml"
                self.assertEqual(archive.extractfile(retained).read(), b"Android resource")
                script = archive.getmember("openh264-2.6.0/codec/common/generate_version.sh")
                self.assertEqual(script.mode, 0o775)
                self.assertEqual((script.mtime, script.uid, script.gid), (0, 0, 0))

    @unittest.skipUnless(os.name == "posix" and bundle.platform.system() == "Linux",
                         "ELF symbol isolation requires Linux")
    def test_private_openh264_api_and_internal_symbols_coexist_with_host_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "codec/api/wels").mkdir(parents=True)
            (root / "build").mkdir()
            (root / "codec/api/wels/codec_api.h").write_text(
                "#ifndef WELS_VIDEO_CODEC_SVC_API_H__\n#define WELS_VIDEO_CODEC_SVC_API_H__\n#endif\n")
            (root / "build/platform-gnu-chain.mk").write_text(
                "SHLDFLAGS = -Wl,-soname,$(LIBPREFIX)$(PROJECT_NAME).$(SHAREDLIBSUFFIXMAJORVER)\n"
                "LDFLAGS += -lpthread\n")
            source_patch = builder.patch_openh264(root)
            (root / "private.c").write_text('#include "codec/api/wels/codec_api.h"\n'
                                           "int codec_internal(void) { return 26; }\n" + "".join(
                                               f"int {name}(void) {{ return codec_internal(); }}\n"
                                               for name in builder.OPENH264_APIS))
            (root / "host.c").write_text("int codec_internal(void) { return 24; }\n" + "".join(
                f"int {name}(void) {{ return codec_internal(); }}\n" for name in builder.OPENH264_APIS))
            subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-soname,libopenh264.so.8",
                            str(root / "host.c"), "-o", str(root / "libopenh264.so.8")], check=True)
            (root / "Makefile").write_text(
                "SRC_PATH=./\nLIBPREFIX=lib\nPROJECT_NAME=openh264\nSHAREDLIBSUFFIXMAJORVER=so.8\n"
                "include build/platform-gnu-chain.mk\nall:\n"
                "\t$(CC) -shared -fPIC private.c -o libopenh264-serein.so.8 $(LDFLAGS) $(SHLDFLAGS)\n")
            subprocess.run(["make", "--no-print-directory", "all"], cwd=root, check=True)
            (root / "plugin.c").write_text("int WelsCreateSVCEncoder(void);\n"
                                           "int host_codec_version(void) { return WelsCreateSVCEncoder(); }\n")
            subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-rpath,$ORIGIN", str(root / "plugin.c"),
                            "-L" + str(root), "-l:libopenh264.so.8", "-o", str(root / "plugin.so")], check=True)
            # Keep global fixtures out of this runner and later native checks.
            subprocess.run([os.sys.executable, "-c",
                            "import ctypes,sys; from pathlib import Path; r=Path(sys.argv[1]); "
                            "host=ctypes.CDLL(str(r/'libopenh264.so.8'), mode=ctypes.RTLD_GLOBAL); "
                            "private=ctypes.CDLL(str(r/'libopenh264-serein.so.8'), mode=ctypes.RTLD_GLOBAL); "
                            "plugin=ctypes.CDLL(str(r/'plugin.so')); "
                            "assert plugin.host_codec_version()==24; "
                            "assert all(getattr(private, 'serein_'+n)()==26 for n in sys.argv[2:]); "
                            "exports=__import__('subprocess').check_output(['nm','-D','--defined-only',"
                            "str(r/'libopenh264-serein.so.8')],text=True); "
                            "assert 'codec_internal' not in exports; "
                            "assert all((' '+n+'@') not in exports and (' '+n+'\\n') not in exports for n in sys.argv[2:])",
                            str(root), *builder.OPENH264_APIS], check=True)
            # Private first must not satisfy the host plugin's dependency even
            # if both OpenH264 builds have upstream ABI major eight.
            subprocess.run([os.sys.executable, "-c",
                            "import ctypes,sys; from pathlib import Path; r=Path(sys.argv[1]); "
                            "private=ctypes.CDLL(str(r/'libopenh264-serein.so.8'), mode=ctypes.RTLD_GLOBAL); "
                            "plugin=ctypes.CDLL(str(r/'plugin.so')); "
                            "assert plugin.host_codec_version()==24; "
                            "assert private.serein_WelsCreateSVCEncoder()==26",
                            str(root)], check=True)
            self.assertIn("serein-openh264.map", source_patch)
            self.assertIn("LDFLAGS += -lpthread", (root / "build/platform-gnu-chain.mk").read_text())
            with self.assertRaisesRegex(ValueError, "unpatched OpenH264"):
                builder.patch_openh264(root)

    def test_hardware_build_options_keep_vaapi_encoder_disabled(self):
        for system in ("Linux", "Windows", "Darwin"):
            for arm64 in (False, True):
                with self.subTest(system=system, arm64=arm64):
                    backends = builder.encoder_backends(system, arm64)
                    options = builder.hardware_options(system, backends)
                    self.assertIn("--disable-encoder=h264_vaapi,hevc_vaapi,av1_vaapi", options)
                    self.assertIn("--disable-mediafoundation", options)
                    enabled = {name for option in options if option.startswith("--enable-encoder=")
                               for name in option.split("=", 1)[1].split(",")}
                    for codec in ("h264", "hevc", "av1"):
                        self.assertEqual(f"{codec}_amf" in enabled, system == "Linux" or (system == "Windows" and not arm64))
                        self.assertEqual(f"{codec}_qsv" in enabled, system in {"Linux", "Windows"} and not arm64)
                        self.assertEqual(f"{codec}_nvenc" in enabled, system in {"Linux", "Windows"} and not (system == "Windows" and arm64))
                    self.assertEqual("hevc_videotoolbox" in enabled, system == "Darwin")
                    self.assertNotIn("av1_videotoolbox", enabled)
                    self.assertEqual("--enable-vaapi" in options, system == "Linux" and not arm64)
                    self.assertEqual("--enable-d3d11va" in options, system == "Windows" and not arm64)
                    self.assertNotIn("--enable-libmfx", options)

    @unittest.skipUnless(os.environ.get("FFMPEG_DIR") and bundle.platform.system() in bundle.LIBRARIES,
                         "Requires a native FFmpeg build; no media is opened")
    def test_native_codec_registry_is_exact_and_contains_no_decoders(self):
        prefix = Path(os.environ["FFMPEG_DIR"])
        system = bundle.platform.system()
        codec = native_codec(prefix, system)
        codec.av_codec_iterate.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        codec.av_codec_iterate.restype = ctypes.c_void_p
        codec.av_codec_is_encoder.argtypes = [ctypes.c_void_p]
        codec.av_codec_is_encoder.restype = ctypes.c_int
        state = ctypes.c_void_p()
        registered = set()
        while value := codec.av_codec_iterate(ctypes.byref(state)):
            self.assertTrue(codec.av_codec_is_encoder(value))
            # AVCodec's first member is const char *name in pinned FFmpeg 7.
            registered.add(ctypes.cast(value, ctypes.POINTER(ctypes.c_char_p)).contents.value.decode())
        recipe = json.loads((prefix / "share/serein-ffmpeg/build.json").read_text())
        self.assertEqual(registered, set(recipe["encoders"]))
        self.assertEqual(registered, builder.encoder_names({name: recipe[name] for name in ("nvenc", "amf", "qsv", "videotoolbox")}))

    def test_amf_offline_rebuild_uses_shipped_headers_without_sdk(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(builder.urllib.request, "urlopen") as download:
            cache = Path(directory)
            source = dict(builder.SOURCES["amf-headers"])
            source["sha256"] = hashlib.sha256(b"shipped verified headers").hexdigest()
            archive = cache / source["file"]
            archive.write_bytes(b"shipped verified headers")
            with patch.dict(builder.SOURCES, {"amf-headers": source}):
                self.assertEqual(builder.amf_headers(cache, offline=True), archive)
            self.assertFalse((cache / builder.SOURCES["amf-sdk"]["file"]).exists())
            download.assert_not_called()

    def test_qsv_allocation_patch_fails_on_unexpected_upstream_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "libavcodec").mkdir()
            (root / "libavutil").mkdir()
            (root / "libavcodec/libavcodec.v").write_text("LIBAVCODEC_MAJOR { local: *; };\n")
            (root / "libavutil/libavutil.v").write_text("LIBAVUTIL_MAJOR { local: *; };\n")
            # The complete recipe patches every platform's source even when
            # only the QSV allocation change is being exercised here.
            (root / "libavcodec/qsvenc.c").write_text(
                '#include "qsvenc.h"\n'
                "ret = av_new_packet(&pkt.pkt, q->packet_size);\n"
                "    ret = MFXVideoENCODE_QueryIOSurf(q->session, &q->param, &q->req);\n"
                "    q->packet_size = q->param.mfx.BufferSizeInKB * q->param.mfx.BRCParamMultiplier * 1000;\n"
                "    dump_video_av1_param(avctx, q, ext_buffers);\n"
                "    if (!extradata.SPSBufSize || (need_pps && !extradata.PPSBufSize)\n")
            (root / "libavcodec/videotoolboxenc.c").write_text(
                "    int allow_sw;\n"
                "#define COMMON_OPTIONS \\\n"
                "    // low-latency mode: eliminate frame reordering, follow a one-in-one-out encoding mode\n")
            (root / "libavcodec/amfenc.c").write_text(
                '#include "libavutil/hwcontext.h"\n'
                "#if CONFIG_D3D11VA\nstatic int amf_init_from_d3d11_device\n"
                "        switch (device_ctx->type) {\n")
            (root / "libavcodec/amfenc.h").write_text("    int                 usage;\n")
            for codec in ["hevc", "av1"]:
                (root / f"libavcodec/amfenc_{codec}.c").write_text(
                    '#include "amfenc.h"\n'
                    "static const AVOption options[] = {\n"
                    "    // init encoder\n")
            (root / "libavutil/hwcontext_vulkan.c").write_text(
                "        dev_select.drm_major = major(drm_node_info.st_dev);\n"
                "        dev_select.drm_minor = minor(drm_node_info.st_dev);\n"
                "    if (select->has_uuid) {\n")
            (root / "configure").write_text('hevc_qsv_encoder_select="hevcparse qsvenc"\n')
            source_patch = builder.patch_ffmpeg(root)
            self.assertIn("+ret = ff_get_encode_buffer(avctx, &pkt.pkt, q->packet_size, 0);", source_patch)
            self.assertEqual((root / "libavcodec/serein_qsv_feature_validation.h").read_text(),
                             builder.QSV_FEATURE_VALIDATION)
            self.assertEqual((root / "libavcodec/serein_amf_split_encoding.h").read_text(),
                             builder.AMF_SPLIT_ENCODING)
            with self.assertRaisesRegex(ValueError, "exactly one"):
                builder.patch_ffmpeg(root)

    def test_msvc_arm64_does_not_require_uninstalled_assembler_preprocessor(self):
        self.assertIn("--arch=aarch64", builder.toolchain_options("Windows", True))
        self.assertIn("--disable-asm", builder.toolchain_options("Windows", True))
        self.assertNotIn("--disable-asm", builder.toolchain_options("Windows", False))
        self.assertEqual(builder.toolchain_options("Linux", True, {}), ["--cc=cc", "--cxx=c++"])
        self.assertEqual(builder.toolchain_options("Darwin", True, {}), ["--cc=cc", "--cxx=c++"])

    def test_unix_build_propagates_compiler_wrappers_to_all_dependencies(self):
        class ConfigureReached(Exception):
            pass

        for system in ("Linux", "Darwin"):
            with self.subTest(system=system), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                args = SimpleNamespace(prefix=root / "prefix", cache_dir=root / "cache",
                                       work_dir=root / "work", jobs=1, offline=True)
                compilers = {"CC": "/nix/store/compiler/bin/cc", "CXX": "/nix/store/compiler/bin/c++"}

                def unpack(archive, destination):
                    destination.mkdir(parents=True)
                    (destination / "AMF").mkdir()
                    if destination.name == "vulkan-headers":
                        (destination / "include/vulkan").mkdir(parents=True)
                        (destination / "include/vulkan/vulkan.h").write_text("synthetic Vulkan header")
                    return destination

                def run(*arguments, **kwargs):
                    if arguments[0] == "make" and "install-shared" in arguments and system == "Linux":
                        library = args.prefix / "lib/libopenh264.so.8"
                        library.parent.mkdir(parents=True)
                        library.write_bytes(b"synthetic shared library")
                    if arguments[:2] == ("bash", "configure"):
                        raise ConfigureReached()

                with patch.object(builder.platform, "system", return_value=system), \
                        patch.dict(os.environ, {**compilers, "VSCMD_ARG_TGT_ARCH": "x86_64"}), \
                        patch.object(builder, "posix", side_effect=lambda path: str(path)), \
                        patch.object(Path, "symlink_to"), \
                        patch.object(builder, "fetch", return_value=root / "source.tar"), \
                        patch.object(builder, "openh264_source", return_value=root / "openh264.tar"), \
                        patch.object(builder, "amf_headers", return_value=root / "amf.tar"), \
                        patch.object(builder, "unpack", side_effect=unpack), \
                        patch.object(builder, "patch_ffmpeg", return_value=""), \
                        patch.object(builder, "patch_openh264", return_value=""), \
                        patch.object(builder, "run", side_effect=run) as command:
                    with self.assertRaises(ConfigureReached):
                        builder.build(args)
                calls = command.call_args_list
                self.assertIn("CC=" + compilers["CC"], calls[0].args)
                self.assertIn("CXX=" + compilers["CXX"], calls[0].args)
                self.assertIn("--cc=" + compilers["CC"], calls[-1].args)
                self.assertIn("--cxx=" + compilers["CXX"], calls[-1].args)
                for call in calls:
                    for name, compiler in compilers.items():
                        self.assertEqual(call.kwargs["env"][name], compiler)
                if system == "Linux":
                    self.assertTrue(any(call.args[:2] == ("cmake", "-S") for call in calls))

    def test_environment_compilers_do_not_override_explicit_msvc_toolchain(self):
        compilers = {"CC": "clang", "CXX": "clang++"}
        self.assertEqual(builder.toolchain_options("Windows", False, compilers),
                         ["--toolchain=msvc", "--target-os=win32", "--arch=x86_64"])

    def test_windows_configure_selects_the_msys_shell_despite_system32_bash(self):
        class ConfigureReached(Exception):
            pass

        for architecture in ("x64", "arm64"):
            with self.subTest(architecture=architecture), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                msys_shell = root / "MSYS2 with spaces/usr/bin/bash.exe"
                msys_shell.parent.mkdir(parents=True)
                msys_shell.write_bytes(b"synthetic MSYS2 executable")
                windows_shell = root / "Windows/System32/bash.exe"
                windows_shell.parent.mkdir(parents=True)
                windows_shell.write_bytes(b"synthetic WSL executable")
                args = SimpleNamespace(prefix=root / "prefix", cache_dir=root / "cache",
                                       work_dir=root / "work", jobs=1, offline=True)

                def unpack(archive, destination):
                    destination.mkdir(parents=True)
                    (destination / "AMF").mkdir()
                    return destination

                def run(*arguments, **kwargs):
                    if arguments[0] == "make" and "install-shared" in arguments:
                        library = args.prefix / "lib/openh264_dll.lib"
                        library.parent.mkdir(parents=True)
                        library.write_bytes(b"synthetic import library")
                    elif arguments[:2] == ("cmake", "--install"):
                        pc = args.prefix / "lib/pkgconfig/vpl.pc"
                        pc.parent.mkdir(parents=True)
                        pc.write_text("Libs: -lvpl\n")
                    elif len(arguments) > 1 and arguments[1] == "configure":
                        raise ConfigureReached()

                environment = {"VSCMD_ARG_TGT_ARCH": architecture,
                               "PATH": str(windows_shell.parent) + os.pathsep + str(msys_shell.parent)}
                with patch.object(builder.platform, "system", return_value="Windows"), \
                        patch.dict(os.environ, environment), \
                        patch.object(builder, "posix", side_effect=lambda path: str(path)), \
                        patch.object(builder, "fetch", return_value=root / "source.tar"), \
                        patch.object(builder, "openh264_source", return_value=root / "openh264.tar"), \
                        patch.object(builder, "amf_headers", return_value=root / "amf.tar"), \
                        patch.object(builder, "unpack", side_effect=unpack), \
                        patch.object(builder, "patch_ffmpeg", return_value=""), \
                        patch.object(builder.subprocess, "check_output", return_value=str(msys_shell) + "\n") as resolve, \
                        patch.object(builder, "run", side_effect=run) as command:
                    with self.assertRaises(ConfigureReached):
                        builder.build(args)
                self.assertEqual(command.call_args_list[-1].args[:2], (str(msys_shell), "configure"))
                resolve.assert_called_once_with(["cygpath", "-w", "/usr/bin/bash.exe"], text=True,
                                                env=resolve.call_args.kwargs["env"])
                self.assertEqual(resolve.call_args.kwargs["env"]["PATH"], environment["PATH"])
                # The dispatcher archive and FFmpeg must both use the static
                # MSVC runtime, including on the first CMake configuration.
                if architecture == "x64":
                    vpl = next(call.args for call in command.call_args_list if call.args[:2] == ("cmake", "-S"))
                    self.assertIn("-DUSE_MSVC_STATIC_RUNTIME=ON", vpl)
                    self.assertIn("-DCMAKE_POLICY_DEFAULT_CMP0091=NEW", vpl)
                    self.assertTrue(any(arg.endswith(" -MT") for arg in command.call_args_list[-1].args))

    def test_windows_configure_rejects_unavailable_msys_shell_without_path_fallback(self):
        environment = {"PATH": "synthetic System32 before MSYS2"}
        for error in (FileNotFoundError("cygpath"), subprocess.CalledProcessError(1, "cygpath")):
            with self.subTest(error=type(error).__name__), \
                    patch.object(builder.subprocess, "check_output", side_effect=error):
                with self.assertRaisesRegex(ValueError, "requires MSYS2 cygpath and Bash"):
                    builder.configure_shell("Windows", environment)
        with tempfile.TemporaryDirectory() as directory:
            for result in ("", "relative/bash.exe", str(Path(directory) / "missing/bash.exe")):
                with self.subTest(result=result), \
                        patch.object(builder.subprocess, "check_output", return_value=result):
                    with self.assertRaisesRegex(ValueError, "existing absolute Bash"):
                        builder.configure_shell("Windows", environment)

    def test_unix_configure_keeps_the_existing_shell_lookup(self):
        with patch.object(builder.subprocess, "check_output") as resolve:
            for system in ("Linux", "Darwin"):
                self.assertEqual(builder.configure_shell(system, {}), "bash")
            resolve.assert_not_called()

    @unittest.skipUnless(os.environ.get("FFMPEG_DIR") and bundle.platform.system() == "Windows",
                         "Requires the native MSVC FFmpeg build and dumpbin")
    def test_windows_codec_dlls_do_not_require_dispatcher_or_cpp_runtime_dlls(self):
        prefix = Path(os.environ["FFMPEG_DIR"])
        for name in bundle.LIBRARIES["Windows"]:
            with self.subTest(library=name):
                dependencies = subprocess.check_output(["dumpbin", "/dependents", str(prefix / "bin" / name)], text=True)
                imports = {name.lower() for name in re.findall(r"\b[\w.-]+\.dll\b", dependencies, flags=re.I)}
                self.assertFalse(any(name.startswith(("libvpl", "vpl", "msvcp140", "vcruntime140")) for name in imports),
                                 f"Codec must use static dispatcher/CRT, got {sorted(imports)}")

    @unittest.skipUnless(os.name == "posix" and os.environ.get("FFMPEG_DIR") and bundle.platform.system() == "Linux",
                         "Requires the native Linux FFmpeg build; no media is opened")
    def test_host_decoder_plugin_does_not_bind_to_encoder_only_ffmpeg(self):
        prefix = Path(os.environ["FFMPEG_DIR"])
        codec = native_codec(prefix, "Linux", mode=ctypes.RTLD_GLOBAL)
        codec.avcodec_find_encoder_by_name.argtypes = [ctypes.c_char_p]
        codec.avcodec_find_encoder_by_name.restype = ctypes.c_void_p
        codec.avcodec_find_decoder_by_name.argtypes = [ctypes.c_char_p]
        codec.avcodec_find_decoder_by_name.restype = ctypes.c_void_p
        self.assertTrue(codec.avcodec_find_encoder_by_name(b"libopenh264"))
        for name in ("h264_vaapi", "hevc_vaapi", "av1_vaapi"):
            self.assertFalse(codec.avcodec_find_encoder_by_name(name.encode()))
        self.assertFalse(codec.avcodec_find_decoder_by_name(b"h264"))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "host.c").write_text('void *avcodec_find_decoder_by_name(const char *name) { static int codec; return &codec; }\n')
            (root / "host.v").write_text("LIBAVCODEC_61 { global: avcodec_*; local: *; };\n")
            (root / "plugin.c").write_text('void *avcodec_find_decoder_by_name(const char *);\n'
                                           'int incoming_decoder_present(void) { return avcodec_find_decoder_by_name("h264") != 0; }\n')
            subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-soname,libavcodec.so.61",
                            "-Wl,--version-script=" + str(root / "host.v"), "-o", str(root / "libavcodec.so.61"),
                            str(root / "host.c")], check=True)
            subprocess.run(["cc", "-shared", "-fPIC", "-Wl,-rpath,$ORIGIN", "-o", str(root / "incoming.so"),
                            str(root / "plugin.c"), "-L" + str(root), "-l:libavcodec.so.61"], check=True)
            plugin = ctypes.CDLL(str(root / "incoming.so"))
            plugin.incoming_decoder_present.restype = ctypes.c_int
            self.assertEqual(plugin.incoming_decoder_present(), 1,
                             "Host decoder plugin must use host FFmpeg, even when Serein's encoders are loaded globally")

    def test_relocatable_shared_libraries_and_exact_source_allowlist(self):
        for system in bundle.LIBRARIES:
            with self.subTest(system=system), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                prefix = root / "prefix"
                libs = prefix / ("bin" if system == "Windows" else "lib")
                libs.mkdir(parents=True)
                for name in bundle.LIBRARIES[system]:
                    (libs / name).write_bytes(name.encode())
                (libs / "unrelated-private-log").write_text("must not ship")
                notices = prefix / "share/serein-ffmpeg"
                (notices / "source").mkdir(parents=True)
                nvenc = system != "Darwin"
                (notices / "build.json").write_text(json.dumps({"system": system, "nvenc": nvenc, "amf": nvenc, "qsv": nvenc,
                    "sources": {"ffmpeg": builder.SOURCES["ffmpeg"]}}))
                for name in ("configure.json", "build-ffmpeg.py", "serein-ffmpeg.patch", "COPYING.LGPLv2.1", "OpenH264-LICENSE",
                             "source/ffmpeg-7.1.5.tar.xz", "source/openh264-2.6.0-source.tar.bz2", "nv-codec-headers-README",
                             "source/nv-codec-headers-12.2.72.0.tar.gz", "AMF-LICENSE", "source/AMF-1.4.36-headers.tar",
                             "oneVPL-LICENSE", "oneVPL-third-party-programs.txt", "source/libvpl-2.14.0.tar.gz"):
                    (notices / name).write_text("synthetic source/notice")
                if system == "Linux":
                    (notices / "serein-openh264.patch").write_text("synthetic private ABI patch")
                vulkan_files = ("Vulkan-Headers-LICENSE.md", "Vulkan-Headers-LICENSES/Apache-2.0.txt",
                                "Vulkan-Headers-LICENSES/MIT.txt", "source/Vulkan-Headers-1.3.290.tar.gz")
                for name in vulkan_files:
                    path = notices / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("synthetic Vulkan source/notice")
                (notices / "private.log").write_text("must not ship")
                stage = root / "dist"
                with patch.object(bundle.platform, "system", return_value=system):
                    bundle.bundle(stage, prefix)
                names = {p.name for p in stage.rglob("*") if p.is_file()}
                self.assertTrue(set(bundle.LIBRARIES[system]).issubset(names))
                self.assertIn("ffmpeg-7.1.5.tar.xz", names)
                self.assertEqual("serein-openh264.patch" in names, system == "Linux")
                self.assertNotIn("private.log", names)
                self.assertNotIn("unrelated-private-log", names)
                self.assertEqual("nv-codec-headers-12.2.72.0.tar.gz" in names, nvenc)
                self.assertEqual("AMF-1.4.36-headers.tar" in names, nvenc)
                self.assertNotIn("AMF-1.4.36.tar.gz", names)
                self.assertEqual("libvpl-2.14.0.tar.gz" in names, nvenc)
                source = stage / ("Serein.app/Contents/Resources/ffmpeg-source" if system == "Darwin" else "ffmpeg-source")
                for name in vulkan_files:
                    self.assertEqual((source / name).is_file(), system == "Linux", name)

    def test_source_checksum_mismatch_fails_without_using_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            source = builder.SOURCES["ffmpeg"]
            (cache / source["file"]).write_bytes(b"corrupt source")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                builder.fetch(source, cache, offline=True)

    def test_offline_missing_source_never_attempts_download(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(builder.urllib.request, "urlopen") as download:
            with self.assertRaisesRegex(ValueError, "Offline source missing"):
                builder.fetch(builder.SOURCES["ffmpeg"], Path(directory), offline=True)
            download.assert_not_called()

    def test_bundle_requires_build_prefix(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(bundle.platform, "system", return_value="Linux"):
            with self.assertRaisesRegex(ValueError, "FFMPEG_DIR"):
                bundle.bundle(Path("unused"))


if __name__ == "__main__":
    unittest.main()
