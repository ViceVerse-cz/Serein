"""Check offline preparation without downloading crates or touching a real toolchain."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import build


def clean_git_environment():
    return {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}


class PreparationTest(unittest.TestCase):
    def test_generate_flatpakref(self):
        ref = build.generate_flatpakref("https://example.com/flatpak/repo")
        self.assertIn("Name=cz.viceverse.serein", ref)
        self.assertIn("Url=https://example.com/flatpak/repo", ref)
        self.assertIn("RuntimeRepo=https://flathub.org/repo/flathub.flatpakrepo", ref)

    def test_locked_sources_and_exact_compiler_exclude_untracked_data(self):
        self.prepare_fixture()

    def test_tracked_symlink_is_rejected(self):
        self.prepare_fixture(symlink=True)

    def test_inherited_git_environment_cannot_modify_parent_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory) / "parent"
            parent.mkdir()
            subprocess.run(["git", "init", "--quiet", str(parent)], check=True, env=clean_git_environment())
            tracked = parent / "parent-tracked"
            tracked.write_text("parent source must remain unchanged")
            subprocess.run(["git", "add", "parent-tracked"], cwd=parent, check=True, env=clean_git_environment())
            index = parent / ".git/index"
            before_index = index.read_bytes()
            before_source = tracked.read_bytes()
            inherited = {
                "GIT_DIR": str(parent / ".git"),
                "GIT_WORK_TREE": str(parent),
                "GIT_INDEX_FILE": str(index),
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "core.worktree",
                "GIT_CONFIG_VALUE_0": str(parent),
                "SEREIN_FIXTURE_ENV": "retained",
            }
            original_output = subprocess.check_output

            def checked_output(*args, **kwargs):
                effective_environment = kwargs.get("env", os.environ)
                self.assertFalse(any(key.startswith("GIT_") for key in effective_environment))
                self.assertEqual(effective_environment["SEREIN_FIXTURE_ENV"], "retained")
                return original_output(*args, **kwargs)

            before_environment = dict(os.environ)
            with patch.dict(os.environ, inherited), patch.object(subprocess, "check_output", side_effect=checked_output):
                polluted_environment = dict(os.environ)
                self.prepare_fixture(inherited=inherited)
                self.assertEqual(dict(os.environ), polluted_environment)
            self.assertEqual(dict(os.environ), before_environment)
            self.assertEqual(index.read_bytes(), before_index)
            self.assertEqual(tracked.read_bytes(), before_source)
            self.assertEqual({path.name for path in parent.iterdir()}, {".git", "parent-tracked"})

    def prepare_fixture(self, symlink=False, inherited=None):
        with patch.dict(os.environ, clean_git_environment(), clear=True):
            self._prepare_fixture(symlink, inherited)

    def _prepare_fixture(self, symlink, inherited=None):
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
            manifest = json.loads((build.ROOT / "packaging/flatpak/cz.viceverse.serein.json").read_text())
            (packaging / "cz.viceverse.serein.json").write_text(json.dumps(manifest))
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            subprocess.run(["git", "add", ".cargo/config.toml", "Cargo.lock", "rust-toolchain.toml",
                            "packaging/flatpak/cz.viceverse.serein.json"], cwd=root, check=True)
            (packaging / "private-untracked").write_text("must not copy packaging secrets")
            initialized = root / "initialized-submodule"
            initialized.mkdir()
            subprocess.run(["git", "init", "--quiet", str(initialized)], check=True)
            (initialized / "tracked-child").write_text("must not copy submodule content")
            subprocess.run(["git", "add", "tracked-child"], cwd=initialized, check=True)
            subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                            "commit", "--quiet", "-m", "Synthetic submodule"], cwd=initialized, check=True)
            (initialized / "private-untracked").write_text("must not copy submodule secrets")
            commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=initialized, text=True).strip()
            for name in ["initialized-submodule", "uninitialized-submodule", "missing-submodule"]:
                subprocess.run(["git", "update-index", "--add", "--cacheinfo", f"160000,{commit},{name}"],
                               cwd=root, check=True)
            (root / "uninitialized-submodule").mkdir()
            if symlink:
                (root / "tracked-symlink").symlink_to("private-untracked")
                subprocess.run(["git", "add", "tracked-symlink"], cwd=root, check=True)
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
                    patch.object(build, "output", side_effect=command):
                if symlink:
                    with self.assertRaisesRegex(ValueError, "Refusing symlink source: tracked-symlink"):
                        build.prepare(destination)
                    return
                # Apply pollution at the actual production boundary, after the
                # independent fixture repositories have been created safely.
                with patch.dict(os.environ, inherited or {}):
                    before_prepare = dict(os.environ)
                    build.prepare(destination)
                    self.assertEqual(dict(os.environ), before_prepare)
                with self.assertRaises(FileExistsError):
                    build.prepare(destination)
            source = destination / "source"
            self.assertFalse((source / "private-untracked").exists())
            self.assertFalse((source / "packaging/flatpak/private-untracked").exists())
            for name in ["initialized-submodule", "uninitialized-submodule", "missing-submodule"]:
                self.assertFalse((source / name).exists())
            self.assertEqual(json.loads((source / "packaging/flatpak/cz.viceverse.serein.json").read_text()), manifest)
            self.assertEqual((source / "Cargo.lock").read_text(), "locked fixture")
            self.assertTrue((source / "flatpak-rust/bin/rustc").is_file())
            config = (source / ".cargo/config.toml").read_text()
            self.assertIn('[alias]', config)
            self.assertIn('directory = "cargo-vendor"', config)
            self.assertEqual(json.loads((destination / "cz.viceverse.serein.json").read_text()), manifest)
