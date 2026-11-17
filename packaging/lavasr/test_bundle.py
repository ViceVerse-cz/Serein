"""Offline speech packaging fixtures; synthetic models/libraries are never executed."""
from pathlib import Path
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import bundle


class BundleTests(unittest.TestCase):
    def test_offline_cache_checks_size_and_digest_and_rejects_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = b"synthetic model"
            digest = hashlib.sha256(data).hexdigest()
            path = root / "model"
            path.write_bytes(data)
            with patch.object(bundle, "urlopen", side_effect=AssertionError("network forbidden")):
                self.assertEqual(bundle.fetch(root, "model", len(data), digest, "", True), path)
                for length, sha in [(len(data) + 1, digest), (len(data), "0" * 64)]:
                    with self.assertRaisesRegex(ValueError, "corrupt"):
                        bundle.fetch(root, "model", length, sha, "", True)
                link = root / "link"
                try:
                    link.symlink_to(path)
                except OSError:
                    return  # Windows unprivileged runners may disallow symlinks.
                with self.assertRaisesRegex(ValueError, "corrupt"):
                    bundle.fetch(root, "link", len(data), digest, "", True)

    def test_downloads_are_bounded_and_unverified_data_never_enters_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            for data in (b"oversized", b"bad"):
                with patch.object(bundle, "urlopen", return_value=io.BytesIO(data)):
                    with self.assertRaisesRegex(ValueError, "Oversized|mismatch"):
                        bundle.fetch(cache, "model", 3, "0" * 64, "https://example.invalid", False)
                self.assertEqual(list(cache.iterdir()), [])

    def test_clean_and_cached_staging_and_native_locations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / "cache"
            cache.mkdir()
            models = []
            for name in ("backbone.onnx", "spec_head.onnx", "denoiser_core.onnx"):
                data = b"synthetic " + name.encode()
                (cache / name).write_bytes(data)
                models.append((name, len(data), hashlib.sha256(data).hexdigest()))
            source_name = bundle.SOURCES[0][0]
            source_data = b"synthetic MPL corresponding source"
            (cache / source_name).write_bytes(source_data)
            sources = ((source_name, len(source_data), hashlib.sha256(source_data).hexdigest(), ""),)
            for system, arch in list(bundle.RUNTIMES):
                native = "onnxruntime.dll" if system == "Windows" else "libonnxruntime.1.23.2.dylib" if system == "Darwin" else "libonnxruntime.so.1.23.2"
                prefix = "synthetic-runtime"
                archive = cache / (prefix + (".zip" if system == "Windows" else ".tgz"))
                entries = {f"{prefix}/lib/{native}": b"synthetic library", f"{prefix}/LICENSE": b"synthetic license",
                           f"{prefix}/ThirdPartyNotices.txt": b"synthetic third-party notices"}
                if system == "Windows":
                    with zipfile.ZipFile(archive, "w") as output:
                        for name, data in entries.items():
                            output.writestr(name, data)
                else:
                    with tarfile.open(archive, "w:gz") as output:
                        for name, data in entries.items():
                            info = tarfile.TarInfo(("./" if system == "Darwin" else "") + name)
                            info.size = len(data)
                            output.addfile(info, io.BytesIO(data))
                runtime = (archive.name, archive.stat().st_size, hashlib.sha256(archive.read_bytes()).hexdigest(), native)
                crt = root / "redist" / ("x64" if arch == "x86_64" else "arm64") / "Microsoft.VC143.CRT"
                crt.mkdir(parents=True, exist_ok=True)
                for name in ("msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll"):
                    (crt / name).write_bytes(b"synthetic CRT")
                destination = root / f"{system}-{arch}"
                if (system, arch) == ("Windows", "arm64"):
                    destination.mkdir()
                    (destination / "vcruntime140_1.dll").write_bytes(b"stale x64 runtime")
                with patch.object(bundle, "MODELS", models), patch.object(bundle, "SOURCES", sources), patch.object(bundle, "RUNTIMES", {(system, arch): runtime}), \
                     patch.dict(bundle.os.environ, {"VCToolsRedistDir": str(root / "redist")}), \
                     patch.object(bundle, "urlopen", side_effect=AssertionError("network forbidden")):
                    bundle.stage(destination, cache, offline=True, target=(system, arch))
                    before = {p.relative_to(destination): p.read_bytes() for p in destination.rglob("*") if p.is_file()}
                    bundle.stage(destination, cache, offline=True, target=(system, arch))
                    after = {p.relative_to(destination): p.read_bytes() for p in destination.rglob("*") if p.is_file()}
                    self.assertEqual(before, after)
                    resources = destination / "Serein.app/Contents/Resources" if system == "Darwin" else destination
                    speech = resources / "voice-enhancement"
                    self.assertTrue(json.loads((speech / "build.json").read_text())["denoising"])
                    self.assertEqual((speech / source_name).read_bytes(), source_data)
                    if system == "Darwin":
                        self.assertTrue((destination / "Serein.app/Contents/Frameworks" / native).is_file())
                        self.assertFalse((speech / native).exists())
                    else:
                        self.assertTrue((speech / native).is_file())
                    if system == "Windows":
                        self.assertTrue((destination / "msvcp140_1.dll").is_file())
                        self.assertEqual((destination / "vcruntime140_1.dll").exists(), arch == "x86_64")
                    (speech / "private.log").write_text("must not ship")
                    with self.assertRaisesRegex(ValueError, "Unexpected"):
                        bundle.stage(destination, cache, offline=True, target=(system, arch))

    def test_only_regular_runtime_members_are_admitted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "bad.tgz"
            with tarfile.open(archive, "w:gz") as output:
                entry = tarfile.TarInfo("runtime/lib/library")
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/outside"
                output.addfile(entry)
            with self.assertRaisesRegex(ValueError, "Invalid runtime member"):
                bundle.extract(archive, "runtime/lib/library", root / "output")
            self.assertFalse((root / "output").exists())

    def test_pins_match_native_verification_flatpak_and_nix(self):
        native = (bundle.REPO / "crates/platform/src/speech.rs").read_text()
        nix = (bundle.REPO / "nix/package.nix").read_text()
        flatpak = json.loads((bundle.REPO / "packaging/flatpak/cz.viceverse.serein.json").read_text())
        module = next(m for m in flatpak["modules"] if m["name"] == "serein-speech-assets")
        sources = {s["dest-filename"]: s["sha256"] for s in module["sources"]}
        for name, size, digest in bundle.MODELS:
            self.assertIn(digest, native)
            self.assertIn(f"{size:_}", native)
            self.assertEqual(sources[name], digest)
            self.assertIn(digest, nix)
        name, _, sha, _ = bundle.RUNTIMES["Linux", "x86_64"]
        self.assertEqual(sources[name], sha)
        self.assertIn(sha, nix)
        for name, _, sha, _ in bundle.SOURCES:
            self.assertEqual(sources[name], sha)
            self.assertIn(sha, nix)


if __name__ == "__main__":
    unittest.main()
