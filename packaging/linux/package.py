"""Build and inspect unsigned native Linux packages from xtask's release files.

No installation or application launch. Uses only Python's standard library and
distribution packaging/desktop tools; run through cargo xtask package.
"""

import filecmp
import argparse
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile


def output(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def checked(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def copy(source, destination, manifest=None):
    manifest = source if manifest is None else manifest
    if source.is_symlink() or manifest.is_symlink():
        raise ValueError(f"Refusing symlink in package input: {source}")
    if source.is_dir():
        destination.mkdir(parents=True, exist_ok=True)
        for child in sorted(manifest.iterdir()):
            copy(source / child.name, destination / child.name, child)
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)


def payload_files(root):
    return sorted(p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file())


def stage_payload(root, stage, prefix="usr"):
    """Copy the shared, allowlisted native/Flatpak installation payload."""
    if prefix not in {"usr", "app"}:
        raise ValueError("Installation prefix must be usr or app")
    doc = stage / prefix / "share/doc/serein"
    copy(root / "serein", stage / prefix / "bin/serein")
    desktop = stage / prefix / "share/applications/org.serein.desktop.desktop"
    desktop.parent.mkdir(parents=True)
    desktop.write_text(Path("packaging/linux/serein.desktop").read_text(), encoding="utf-8")
    copy(Path("packaging/linux/hicolor"), stage / prefix / "share/icons/hicolor")
    for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"]:
        copy(root / name, doc / name)
    for name in ["NotoSansCJK-LICENSE.txt", "NotoSansArabic-OFL.txt", "Inter-OFL.txt",
                 "Twemoji-CC-BY-4.0.txt", "Unicode-LICENSE.txt", "Phosphor-Icons-MIT.txt",
                 "Simple-Icons-CC0.txt"]:
        copy(root / "licenses" / name, doc / "licenses" / name)
    for name in ["files", "notifications", "login", "audio", "voice", "dependencies"]:
        copy(root / "licenses" / name, doc / "licenses" / name, Path("assets/licenses") / name)
    for path in [stage, *stage.rglob("*")]:
        path.chmod(0o755 if path.is_dir() or path == stage / prefix / "bin/serein" else 0o644)


def smoke(package, stage, temporary, version, architecture, depends):
    fields = {
        "Package": "serein", "Version": version, "Architecture": architecture,
        "Depends": depends,
    }
    for field, expected in fields.items():
        actual = output("dpkg-deb", "--field", str(package), field)
        if actual != expected:
            raise ValueError(f"Package {field}: expected {expected!r}, got {actual!r}")
    # Inspect before extraction; never extract links, device nodes or traversal paths.
    archive = temporary / "payload.tar"
    with archive.open("wb") as stream:
        subprocess.run(["dpkg-deb", "--fsys-tarfile", str(package)], stdout=stream, check=True)
    with tarfile.open(archive) as contents:
        for member in contents:
            path = Path(member.name)
            expected_mode = 0o755 if member.isdir() or member.name == "./usr/bin/serein" else 0o644
            if (path.is_absolute() or ".." in path.parts
                    or not (member.isfile() or member.isdir())
                    or member.mode != expected_mode or member.uid or member.gid):
                raise ValueError(f"Unsafe package entry or incorrect mode/owner: {member.name}")
    extracted = temporary / "extracted"
    checked("dpkg-deb", "--extract", str(package), str(extracted))
    expected = [name for name in payload_files(stage) if not name.startswith("DEBIAN/")]
    if payload_files(extracted) != expected:
        raise ValueError("Package payload differs from staged allowlist")
    for name in expected:
        if not filecmp.cmp(stage / name, extracted / name, shallow=False):
            raise ValueError(f"Package changed file contents: {name}")
    control = temporary / "extracted-control"
    checked("dpkg-deb", "--control", str(package), str(control))
    if payload_files(control) != ["control"]:
        raise ValueError("Unexpected control files or maintainer scripts")
    checked("desktop-file-validate", str(extracted / "usr/share/applications/org.serein.desktop.desktop"))
    libraries = output("ldd", str(extracted / "usr/bin/serein"))
    if "not found" in libraries:
        raise ValueError(f"Unresolved packaged executable dependencies:\n{libraries}")
    print(f"Debian package smoke passed: {len(expected)} files; executable, desktop entry, "
          "metadata, ownership, content and host shared-library closure verified.")


def package(root, application_version):
    if sys.platform != "linux":
        raise ValueError("Debian packaging must run natively on Debian/Ubuntu Linux")
    architecture = output("dpkg", "--print-architecture")
    machine = {"amd64": 62, "arm64": 183}.get(architecture)
    if machine is None:
        raise ValueError(f"Unverified Debian package architecture: {architecture}")
    # Rust and dpkg must agree: do not label a cross-built binary as the host arch.
    with (root / "serein").open("rb") as executable:
        header = executable.read(20)
    if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
            or struct.unpack("<H", header[18:20])[0] != machine):
        raise ValueError("Expected a native little-endian 64-bit ELF executable")
    version = application_version.replace("-", "~", 1) + "-1"
    checked("dpkg", "--validate-version", version)
    # A fresh owned directory prevents stale files, logs or credentials in dist
    # from entering a package. TemporaryDirectory removes only this invocation.
    with tempfile.TemporaryDirectory(prefix="serein-debian-package-") as directory:
        temporary = Path(directory).resolve()
        stage = temporary / "debian/serein"
        stage_payload(root, stage)
        debian = temporary / "debian"
        debian.mkdir(exist_ok=True)
        (debian / "control").write_text(
            "Source: serein\nSection: net\nPriority: optional\n"
            "Maintainer: Serein contributors <noreply@github.com>\n\n"
            "Package: serein\nArchitecture: any\nDescription: Unofficial native Discord client\n",
            encoding="utf-8")
        (stage / "DEBIAN").mkdir()
        dependencies = output("dpkg-shlibdeps", "-O", "debian/serein/usr/bin/serein", cwd=temporary)
        depends = next(line.removeprefix("shlibs:Depends=") for line in dependencies.splitlines()
                       if line.startswith("shlibs:Depends="))
        # dlopen libraries and desktop services are invisible to ELF DT_NEEDED.
        depends += (", libvulkan1, libegl1, libxkbcommon0, libxkbcommon-x11-0, "
                    "libwayland-client0, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxrandr2, "
                    "dbus-user-session | dbus-x11, xdg-desktop-portal, gstreamer1.0-plugins-good")
        installed_kib = sum(1 if p.is_dir() else max(1, (p.stat().st_size + 1023) // 1024)
                            for p in stage.rglob("*"))
        control = stage / "DEBIAN/control"
        control.write_text(
            f"Package: serein\nVersion: {version}\nArchitecture: {architecture}\n"
            "Section: net\nPriority: optional\n"
            "Maintainer: Serein contributors <noreply@github.com>\n"
            "Homepage: https://github.com/ViceVerse-cz/rustcord\n"
            f"Installed-Size: {installed_kib}\nDepends: {depends}\n"
            "Recommends: gnome-keyring, xdg-desktop-portal-gtk | xdg-desktop-portal-kde, "
            "gstreamer1.0-plugins-base, gstreamer1.0-libav\n"
            "Description: Unofficial native Discord client\n"
            " Native Rust desktop client for existing Discord accounts.\n"
            " Unofficial, experimental, and not endorsed by Discord.\n",
            encoding="utf-8")
        for path in [stage, *stage.rglob("*")]:
            path.chmod(0o755 if path.is_dir() or path == stage / "usr/bin/serein" else 0o644)
        checked("desktop-file-validate", str(stage / "usr/share/applications/org.serein.desktop.desktop"))
        artifact = root / f"serein_{version}_{architecture}.deb"
        candidate = temporary / artifact.name
        checked("dpkg-deb", "--root-owner-group", "-Zxz", "--build", str(stage), str(candidate))
        smoke(candidate, stage, temporary, version, architecture, depends)
        shutil.copyfile(candidate, artifact)
        print(f"Unsigned Debian package: {artifact} ({artifact.stat().st_size} bytes; "
              f"Installed-Size {installed_kib} KiB).")
        print(f"Runtime dependencies: {depends}; recommends a Secret Service provider "
              "(gnome-keyring) and GStreamer plugins for inline attachment video.")


def native_elf(root):
    machine = {"x86_64": 62, "aarch64": 183}.get(platform.machine())
    with (root / "serein").open("rb") as stream:
        header = stream.read(20)
    if (sys.platform != "linux" or machine is None or len(header) != 20
            or header[:6] != b"\x7fELF\x02\x01"
            or struct.unpack("<H", header[18:20])[0] != machine):
        raise ValueError("Expected a native little-endian 64-bit ELF executable")


def check_payload(stage, extracted):
    expected = payload_files(stage)
    if payload_files(extracted) != expected:
        raise ValueError("Package payload differs from staged allowlist")
    for name in expected:
        path = extracted / name
        if path.is_symlink() or not filecmp.cmp(stage / name, path, shallow=False):
            raise ValueError(f"Package changed file contents: {name}")


def native_package(root, application_version, format):
    native_elf(root)
    # Versions enter native metadata and shell build recipes: accept semver only.
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", application_version):
        raise ValueError("Expected a semantic application version")
    if format == "dir":
        destination = root / "linux-root"
        # Never recursively delete a previous tree supplied by another invocation.
        if destination.exists() or destination.is_symlink():
            raise ValueError(f"Staging destination already exists; move it aside first: {destination}")
        stage_payload(root, destination)
        print(f"Linux installation tree: {destination}")
        return
    distro = platform.freedesktop_os_release()["ID"]
    if (format == "rpm" and distro not in {"fedora", "opensuse-tumbleweed"}
            or format == "arch" and distro != "arch"):
        raise ValueError(f"Build {format} natively on its supported distribution, not {distro}")
    if format == "arch" and os.getuid() == 0:
        raise ValueError("Run Arch builds as an unprivileged user; makepkg refuses root")
    with tempfile.TemporaryDirectory(prefix="serein-linux-package-") as directory:
        temporary = Path(directory).resolve()
        stage = temporary / "payload"
        stage_payload(root, stage)
        checked("desktop-file-validate", str(stage / "usr/share/applications/org.serein.desktop.desktop"))
        libraries = output("ldd", str(stage / "usr/bin/serein"))
        if "not found" in libraries:
            raise ValueError(f"Unresolved packaged executable dependencies:\n{libraries}")
        if format == "rpm":
            artifact = rpm_package(temporary, stage, application_version, distro)
        else:
            artifact = arch_package(temporary, stage, application_version, libraries)
        destination = root / artifact.name
        shutil.copyfile(artifact, destination)
        print(f"Unsigned native {distro} package: {destination} ({destination.stat().st_size} bytes)")


def rpm_package(temporary, stage, application_version, distro):
    version = application_version.replace("-", "~", 1).replace("-", ".")
    release = "1.fc" + platform.freedesktop_os_release()["VERSION_ID"] if distro == "fedora" else "1.suse"
    if not re.fullmatch(r"1\.(?:fc[0-9]+|suse)", release):
        raise ValueError("Unsupported RPM distribution version")
    # SONAME capabilities work with both Fedora and openSUSE package naming.
    # rpmbuild adds the executable's actual ELF dependencies automatically.
    requires = [f"{soname}()(64bit)" for soname in [
        "libvulkan.so.1", "libEGL.so.1", "libxkbcommon.so.0", "libxkbcommon-x11.so.0",
        "libwayland-client.so.0", "libX11.so.6", "libX11-xcb.so.1", "libXcursor.so.1",
        "libXi.so.6", "libXrandr.so.2"]] + [
            "dbus" if distro == "fedora" else "dbus-1", "xdg-desktop-portal",
            "gstreamer1-plugins-good" if distro == "fedora" else "gstreamer-plugins-good"]
    plugins = "gstreamer1-plugins-base" if distro == "fedora" else "gstreamer-plugins-base"
    spec = temporary / "serein.spec"
    spec.write_text(
        "%global debug_package %{nil}\n%global __os_install_post %{nil}\n"
        "%global _build_id_links none\n"
        f"Name: serein\nVersion: {version}\nRelease: {release}\n"
        "Summary: Unofficial native Discord client\nLicense: MIT OR Apache-2.0\n"
        "URL: https://github.com/ViceVerse-cz/Serein\n"
        + "\n".join(f"Requires: {item}" for item in requires)
        + f"\nRecommends: gnome-keyring, {plugins}\n"
        "\n%description\nNative Rust client for existing Discord accounts, including voice.\n"
        "Unofficial, experimental, and not endorsed by Discord.\n"
        "\n%install\nmkdir -p %{buildroot}\ncp -a %{_serein_payload}/. %{buildroot}/\n"
        "\n%files\n%defattr(-,root,root,-)\n/usr/bin/serein\n"
        "/usr/share/applications/org.serein.desktop.desktop\n/usr/share/doc/serein\n"
        + "".join(f"/{name}\n" for name in payload_files(stage) if name.startswith("usr/share/icons/")),
        encoding="utf-8")
    # Paths are passed as RPM macro values; reject macro/shell metacharacters.
    if not re.fullmatch(r"[/A-Za-z0-9_.-]+", str(temporary)):
        raise ValueError("RPM temporary directory must have a simple absolute path")
    checked("rpmbuild", "-bb", "--define", f"_topdir {temporary}/rpm",
            "--define", f"_serein_payload {stage}", str(spec))
    artifact, = (temporary / "rpm/RPMS").rglob("*.rpm")
    actual = output("rpm", "-qp", "--qf", "%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}", str(artifact))
    if actual != f"serein\n{version}\n{release}\n{platform.machine()}":
        raise ValueError(f"Incorrect RPM metadata: {actual}")
    if output("rpm", "-qp", "--scripts", str(artifact)):
        raise ValueError("Unexpected RPM install scripts")
    listing = output("rpm", "-qp", "--qf", "[%{FILENAMES}\t%{FILEMODES:octal}\t%{FILEUSERNAME}\t%{FILEGROUPNAME}\n]", str(artifact))
    for line in listing.splitlines():
        name, mode, user, group = line.split("\t")
        path = stage / name.removeprefix("/")
        expected_mode = 0o40755 if path.is_dir() else 0o100755 if name == "/usr/bin/serein" else 0o100644
        if (".." in Path(name).parts or not path.exists()
                or int(mode, 8) != expected_mode or user != "root" or group != "root"):
            raise ValueError(f"Unexpected RPM payload entry: {line}")
    archive = temporary / "payload.cpio"
    with archive.open("wb") as stream:
        subprocess.run(["rpm2cpio", str(artifact)], stdout=stream, check=True)
    extracted = temporary / "extracted"
    extracted.mkdir()
    with archive.open("rb") as stream:
        subprocess.run(["cpio", "-id", "--no-absolute-filenames"], stdin=stream, cwd=extracted, check=True)
    check_payload(stage, extracted)
    actual_requires = output("rpm", "-qp", "--requires", str(artifact)).splitlines()
    if not set(requires).issubset(actual_requires):
        raise ValueError("Missing RPM runtime dependencies")
    print("RPM smoke passed: metadata, dependency capabilities, payload contents, modes, owners and no scripts.")
    return artifact


def arch_package(temporary, stage, application_version, libraries):
    # pacman's letter suffix sorts below the final version; '_' sorts above it.
    version = application_version.replace("-", "pre.", 1).replace("-", ".")
    depends = {"vulkan-icd-loader", "libglvnd", "libxkbcommon", "libxkbcommon-x11",
               "wayland", "libx11", "libxcursor", "libxi", "libxrandr", "dbus", "xdg-desktop-portal",
               "gst-plugins-good"}
    # Resolve linked libraries to the native pacman package/version (ABI floor).
    for path in re.findall(r"(?:=>\s+|^\s*)(/\S+)", libraries, re.MULTILINE):
        owner = output("pacman", "-Qqo", path)
        name, installed_version = output("pacman", "-Q", owner).split()
        if name in {"zlib", "zlib-ng-compat"} or Path(path).name.startswith("libz.so"):
            depends.add("libz.so")
            continue
        depends.add(f"{name}>={installed_version}")
    if any(not re.fullmatch(r"[A-Za-z0-9@._+:>=-]+", item) for item in depends):
        raise ValueError("Invalid native Arch dependency metadata")
    (temporary / "PKGBUILD").write_text(
        f"pkgname=serein\npkgver='{version}'\npkgrel=1\n"
        "pkgdesc='Unofficial native Discord client'\n"
        f"arch=('{platform.machine()}')\nurl='https://github.com/ViceVerse-cz/Serein'\n"
        "license=('MIT' 'Apache-2.0')\noptions=('!strip' '!debug' '!lto')\n"
        + "depends=(" + " ".join(f"'{item}'" for item in sorted(depends)) + ")\n"
        "optdepends=('gnome-keyring: Secret Service credential provider' "
        "'gst-plugins-base: inline video' 'gst-libav: inline video')\n"
        "package() { cp -a \"$startdir/payload/.\" \"$pkgdir/\"; }\n", encoding="utf-8")
    checked("makepkg", "--nodeps", "--noconfirm", cwd=temporary)
    artifact, = temporary.glob("serein-*.pkg.tar.*")
    archive = temporary / "payload.tar"
    with archive.open("wb") as stream:
        subprocess.run(["bsdtar", "-cf", "-", "--format=ustar", "@" + str(artifact)], stdout=stream, check=True)
    extracted = temporary / "extracted"
    extracted.mkdir()
    with tarfile.open(archive) as contents:
        metadata = contents.extractfile(".PKGINFO").read().decode()
        for item in [f"pkgname = serein", f"pkgver = {version}-1", f"arch = {platform.machine()}",
                     *(f"depend = {dependency}" for dependency in depends)]:
            if item not in metadata.splitlines():
                raise ValueError(f"Missing Arch metadata: {item}")
        for member in contents:
            path = Path(member.name)
            if (path.is_absolute() or ".." in path.parts
                    or not (member.isfile() or member.isdir()) or member.uid or member.gid):
                raise ValueError(f"Unsafe Arch archive member: {member.name}")
            if member.name in {".PKGINFO", ".BUILDINFO", ".MTREE"}:
                continue
            expected_mode = 0o755 if member.isdir() or member.name == "usr/bin/serein" else 0o644
            if member.mode != expected_mode:
                raise ValueError(f"Incorrect Arch payload mode: {member.name}")
            contents.extract(member, extracted, filter="data")
    check_payload(stage, extracted)
    print("Arch smoke passed: metadata, dependency versions, payload contents, modes, owners and no scripts.")
    return artifact


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("staged_directory", type=Path)
    parser.add_argument("application_version")
    parser.add_argument("--format", choices=["deb", "rpm", "arch", "dir"], default="deb")
    arguments = parser.parse_args()
    root = arguments.staged_directory.resolve()
    if arguments.format == "deb":
        package(root, arguments.application_version)
    else:
        native_package(root, arguments.application_version, arguments.format)
