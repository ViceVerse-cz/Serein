"""Prepare locked offline sources and build an unsigned Flatpak bundle on Linux."""

import argparse
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REPO_URL = "https://viceverse-cz.github.io/Serein/flatpak/repo"


def output(*args, cwd=ROOT):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def generate_flatpakref(repo_url=DEFAULT_REPO_URL):
    return f"""[Flatpak Ref]
Name=org.serein.desktop
Branch=master
Title=Serein
Comment=Fast, secure and lightweight native Discord client
Icon=https://viceverse-cz.github.io/Serein/icons/serein.png
Url={repo_url}
RuntimeRepo=https://flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
"""


def prepare(destination):
    if platform.system() != "Linux":
        raise ValueError("Flatpak preparation requires a native Linux Rust toolchain")
    pin = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    sysroot = Path(output("rustup", "run", pin, "rustc", "--print", "sysroot"))
    version = output(str(sysroot / "bin/rustc"), "--version").split()[1]
    manifest = json.loads((ROOT / "packaging/flatpak/org.serein.desktop.json").read_text())
    if version != pin or f"= {pin}" not in manifest["modules"][0]["build-commands"][0]:
        raise ValueError("Flatpak manifest and installed Rust must match rust-toolchain.toml")
    destination.mkdir(parents=True, exist_ok=False)
    source = destination / "source"
    source.mkdir()
    # Copy tracked working-tree inputs only: never private untracked files or target/.
    paths = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    paths += [p.relative_to(ROOT).as_posix() for p in (ROOT / "packaging/flatpak").glob("*") if p.is_file()]
    for name in sorted(set(paths) - {""}):
        src = ROOT / name
        if src.is_symlink():
            raise ValueError(f"Refusing symlink source: {name}")
        dest = source / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)
    shutil.copytree(sysroot, source / "flatpak-rust", symlinks=False)
    config = output("rustup", "run", pin, "cargo", "vendor", "--locked", "cargo-vendor", cwd=source)
    with (source / ".cargo/config.toml").open("a") as stream:
        stream.write("\n" + config + "\n")
    (destination / "org.serein.desktop.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="New build directory; existing paths are refused")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--repo-url", default=DEFAULT_REPO_URL,
                        help="Hosted OSTree repository URL for the generated .flatpakref")
    args = parser.parse_args()
    destination = args.destination.resolve()
    prepare(destination)
    if not args.prepare_only:
        subprocess.run(["flatpak-builder", "--user", "--repo=repo", "build",
                        "org.serein.desktop.json"], cwd=destination, check=True)
        subprocess.run(["flatpak", "build-update-repo", "--generate-static-deltas", "repo"],
                       cwd=destination, check=True)
        subprocess.run(["flatpak", "build-bundle", "--runtime-repo=https://flathub.org/repo/flathub.flatpakrepo",
                        "repo", "Serein-linux.flatpak", "org.serein.desktop"], cwd=destination, check=True)
        (destination / "serein.flatpakref").write_text(generate_flatpakref(args.repo_url))


if __name__ == "__main__":
    main()
