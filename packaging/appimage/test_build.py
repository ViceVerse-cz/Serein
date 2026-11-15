"""AppImage packaging and updater environment regression checks; never launches Serein."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import build as appimage_build

WITH_TOOLS = False


def make_minimal_appdir(root):
    appdir = root / "Serein.AppDir"
    (appdir / "bin").mkdir(parents=True)
    (appdir / "shared/bin").mkdir(parents=True)
    (appdir / "lib").mkdir(parents=True)
    os.symlink("../lib", appdir / "shared/lib")
    for rel in ("AppRun", "AppRun.sh", "sharun", "dash", "bin/serein", "shared/bin/serein"):
        path = appdir / rel
        path.write_bytes(b"\x7fELFsynthetic")
        path.chmod(0o755)
    return appdir


class AppImagePackagingTest(unittest.TestCase):
    def test_update_metadata_validation(self):
        name, info, url = appimage_build.update_metadata("1.2.3-nightly.1", "v1.2.3-nightly.1")
        self.assertEqual(name, "serein-v1.2.3-nightly.1-Linux-X64.AppImage")
        self.assertIn("|latest-pre|", info)
        self.assertIn("/v1.2.3-nightly.1/", url)

        name_prod, info_prod, _ = appimage_build.update_metadata("1.2.3", "")
        self.assertEqual(name_prod, "serein-1.2.3-Linux-X64.AppImage")
        self.assertIn("|latest|", info_prod)

        with self.assertRaisesRegex(ValueError, "semantic application version"):
            appimage_build.update_metadata("1.2.3\nbad", "")
        with self.assertRaisesRegex(ValueError, "release tag"):
            appimage_build.update_metadata("1.2.3", "v1.2.4")

    def test_verify_pinned_tools_rejects_missing_and_tampered_files(self):
        with tempfile.TemporaryDirectory(prefix="serein-appimage-tools-") as directory:
            tools = Path(directory)
            with self.assertRaisesRegex(ValueError, "install-tools.sh"):
                appimage_build.verify_pinned_tools(tools)

            for relative in appimage_build.PINNED_TOOLS:
                target = tools / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(b"wrong contents")
                target.chmod(0o755)
            with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
                appimage_build.verify_pinned_tools(tools)

    def test_gstreamer_pruning_and_elf_closure_with_real_patchelf(self):
        if not shutil.which("patchelf") or not Path("/bin/true").is_file():
            self.skipTest("Requires patchelf and /bin/true")
        with tempfile.TemporaryDirectory(prefix="serein-appimage-gst-") as directory:
            root = Path(directory)
            appdir = make_minimal_appdir(root)
            shutil.copyfile("/bin/true", appdir / "shared/bin/serein")
            (appdir / "shared/bin/serein").chmod(0o755)

            needed = appimage_build.elf_needed(appdir / "shared/bin/serein")
            for soname in needed:
                lib = appdir / "lib" / soname
                lib.write_bytes(b"placeholder")
                lib.chmod(0o644)

            gst_dir = appdir / "lib/gstreamer-1.0"
            gst_dir.mkdir(parents=True)
            for core in appimage_build.CORE_GSTREAMER_PLUGINS:
                shutil.copyfile("/bin/true", gst_dir / core)
                (gst_dir / core).chmod(0o755)

            broken_plugin = gst_dir / "libgstbrokenoptional.so"
            shutil.copyfile("/bin/true", broken_plugin)
            broken_plugin.chmod(0o755)
            subprocess.run(
                ["patchelf", "--add-needed", "libmissingoptional.so.99", str(broken_plugin)],
                check=True,
            )
            os.symlink("libnonexistentplugin.so", gst_dir / "libdangling.so")

            appimage_build.prune_broken_gstreamer_plugins(
                appdir, require_gstreamer=True, require_pipewire=False
            )
            self.assertFalse(broken_plugin.exists())
            self.assertFalse((gst_dir / "libdangling.so").is_symlink())
            appimage_build.verify_elf_closure(appdir)

            with self.assertRaisesRegex(ValueError, "libgstpipewire.so"):
                appimage_build.prune_broken_gstreamer_plugins(
                    appdir, require_gstreamer=True, require_pipewire=True
                )

            (gst_dir / "libgstpulseaudio.so").unlink()
            with self.assertRaisesRegex(ValueError, "libgstpulseaudio.so"):
                appimage_build.prune_broken_gstreamer_plugins(
                    appdir, require_gstreamer=True, require_pipewire=False
                )

            # An unresolved non-GStreamer library dependency must still fail closure verification.
            broken_helper = appdir / "lib/librequiredhelper.so.1"
            shutil.copyfile("/bin/true", broken_helper)
            subprocess.run(
                ["patchelf", "--add-needed", "libmissingcritical.so.0", str(broken_helper)],
                check=True,
            )
            with self.assertRaisesRegex(ValueError, "libmissingcritical.so.0"):
                appimage_build.verify_elf_closure(appdir)

    def test_appdir_invariants_and_extracted_tree_guards(self):
        with tempfile.TemporaryDirectory(prefix="serein-appimage-guards-") as directory:
            root = Path(directory)
            appdir = make_minimal_appdir(root)
            appimage_build.verify_appdir_invariants(appdir)

            (appdir / ".env").write_text("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1\n")
            with self.assertRaisesRegex(ValueError, "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS"):
                appimage_build.verify_appdir_invariants(appdir)

            (appdir / ".env").write_text("GSK_RENDERER=cairo\n")
            with self.assertRaisesRegex(ValueError, "GSK_RENDERER=cairo"):
                appimage_build.verify_appdir_invariants(appdir)
            (appdir / ".env").unlink()

            leaked = appdir / "share/vulkan/icd.d/radeon_icd.x86_64.json"
            leaked.parent.mkdir(parents=True)
            leaked.write_text("{}")
            with self.assertRaisesRegex(ValueError, "leaked"):
                appimage_build.verify_appdir_invariants(appdir)
            leaked.unlink()

            (appdir / "usr/bin").mkdir(parents=True)
            (appdir / "usr/bin/serein").write_bytes(b"dup")
            with self.assertRaisesRegex(ValueError, "usr/bin/serein"):
                appimage_build.verify_appdir_invariants(appdir)
            (appdir / "usr/bin/serein").unlink()

            extracted = root / "extracted"
            shutil.copytree(appdir, extracted, symlinks=True)
            appimage_build.verify_extracted_tree(appdir, extracted)

            (extracted / "AppRun.sh").write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "changed payload bytes"):
                appimage_build.verify_extracted_tree(appdir, extracted)
            shutil.copyfile(appdir / "AppRun.sh", extracted / "AppRun.sh")
            (extracted / "AppRun.sh").chmod(0o755)

            os.symlink("../../outside", appdir / "badlink")
            os.symlink("../../outside", extracted / "badlink")
            with self.assertRaisesRegex(ValueError, "escapes AppDir"):
                appimage_build.verify_extracted_tree(appdir, extracted)
            (appdir / "badlink").unlink()
            (extracted / "badlink").unlink()

            os.symlink("/bin/true", appdir / "abslink")
            os.symlink("/bin/true", extracted / "abslink")
            with self.assertRaisesRegex(ValueError, "absolute symlink"):
                appimage_build.verify_extracted_tree(appdir, extracted)

    def test_zsync_control_validation(self):
        with tempfile.TemporaryDirectory(prefix="serein-appimage-zsync-") as directory:
            zsync = Path(directory) / "test.AppImage.zsync"
            image_size = 4096
            header = (
                "zsync: 0.6.2\n"
                "Filename: serein.AppImage\n"
                "Blocksize: 2048\n"
                f"Length: {image_size}\n"
                "Hash-Lengths: 2,2,4\n"
                "URL: serein.AppImage\n\n"
            ).encode("utf-8")
            sums = b"\x00" * (2 * (2 + 4))
            zsync.write_bytes(header + sums)
            appimage_build.verify_zsync_control(zsync, image_size)

            with self.assertRaisesRegex(ValueError, "Length"):
                appimage_build.verify_zsync_control(zsync, image_size + 1)

            zsync.write_bytes(header + sums[:-1])
            with self.assertRaisesRegex(ValueError, "block count"):
                appimage_build.verify_zsync_control(zsync, image_size)

    def test_updater_linux_script_resets_runtime_env_and_preserves_host_vars(self):
        script_path = Path(__file__).resolve().parents[2] / "apps/desktop/src/updater_linux.sh"
        script_text = script_path.read_text(encoding="utf-8")
        marker_line = 'printf \'%s\\n\' "$$" > "$stage/helper-ready"'
        self.assertIn(marker_line, script_text)
        prelude = script_text.split(marker_line, 1)[0]
        old_appdir = "/tmp/mount Serein/appdir"
        # Inject LD_PRELOAD/LD_LIBRARY_PATH inside the shell so ld.so doesn't warn on startup
        # before the script's unset logic executes.
        prelude_with_preload = (
            '#!/bin/sh\nset -eu\n'
            f'export LD_LIBRARY_PATH="{old_appdir}/lib" '
            f'LD_PRELOAD="{old_appdir}/lib/sharun-preload/anylinux.so"\n'
            + prelude.split("\nset -eu\n", 1)[1]
        )
        harness = prelude_with_preload + "\nenv\n"

        with tempfile.TemporaryDirectory(prefix="serein-updater-sh-") as directory:
            runner = Path(directory) / "env_check.sh"
            runner.write_text(harness, encoding="utf-8")
            clean_env = {
                "APPIMAGE": "/home/user/Serein.AppImage",
                "APPDIR": f"{old_appdir}/",
                "ARGV0": "serein",
                "ARG0": "/home/user/Serein.AppImage",
                "OWD": "/home/user",
                "PATH": f"{old_appdir}/bin:/usr/bin:/my host tools",
                "HOSTPATH": "/usr/bin:/my host tools",
                "HOME": "/home/user",
                "HOST_HOME": "/home/user",
                "XDG_CACHE_HOME": "/home/user/.cache/AppImage-Cache",
                "HOST_XDG_CACHE_HOME": "/home/user/.cache",
                "XDG_DATA_DIRS": f"{old_appdir}/share:/home/user/.local/share:/usr/share",
                "GCONV_PATH": f"{old_appdir}/lib/gconv",
                "SSL_CERT_FILE": "/etc/ssl/certs/ca-certificates.crt",
                "OPENSSL_CONF": f"{old_appdir}/etc/ssl/openssl.cnf",
                "CROSS_LIBC_DLOPEN_ROOT": old_appdir,
                "SHARUN_DIR": old_appdir,
                "SHARUN_BIN_NAME": "serein",
                "URUNTIME": "/home/user/Serein.AppImage",
                "APPIMAGE_EXTRACT_AND_RUN": "1",
            }
            output = subprocess.check_output(
                ["/bin/sh", str(runner), directory, "/home/user/Serein.AppImage", "1", f"{directory}/commit"],
                env=clean_env,
                text=True,
            )
            parsed = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
            self.assertEqual(parsed.get("PATH"), "/usr/bin:/my host tools")
            self.assertEqual(parsed.get("XDG_CACHE_HOME"), "/home/user/.cache")
            self.assertEqual(parsed.get("XDG_DATA_DIRS"), "/home/user/.local/share:/usr/share")
            self.assertEqual(parsed.get("SSL_CERT_FILE"), "/etc/ssl/certs/ca-certificates.crt")
            self.assertEqual(parsed.get("APPIMAGE_EXTRACT_AND_RUN"), "1")
            for removed in (
                "APPIMAGE",
                "APPDIR",
                "ARGV0",
                "ARG0",
                "OWD",
                "LD_LIBRARY_PATH",
                "LD_PRELOAD",
                "GCONV_PATH",
                "OPENSSL_CONF",
                "CROSS_LIBC_DLOPEN_ROOT",
                "SHARUN_DIR",
                "URUNTIME",
                "HOSTPATH",
                "HOST_XDG_CACHE_HOME",
            ):
                self.assertNotIn(removed, parsed)

            # Fallback without HOSTPATH / HOST_XDG_CACHE_HOME still strips old AppDir and AppImage-Cache.
            fallback_runner = Path(directory) / "env_check_fallback.sh"
            fallback_runner.write_text(prelude + "\nenv\n", encoding="utf-8")
            fallback_env = {
                "APPDIR": old_appdir,
                "PATH": f"{old_appdir}/bin:/opt/custom bin:/bin",
                "XDG_CACHE_HOME": "/home/user/.cache/AppImage-Cache",
            }
            fallback_out = subprocess.check_output(
                ["/bin/sh", str(fallback_runner), directory, "/home/user/Serein.AppImage", "1", f"{directory}/commit"],
                env=fallback_env,
                text=True,
            )
            fallback_parsed = dict(line.split("=", 1) for line in fallback_out.splitlines() if "=" in line)
            self.assertEqual(fallback_parsed.get("PATH"), "/opt/custom bin:/bin")
            self.assertNotIn("XDG_CACHE_HOME", fallback_parsed)

    def test_end_to_end_appimage_build_with_pinned_tools(self):
        if not WITH_TOOLS:
            self.skipTest("Pass --with-tools to exercise quick-sharun, appimagetool and uruntime end-to-end")
        probe_exe = shutil.which("printenv")
        self.assertIsNotNone(probe_exe, "printenv is required for synthetic AppImage runtime verification")
        with tempfile.TemporaryDirectory(prefix="serein-appimage-e2e-") as directory:
            root = Path(directory)
            staged = root / "staged"
            staged.mkdir()
            shutil.copyfile(probe_exe, staged / "serein")
            (staged / "serein").chmod(0o755)
            for name in ("README.md", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"):
                (staged / name).write_text("synthetic package fixture\n", encoding="utf-8")
            (staged / "licenses").mkdir()
            for name in (
                "NotoSansCJK-LICENSE.txt",
                "NotoSansArabic-OFL.txt",
                "NotoSansMath-OFL.txt",
                "NotoSansSymbols2-OFL.txt",
                "Inter-OFL.txt",
                "Bangers-OFL.txt",
                "BioRhyme-OFL.txt",
                "CherryBombOne-OFL.txt",
                "Chicle-OFL.txt",
                "Compagnon-OFL.txt",
                "MuseoModerno-OFL.txt",
                "PixelifySans-OFL.txt",
                "Ribes-OFL.txt",
                "Sinistre-OFL.txt",
                "ZillaSlab-OFL.txt",
                "PlaypenSans-OFL.txt",
                "Orbitron-OFL.txt",
                "NewRocker-OFL.txt",
                "Kalam-OFL.txt",
                "Twemoji-CC-BY-4.0.txt",
                "Unicode-LICENSE.txt",
                "Phosphor-Icons-MIT.txt",
                "Simple-Icons-CC0.txt",
            ):
                (staged / "licenses" / name).write_text("synthetic license\n", encoding="utf-8")
            for name in (
                "licenses/files",
                "licenses/notifications",
                "licenses/login",
                "licenses/voice",
                "licenses/audio",
                "licenses/dependencies",
                "source/hpke-rs",
            ):
                source = Path("vendor/hpke-rs") if name.startswith("source/") else Path("assets") / name
                shutil.copytree(source, staged / name)
                (staged / name / "stale-nested.log").write_text("synthetic private marker\n", encoding="utf-8")

            # Simulate a dirty dist tree: none of these should enter the AppImage.
            (staged / "debug.log").write_text("synthetic private marker\n", encoding="utf-8")
            (staged / "previous.AppImage").write_text("stale\n", encoding="utf-8")

            appimage_build.build(staged, "0.1.0-test")
            artifact = staged / "serein-0.1.0-test-Linux-X64.AppImage"
            zsync = staged / "serein-0.1.0-test-Linux-X64.AppImage.zsync"
            self.assertTrue(artifact.is_file())
            self.assertTrue(zsync.is_file())

            extract_dir = root / "verify-extract"
            extract_dir.mkdir()
            subprocess.run([str(artifact), "--appimage-extract"], cwd=extract_dir, check=True)
            extracted = extract_dir / "AppDir"
            for required in (
                "AppRun",
                "AppRun.sh",
                "sharun",
                "dash",
                "bin/serein",
                "shared/bin/serein",
                "share/licenses/dash-COPYING",
                "lib/sharun-preload/cross-libc-dlopen.so",
                "usr/share/doc/serein/AppImage-README.md",
                "usr/share/doc/serein/licenses/AppImage-runtime.txt",
                "usr/share/doc/serein/licenses/cross-libc-dlopen-LICENSE.txt",
                "usr/share/doc/serein/licenses/dependencies/PROVENANCE.md",
                "usr/share/doc/serein/AppImage-bundled-packages.txt",
            ):
                self.assertTrue((extracted / required).exists(), f"Missing {required}")
            listing = "\n".join(p.relative_to(extracted).as_posix() for p in extracted.rglob("*"))
            for excluded in ("debug.log", "stale-nested.log", "previous.AppImage", "source/hpke-rs", "usr/bin/serein"):
                self.assertNotIn(excluded, listing)

            printed_appdir = subprocess.check_output(
                [str(artifact), "APPDIR"],
                env={**os.environ, "APPIMAGE_EXTRACT_AND_RUN": "1"},
                text=True,
            ).strip()
            self.assertTrue(Path(printed_appdir).is_absolute())

            with self.assertRaisesRegex(ValueError, "semantic application version"):
                appimage_build.build(staged, "0.1.0\nbad")
            (staged / "serein").write_bytes(b"MZ synthetic wrong architecture")
            with self.assertRaisesRegex(ValueError, "ELF executable"):
                appimage_build.build(staged, "0.1.0")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--with-tools",
        action="store_true",
        help="Run the full synthetic AppImage build/extraction check using target/appimage-tools",
    )
    args, remaining = parser.parse_known_args()
    WITH_TOOLS = args.with_tools
    unittest.main(argv=[__file__, *remaining])
