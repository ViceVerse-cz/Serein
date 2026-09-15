#!/usr/bin/env python3
"""Prepare one signed, static package repository; never upload or install anything."""

import argparse
from datetime import datetime, timedelta, timezone
from email.utils import format_datetime
import gzip
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from urllib.parse import urlsplit


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True)


def validate(args):
    for value in (args.distribution, args.architecture):
        if not re.fullmatch(r"[a-z0-9][a-z0-9_.-]{0,63}", value):
            raise ValueError("invalid distribution or architecture")
    if not re.fullmatch(r"[A-Fa-f0-9]{40}|[A-Fa-f0-9]{64}", args.key):
        raise ValueError("--key must be a full GPG fingerprint")
    url = urlsplit(args.base_url)
    if (url.scheme != "https" or not url.hostname or url.username or url.password
            or url.query or url.fragment or any(c.isspace() for c in args.base_url)):
        raise ValueError("--base-url must be an HTTPS URL without credentials/query/fragment")


def sign(path, key, clear=False, armor=False):
    output = path.with_name("InRelease") if clear else Path(str(path) + ".sig")
    run("gpg", "--batch", "--yes", "--local-user", key, "--output", str(output),
        *(["--armor"] if armor else []), "--clearsign" if clear else "--detach-sign", str(path))
    run("gpg", "--batch", "--verify", str(output), *([] if clear else [str(path)]))
    return output


def build(args):
    validate(args)
    suffix = {"deb": ".deb", "rpm": ".rpm", "arch": ".pkg.tar.zst"}[args.format]
    packages = sorted(args.input.glob("*" + suffix))
    if not packages or len(packages) > 100:
        raise ValueError("input must contain 1–100 packages of the selected format")
    for package in packages:
        if (package.is_symlink() or not package.is_file()
                or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.+~-]*", package.name)
                or package.stat().st_size > 2 * 1024**3):
            raise ValueError(f"unsafe or oversized package: {package}")
    kind = "apt" if args.format == "deb" else args.format
    relative = Path(args.channel) / args.distribution / args.architecture / kind
    destination = args.output.resolve() / relative
    if destination.exists():
        raise ValueError(f"output already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        stage = Path(temporary) / kind
        stage.mkdir()
        keyfile = stage / "serein.asc"
        keyfile.write_text(run("gpg", "--batch", "--armor", "--export", args.key))
        if not keyfile.stat().st_size:
            raise ValueError("signing public key not found")
        copied = []
        for package in packages:
            target = stage / package.name
            shutil.copyfile(package, target)
            if args.format == "deb":
                identity = run("dpkg-deb", "--field", str(target), "Package").strip()
                arch = run("dpkg-deb", "--field", str(target), "Architecture").strip()
            elif args.format == "rpm":
                identity, arch = run("rpm", "-qp", "--qf", "%{NAME} %{ARCH}", str(target)).split()
            else:
                metadata = run("bsdtar", "-xOf", str(target), ".PKGINFO")
                values = dict(line.split(" = ", 1) for line in metadata.splitlines() if " = " in line)
                identity, arch = values.get("pkgname"), values.get("arch")
            if identity != "serein" or arch != args.architecture:
                raise ValueError(f"unexpected package identity/architecture: {package}")
            copied.append(target)
        if args.format == "deb":
            index = run("apt-ftparchive", "packages", ".", cwd=stage).encode()
            (stage / "Packages").write_bytes(index)
            (stage / "Packages.gz").write_bytes(gzip.compress(index, mtime=0))
            release = run("apt-ftparchive", "-o", "APT::FTPArchive::Release::Origin=Serein",
                          "-o", f"APT::FTPArchive::Release::Architectures={args.architecture}",
                          "-o", f"APT::FTPArchive::Release::Suite={args.channel}", "release", ".", cwd=stage)
            expiry = format_datetime(datetime.now(timezone.utc) + timedelta(days=30), usegmt=True)
            (stage / "Release").write_text(release + f"Valid-Until: {expiry}\n")
            sign(stage / "Release", args.key, clear=True)
            sign(stage / "Release", args.key).rename(stage / "Release.gpg")
        elif args.format == "rpm":
            with tempfile.TemporaryDirectory() as database:
                run("rpm", "--dbpath", database, "--import", str(keyfile))
                for package in copied:
                    run("rpmsign", "--define", f"_gpg_name {args.key}",
                        "--define", f"_openpgp_sign_id {args.key}", "--addsign", str(package))
                    verification = run("rpmkeys", "--dbpath", database, "--checksig", str(package))
                    if "signatures OK" not in verification:
                        raise ValueError(f"RPM signature not verified: {verification}")
            run("createrepo_c", str(stage))
            sign(stage / "repodata/repomd.xml", args.key, armor=True).rename(stage / "repodata/repomd.xml.asc")
        else:
            for package in copied:
                sign(package, args.key)
            run("repo-add", "--sign", "--key", args.key, "serein.db.tar.gz",
                *(p.name for p in copied), cwd=stage)
            run("gpg", "--batch", "--verify", str(stage / "serein.db.tar.gz.sig"),
                str(stage / "serein.db.tar.gz"))
            # Static hosts/artifact uploads often discard symlinks; publish actual aliases.
            for alias in stage.iterdir():
                if alias.is_symlink():
                    content = alias.read_bytes()
                    alias.unlink()
                    alias.write_bytes(content)
        (stage / "key-fingerprint.txt").write_text(args.key.upper() + "\n")
        url = args.base_url.rstrip("/") + "/" + relative.as_posix()
        if args.format == "rpm":
            (stage / "serein.repo").write_text(
                f"[serein-{args.channel}]\nname=Serein {args.channel}\nbaseurl={url}\n"
                f"enabled=1\ngpgcheck=1\nrepo_gpgcheck=1\ngpgkey={url}/serein.asc\n")
        stage.chmod(0o755)
        for path in stage.rglob("*"):
            path.chmod(0o755 if path.is_dir() else 0o644)
        stage.rename(destination)
    print(destination)




def generate_index(destination: Path):
    html = """<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Serein Linux Repositories</title>
  <meta http-equiv="refresh" content="0; url=https://github.com/ViceVerse-cz/Serein">
  <style>
    body { font-family: system-ui, -apple-system, sans-serif; background: #111214; color: #dbdee1; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
    .card { background: #2b2d31; padding: 2rem; border-radius: 8px; text-align: center; max-width: 480px; box-shadow: 0 8px 24px rgba(0,0,0,0.4); }
    h1 { margin-top: 0; color: #fff; }
    a { color: #5865f2; text-decoration: none; font-weight: bold; }
    a:hover { text-decoration: underline; }
    code { background: #1e1f22; padding: 0.2rem 0.4rem; border-radius: 4px; font-size: 0.9em; }
  </style>
</head>
<body>
  <div class="card">
    <h1>Serein Linux Repositories</h1>
    <p>Signed native packages and Flatpak repository for Serein.</p>
    <p>Run <code>curl -fsSL https://viceverse-cz.github.io/Serein/setup.sh | sh</code> to install.</p>
    <p><a href="https://github.com/ViceVerse-cz/Serein">View project on GitHub &rarr;</a></p>
  </div>
</body>
</html>
"""
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "index.html").write_text(html)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generate-index", type=Path, help="Generate landing index.html into directory")
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--channel", choices=("nightly", "production"))
    parser.add_argument("--distribution")
    parser.add_argument("--format", choices=("deb", "rpm", "arch"))
    parser.add_argument("--architecture")
    parser.add_argument("--key")
    parser.add_argument("--base-url")
    args = parser.parse_args()
    if args.generate_index:
        generate_index(args.generate_index)
        return
    if not (args.input and args.output and args.channel and args.distribution and args.format and args.architecture and args.key and args.base_url):
        parser.error("missing required repository build arguments")
    build(args)


if __name__ == "__main__":
    main()
