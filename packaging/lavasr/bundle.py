"""Stage pinned LavaSRv2 models and the native CPU runtime; no runtime downloads."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import tarfile
import tempfile
from urllib.request import urlopen
import zipfile

REPO = Path(__file__).resolve().parents[2]
VERSION = "1.23.2"
MODELS = (
    ("backbone.onnx", 51761958, "0bc915649f765d40ca52bd3a2272a28e8f7636b09c2ee3148662a69041f28cd9"),
    ("spec_head.onnx", 4208522, "a54a5d5d652aceb76b2f48cf9f0ab3f4def0d44427dc7af641fd24284342a48a"),
    ("denoiser_core.onnx", 1173265, "515a1be1eea4b1108cc6a758198d726fc506e68ec53d7e522bc2618ff2602175"),
)
RUNTIMES = {
    ("Linux", "x86_64"): ("onnxruntime-linux-x64-1.23.2.tgz", 8309231,
        "1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b", "libonnxruntime.so.1.23.2"),
    ("Linux", "arm64"): ("onnxruntime-linux-aarch64-1.23.2.tgz", 7254068,
        "7c63c73560ed76b1fac6cff8204ffe34fe180e70d6582b5332ec094810241e5c", "libonnxruntime.so.1.23.2"),
    ("Darwin", "arm64"): ("onnxruntime-osx-arm64-1.23.2.tgz", 9999931,
        "b4d513ab2b26f088c66891dbbc1408166708773d7cc4163de7bdca0e9bbb7856", "libonnxruntime.1.23.2.dylib"),
    ("Darwin", "x86_64"): ("onnxruntime-osx-x86_64-1.23.2.tgz", 11676322,
        "d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6", "libonnxruntime.1.23.2.dylib"),
    ("Windows", "arm64"): ("onnxruntime-win-arm64-1.23.2.zip", 78932411,
        "1cfe88b6435df3b5fb0e9f6bd7d6f5df1e887b6174de7f6e2a47bab956f3f168", "onnxruntime.dll"),
    ("Windows", "x86_64"): ("onnxruntime-win-x64-1.23.2.zip", 78127794,
        "0b38df9af21834e41e73d602d90db5cb06dbd1ca618948b8f1d66d607ac9f3cd", "onnxruntime.dll"),
}
SOURCES = (
    ("eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip", 4075744,
     "6a60d76351f97132669daeeb721d6bf14b008101883ad2d687a3201c5c461eb0",
     "https://github.com/eigen-mirror/eigen/archive/1d8b82b0740839c0de7f1242a3585e3390ff5f33/eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip"),
)


def host():
    arch = platform.machine().lower()
    if platform.system() == "Windows":
        arch = os.environ.get("VSCMD_ARG_TGT_ARCH", arch).lower()
    return platform.system(), {"amd64": "x86_64", "x64": "x86_64", "aarch64": "arm64"}.get(arch, arch)


def valid(path, size, digest):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != size:
        return False
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() == digest


def fetch(cache, name, size, digest, url, offline):
    path = cache / name
    if valid(path, size, digest):
        return path
    if offline:
        raise ValueError(f"Missing or corrupt pinned offline asset: {name}")
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=cache, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            with urlopen(url, timeout=60) as response:
                count = 0
                while data := response.read(65536):
                    count += len(data)
                    if count > size:
                        raise ValueError(f"Oversized download: {name}")
                    stream.write(data)
            stream.close()
            if not valid(temporary, size, digest):
                raise ValueError(f"Asset checksum/size mismatch: {name}")
            temporary.replace(path)
        finally:
            stream.close()
            temporary.unlink(missing_ok=True)
    return path


def extract(archive, member, destination):
    """Read only one exact regular file; never extract paths or links from an archive."""
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as bundle:
            entry = bundle.getinfo(member)
            if entry.is_dir() or stat.S_IFMT(entry.external_attr >> 16) not in (0, stat.S_IFREG) or entry.file_size > 64 * 1024 * 1024:
                raise ValueError(f"Invalid runtime member: {member}")
            with bundle.open(entry) as stream:
                data = stream.read(64 * 1024 * 1024 + 1)
    else:
        with tarfile.open(archive) as bundle:
            try:
                entry = bundle.getmember(member)
            except KeyError:
                # Official macOS archives retain an explicit ./ prefix.
                entry = bundle.getmember("./" + member)
            if not entry.isfile() or entry.size > 64 * 1024 * 1024:
                raise ValueError(f"Invalid runtime member: {member}")
            with bundle.extractfile(entry) as stream:
                data = stream.read(64 * 1024 * 1024 + 1)
    if not data or len(data) > 64 * 1024 * 1024:
        raise ValueError(f"Invalid runtime file: {member}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Replace the inode, including a previously loaded macOS signed library.
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(data)
            stream.close()
            temporary.replace(destination)
            destination.chmod(0o644)
        finally:
            stream.close()
            temporary.unlink(missing_ok=True)


def windows_crt(root, destination, arch):
    redist = os.environ.get("VCToolsRedistDir")
    if not redist:
        raise ValueError("Windows packaging needs VCToolsRedistDir from the MSVC development environment")
    choices = sorted((Path(redist) / ("x64" if arch == "x86_64" else "arm64")).glob("Microsoft.VC*.CRT"))
    if not choices:
        raise ValueError("MSVC app-local runtime directory is unavailable")
    required = ("msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll")
    if arch == "x86_64":
        required += ("vcruntime140_1.dll",)
    for name in required:
        source = choices[-1] / name
        if not source.is_file() or source.is_symlink():
            raise ValueError(f"MSVC app-local runtime is unavailable: {name}")
        # Application-directory placement also satisfies the default Windows loader.
        shutil.copyfile(source, root / name)
    if arch == "arm64":
        # MSVC can place the x64-only EH runtime in the ARM64 redist directory for emulation.
        (root / "vcruntime140_1.dll").unlink(missing_ok=True)
    shutil.copyfile(REPO / "assets/licenses/voice/lavasr/MSVC-LICENSE.txt", destination / "MSVC-LICENSE.txt")


def stage(root, cache, offline=False, target=None):
    system, arch = target or host()
    try:
        name, size, digest, library = RUNTIMES[system, arch]
    except KeyError as error:
        raise ValueError(f"Unverified speech runtime platform: {system}/{arch}") from error
    resources = root / "Serein.app/Contents/Resources" if system == "Darwin" else root
    destination = resources / "voice-enhancement"
    if destination.is_symlink():
        raise ValueError("Refusing symlink speech output")
    destination.mkdir(parents=True, exist_ok=True)
    allowed = {model[0] for model in MODELS} | {s[0] for s in SOURCES} | {"ONNXRuntime-LICENSE", "ONNXRuntime-ThirdPartyNotices.txt", "build.json", library}
    if system == "Windows":
        allowed |= {"MSVC-LICENSE.txt"}
    # Reject stale/unexpected payload rather than ship arbitrary contents of a cached folder.
    if any(p.is_symlink() or p.name not in allowed or not p.is_file() for p in destination.iterdir()):
        raise ValueError("Unexpected file in speech staging directory; use a clean package output")
    for model, length, sha in MODELS:
        source = fetch(cache, model, length, sha,
            f"https://github.com/idocinthebox/LavaSR-ONNX-Embedded/releases/download/v1.0.0/{model}", offline)
        shutil.copyfile(source, destination / model)
    for source, length, sha, url in SOURCES:
        archive = fetch(cache, source, length, sha, url, offline)
        shutil.copyfile(archive, destination / source)
    archive = fetch(cache, name, size, digest,
        f"https://github.com/microsoft/onnxruntime/releases/download/v{VERSION}/{name}", offline)
    prefix = name.removesuffix(".tgz").removesuffix(".zip")
    native = root / "Serein.app/Contents/Frameworks" if system == "Darwin" else destination
    extract(archive, f"{prefix}/lib/{library}", native / library)
    extract(archive, f"{prefix}/LICENSE", destination / "ONNXRuntime-LICENSE")
    extract(archive, f"{prefix}/ThirdPartyNotices.txt", destination / "ONNXRuntime-ThirdPartyNotices.txt")
    if system == "Windows":
        windows_crt(root, destination, arch)
    (destination / "build.json").write_text(json.dumps({
        "model": "LavaSRv2", "denoising": True,
        "model_source": "ysharma3501/LavaSR@33ac040892519c1bb4aed7eb32e79af51cc29e2a",
        "model_revision": "b98dc8be472da45ab7b6346ad7997e1dfeb5911d",
        "exporter": "idocinthebox/LavaSR-ONNX-Embedded@b591321da28c070cdfb8af961df4cb5be662490d",
        "runtime": VERSION, "platform": [system, arch],
        "assets": [{"name": m, "size": s, "sha256": h} for m, s, h in MODELS],
        "runtime_archive_sha256": digest,
        "corresponding_source": [{"name": n, "sha256": h, "url": u} for n, _, h, u in SOURCES],
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Staged LavaSRv2 with denoising and ONNX Runtime {VERSION} ({system}/{arch})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--cache-dir", type=Path, default=Path(os.environ.get("SEREIN_LAVASR_CACHE", str(Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "lavasr-cache"))))
    parser.add_argument("--offline", action="store_true", default=os.environ.get("SEREIN_LAVASR_OFFLINE") == "1")
    args = parser.parse_args()
    stage(args.root, args.cache_dir, args.offline)


if __name__ == "__main__":
    main()
