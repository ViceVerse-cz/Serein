"""Check offline preparation without downloading crates or touching a real toolchain."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build


class PreparationTest(unittest.TestCase):
    def test_generate_flatpakref(self):
        ref = build.generate_flatpakref("https://example.com/flatpak/repo")
        self.assertIn("Name=org.serein.desktop", ref)
        self.assertIn("Url=https://example.com/flatpak/repo", ref)
        self.assertIn("RuntimeRepo=https://flathub.org/repo/flathub.flatpakrepo", ref)

    def test_locked_sources_and_exact_compiler_exclude_untracked_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            (root / ".cargo").mkdir()
            (root / ".cargo/config.toml").write_text('[alias]\nxtask = "run -p xtask --"\n')
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.1"\n')
            (root / "Cargo.lock").write_text("locked fixture")
            (root / "private-untracked").write_text("must not copy")
            packaging = root / "packaging/flatpak"
            packaging.mkdir(parents=True)
            manifest = json.loads((build.ROOT / "packaging/flatpak/org.serein.desktop.json").read_text())
            (packaging / "org.serein.desktop.json").write_text(json.dumps(manifest))
            compiler = Path(directory) / "compiler"
            (compiler / "bin").mkdir(parents=True)
            (compiler / "bin/rustc").write_text("compiler fixture")
            destination = Path(directory) / "prepared"

            def command(*args, cwd=build.ROOT):
                if args[-2:] == ("--print", "sysroot"):
                    return str(compiler)
                if args[-1] == "--version":
                    return "rustc 1.98.1 (fixture)"
                self.assertEqual(args, ("rustup", "run", "1.98.1", "cargo", "vendor", "--locked", "cargo-vendor"))
                self.assertEqual(cwd, destination / "source")
                return '[source.crates-io]\nreplace-with = "vendored-sources"\n[source.vendored-sources]\ndirectory = "cargo-vendor"'

            with patch.object(build, "ROOT", root), patch.object(build.platform, "system", return_value="Linux"), \
                    patch.object(build, "output", side_effect=command), \
                    patch.object(build.subprocess, "check_output", return_value=b".cargo/config.toml\0Cargo.lock\0rust-toolchain.toml\0"):
                build.prepare(destination)
                with self.assertRaises(FileExistsError):
                    build.prepare(destination)
            source = destination / "source"
            self.assertFalse((source / "private-untracked").exists())
            self.assertEqual((source / "Cargo.lock").read_text(), "locked fixture")
            self.assertTrue((source / "flatpak-rust/bin/rustc").is_file())
            config = (source / ".cargo/config.toml").read_text()
            self.assertIn('[alias]', config)
            self.assertIn('directory = "cargo-vendor"', config)
            self.assertEqual(json.loads((destination / "org.serein.desktop.json").read_text()), manifest)
