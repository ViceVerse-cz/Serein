"""Small trust-boundary regression check; native signature checks run during build."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

from build import input_packages, validate
from verify_downloads import verify


class RepositoryInputs(unittest.TestCase):
    def test_publishing_url_matches_current_pages_site_or_explicit_override(self):
        source = Path(__file__).resolve().parents[2]
        workflow = (source / ".github/workflows/package-repositories.yml").read_text()
        download = workflow.split("\n  download:\n", 1)[1].split("\n  repositories:\n", 1)[0]
        self.assertIn("pages: read", download)
        self.assertIn("base_url: ${{ steps.hosting.outputs.base_url }}", download)
        step = download.split("      - name: Resolve repository URL\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("GH_REPO: ${{ github.repository }}", step)
        self.assertIn("INPUT_BASE_URL: ${{ inputs.base_url }}", step)
        command = textwrap.dedent(step.split("        run: |\n", 1)[1])
        self.assertEqual(workflow.count("BASE_URL: ${{ needs.download.outputs.base_url }}"), 2)
        release = (source / ".github/workflows/release.yml").read_text()
        self.assertNotIn("base_url:", release.split("\n  repositories:\n", 1)[1])
        self.assertNotIn("default: https://viceverse-cz.github.io/Serein", workflow)
        call_url = workflow.split("  workflow_call:\n", 1)[1].split("      base_url:\n", 1)[1].split("      deploy:\n", 1)[0]
        self.assertIn("required: false", call_url)
        flatpak = re.search(r"python3 - <<'PY'\n(.*?)\n          PY\n", workflow, re.S)
        self.assertIsNotNone(flatpak)
        with tempfile.TemporaryDirectory(prefix="serein-publishing-url-") as temporary:
            root = Path(temporary)
            commands = root / "commands"
            commands.mkdir()
            gh = commands / "gh"
            gh.write_text('#!/bin/sh\nset -eu\nprintf "%s\\n" "$*" > "$GH_FIXTURE_LOG"\n'
                          'test "${GH_FIXTURE_FAIL:-0}" = 0\nprintf "%s\\n" "$GH_FIXTURE_URL"\n')
            gh.chmod(0o755)
            (root / "packaging/flatpak").mkdir(parents=True)
            (root / "packaging/flatpak/serein.flatpakref").write_bytes(
                (source / "packaging/flatpak/serein.flatpakref").read_bytes())
            (root / "site/flatpak").mkdir(parents=True)
            for repository, configured_url, override in (
                ("ViceVerse-cz/Serein", "https://viceverse-cz.github.io/Serein/", ""),
                ("Fork-Owner/NuSerein", "https://fork-owner.github.io/NuSerein/", ""),
                ("Fork-Owner/fork-owner.github.io", "https://fork-owner.github.io/", ""),
                ("Fork-Owner/NuSerein", "https://packages.example.org/", ""),
                ("Fork-Owner/NuSerein", "https://unused.example.org/", "https://mirror.example.org/serein/"),
            ):
                with self.subTest(repository=repository, configured_url=configured_url, override=override):
                    output, log = root / "output", root / "gh.log"
                    output.write_text("")
                    log.unlink(missing_ok=True)
                    environment = os.environ | {"PATH": str(commands) + os.pathsep + os.environ["PATH"],
                        "GH_REPO": repository, "INPUT_BASE_URL": override, "GITHUB_OUTPUT": str(output),
                        "GH_FIXTURE_LOG": str(log), "GH_FIXTURE_URL": configured_url,
                        "GH_FIXTURE_FAIL": "1" if override else "0"}
                    result = subprocess.run(["bash", "-e", "-c", command], cwd=root,
                                            env=environment, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    expected = (override or configured_url).rstrip("/")
                    self.assertEqual(output.read_text(), f"base_url={expected}\n")
                    if override:
                        self.assertFalse(log.exists())
                    else:
                        self.assertEqual(log.read_text(), f"api repos/{repository}/pages --jq .html_url\n")
                    subprocess.run([os.sys.executable, "-c", textwrap.dedent(flatpak.group(1))],
                                   cwd=root, env=environment | {"BASE_URL": expected}, check=True)
                    reference = (root / "site/flatpak/serein.flatpakref").read_text()
                    self.assertIn(f"\nUrl={expected}/flatpak/repo\n", reference)
                    self.assertIn(f"\nIcon={expected}/icons/serein.png\n", reference)
            output.write_text("")
            result = subprocess.run(["bash", "-e", "-c", command], cwd=root, capture_output=True,
                                    env=environment | {"INPUT_BASE_URL": "", "GH_FIXTURE_FAIL": "1"})
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(output.read_text(), "")

    def test_downloads_require_complete_matching_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "serein.deb"
            package.write_bytes(b"synthetic package")
            manifest = root / "SHA256SUMS.txt"
            entry = hashlib.sha256(package.read_bytes()).hexdigest() + "  ./serein.deb\n"
            manifest.write_text(entry + "0" * 64 + "  ./macOS.zip\n")
            verify(root)
            package.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                verify(root)
            package.write_bytes(b"synthetic package")
            (root / "unlisted.rpm").write_bytes(b"unlisted")
            with self.assertRaisesRegex(ValueError, "missing"):
                verify(root)
            (root / "unlisted.rpm").unlink()
            manifest.write_text(entry + entry)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                verify(root)
            manifest.write_text("0" * 64 + "  ../escape.deb\n")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                verify(root)
            manifest.write_text(entry)
            package.unlink()
            package.symlink_to(manifest)
            with self.assertRaisesRegex(ValueError, "unsafe"):
                verify(root)
            package.unlink()
            flatpak_pkg = root / "serein.flatpak"
            flatpak_pkg.write_bytes(b"synthetic flatpak")
            flatpak_entry = hashlib.sha256(flatpak_pkg.read_bytes()).hexdigest() + "  ./serein.flatpak\n"
            manifest.write_text(flatpak_entry)
            verify(root)

    def test_mixed_release_packages_are_selected_per_distribution_and_architecture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = [
                "serein-v1.0.0-Linux-ubuntu-26.04-serein_1.0.0_arm64.deb",
                "serein-v1.0.0-Linux-ubuntu-26.04-serein_1.0.0_amd64.deb",
                "serein-v1.0.0-Linux-ubuntu-24.04-serein_1.0.0_arm64.deb",
                "serein-v1.0.0-Linux-fedora-44-serein-1.0.0.x86_64.rpm",
                "serein-v1.0.0-Linux-x86_64.flatpak",
                "SHA256SUMS.txt",
            ]
            for name in names:
                (root / name).write_bytes(b"synthetic release package")
            for arch, index in [("arm64", 0), ("amd64", 1)]:
                selected = input_packages(root, "deb", "ubuntu-26.04", arch)
                self.assertEqual(selected, [root / names[index]])
            self.assertEqual(input_packages(root, "rpm", "fedora-44", "x86_64"), [root / names[3]])
            with self.assertRaisesRegex(ValueError, "1–100 packages"):
                input_packages(root, "deb", "ubuntu-26.04", "armhf")
            self.assertEqual(len(input_packages(root, "deb")), 3)

    def test_rejects_unsafe_paths_keys_and_urls(self):
        good = dict(distribution="ubuntu-26.04", architecture="amd64",
                    key="A" * 40, base_url="https://packages.example.org/serein")
        validate(argparse.Namespace(**good))
        for field, value in [("distribution", "../escape"), ("architecture", "/amd64"),
                             ("key", "ABC123"), ("key", "A" * 40 + "\n"),
                             ("base_url", "http://example.org"),
                             ("base_url", "https://user:secret@example.org"),
                             ("base_url", "https://example.org/\nenabled=0"),
                             ("base_url", "https://example.org/?token=secret")]:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate(argparse.Namespace(**(good | {field: value})))

    def test_setup_script_syntax_and_default_fingerprint(self):
        setup_sh = Path(__file__).resolve().parent / "setup.sh"
        self.assertTrue(setup_sh.is_file())
        res = subprocess.run(["sh", "-n", str(setup_sh)], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, res.stderr)
        content = setup_sh.read_text()
        self.assertIn("EXPECTED_FINGERPRINT=", content)
        self.assertIn("CA19DA939E9BCAB500751CE480FE95CAD86141A5", content)

    def test_generate_index(self):
        from build import generate_index
        with tempfile.TemporaryDirectory() as temporary:
            dest = Path(temporary) / "site"
            generate_index(dest)
            index_file = dest / "index.html"
            self.assertTrue(index_file.is_file())
            content = index_file.read_text()
            self.assertIn("Serein Linux Repositories", content)
            self.assertIn("setup.sh", content)


if __name__ == "__main__":
    unittest.main()
