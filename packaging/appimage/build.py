"""Package the native Linux payload as a portable Anylinux DwarFS Type 2 AppImage; never launch Serein."""

import argparse
import filecmp
import hashlib
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

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "linux"))
from package import native_elf, stage_payload

PINNED_TOOLS = {
    "quick-sharun.sh": (
        "b3c561d35b08afc8694414f710c2074f76e2c1e19db3cefa43e06604acc55834",
        True,
    ),
    "sharun+helper-libs-x86_64.tar": (
        "cc820113162cd8a56ccc66a85b638d9c4fc3802819327719323911adc9311b2b",
        False,
    ),
    "cross-libc-dlopen-x86_64.tar": (
        "35bd45e4d0ce94e8ac86b2dded676f4a371747d0f47004861d4c25f374351ec8",
        False,
    ),
    "appimagetool": (
        "b8bc7a63782978a8aebc0b7012ee8962eba69e05d0dfc8289b508d463c761887",
        True,
    ),
    "runtime-x86_64": (
        "649948f2bf4c012fa909f0b746949f22e97fd4a9b86ae1aea0512b70813a9248",
        True,
    ),
    "bin/mkdwarfs": (
        "50891c38ba359db8271819a6cbf6aaa8068681523f0c4f2b8242007a45edaa28",
        True,
    ),
}

REQUIRED_HOST_TOOLS = (
    "patchelf",
    "desktop-file-validate",
    "file",
    "ldd",
    "tar",
    "awk",
    "grep",
    "sed",
    "find",
)

CORE_GSTREAMER_PLUGINS = (
    "libgstcoreelements.so",
    "libgstplayback.so",
    "libgsttypefindfunctions.so",
    "libgstaudioconvert.so",
    "libgstvideoconvertscale.so",
    "libgstautodetect.so",
    "libgstapp.so",
    "libgstpulseaudio.so",
)

CORE_GLYCIN_LOADERS = (
    "glycin-image-rs",
    "glycin-svg",
)

FORBIDDEN_ENV_TOKENS = (
    "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS",
    "ELECTRON_DISABLE_SANDBOX",
    "QTWEBENGINE_DISABLE_SANDBOX",
    "GSK_RENDERER=cairo",
    "GDK_DISABLE=gl,vulkan",
    "GDK_GL=disable",
)

MAX_IMAGE_BYTES = 512 * 1024 * 1024
MAX_ZSYNC_BYTES = 16 * 1024 * 1024
DWARFS_COMPRESSION = "zstd:level=22 -S25 -B2"


def update_metadata(version, release_tag):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ValueError("Expected a semantic application version")
    if release_tag and release_tag != f"v{version}":
        raise ValueError("AppImage release tag must match the application version")
    name = f"serein-{release_tag or version}-Linux-X64.AppImage"
    channel = "latest-pre" if "-" in version.split("+", 1)[0] else "latest"
    information = f"gh-releases-zsync|ViceVerse-cz|Serein|{channel}|serein-*-Linux-X64.AppImage.zsync"
    url = f"https://github.com/ViceVerse-cz/Serein/releases/download/v{version}/serein-v{version}-Linux-X64.AppImage"
    return name, information, url


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while True:
            chunk = stream.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def verify_pinned_tools(tools_dir):
    if not tools_dir.is_dir():
        raise ValueError("Run bash packaging/appimage/install-tools.sh first")
    for relative, (expected_sha, must_execute) in PINNED_TOOLS.items():
        path = tools_dir / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"Missing pinned AppImage tool '{relative}'; run bash packaging/appimage/install-tools.sh first")
        actual_sha = sha256_file(path)
        if actual_sha != expected_sha:
            raise ValueError(
                f"Checksum mismatch for '{relative}': expected {expected_sha}, got {actual_sha}; "
                "re-run bash packaging/appimage/install-tools.sh"
            )
        if must_execute:
            path.chmod(0o755)


def verify_host_tools():
    for tool in REQUIRED_HOST_TOOLS:
        if not shutil.which(tool):
            raise ValueError(f"Required host packaging tool '{tool}' was not found in PATH")


def extract_tar_member(tar_path, member_name, destination, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(tar_path, "r") as archive:
        member = archive.getmember(member_name)
        if not member.isfile():
            raise ValueError(f"Expected regular file '{member_name}' inside {tar_path.name}")
        source = archive.extractfile(member)
        if source is None:
            raise ValueError(f"Unable to read '{member_name}' from {tar_path.name}")
        with source, destination.open("wb") as target:
            shutil.copyfileobj(source, target)
    destination.chmod(mode)


def is_elf_file(path):
    if path.is_symlink() or not path.is_file():
        return False
    with path.open("rb") as stream:
        return stream.read(4) == b"\x7fELF"


def should_deploy_pipewire(libraries):
    if "libgstreamer" not in libraries:
        return False
    lib_roots = (Path("/usr/lib"), Path("/usr/lib64"), Path("/usr/lib/x86_64-linux-gnu"))
    has_lib = any(root.is_dir() and any(root.glob("libpipewire-0.3.so*")) for root in lib_roots)
    has_plugin = any((root / "gstreamer-1.0/libgstpipewire.so").is_file() for root in lib_roots)
    return has_lib and has_plugin


def discover_webkit_binaries(libraries):
    match = re.search(r"=>\s+(/\S*/libwebkit[^\s/]*\.so\S*)", libraries)
    if not match:
        return []
    lib_path = Path(match.group(1)).resolve()
    directory_name = lib_path.name.removeprefix("lib").split(".so", 1)[0]
    candidate_dir = lib_path.parent / directory_name
    if not candidate_dir.is_dir():
        libexec_dir = Path("/usr/libexec") / directory_name
        if libexec_dir.is_dir():
            candidate_dir = libexec_dir
    if not candidate_dir.is_dir():
        raise ValueError(f"Linked WebKit library {lib_path} is missing helper directory {candidate_dir}")
    binaries = []
    for entry in sorted(candidate_dir.iterdir()):
        if entry.name.endswith(".so") or ".so." in entry.name:
            continue
        if os.access(entry, os.X_OK) and is_elf_file(entry):
            binaries.append(str(entry))
    return binaries


def bundled_library_names(appdir):
    names = set()
    lib_root = appdir / "lib"
    if not lib_root.is_dir():
        return names
    for root_dir, _, filenames in os.walk(lib_root):
        for filename in filenames:
            candidate = Path(root_dir) / filename
            if not candidate.exists():
                continue
            if filename.endswith(".so") or ".so." in filename:
                names.add(filename)
    return names


def elf_needed(path):
    result = subprocess.run(
        ["patchelf", "--print-needed", str(path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"Failed to inspect ELF dependencies of {path}: {result.stderr.strip()}")
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def prune_broken_gstreamer_plugins(appdir, require_gstreamer, require_pipewire):
    lib_root = appdir / "lib"
    gst_dirs = sorted(p for p in lib_root.glob("gstreamer-*") if p.is_dir() and not p.is_symlink())
    if require_gstreamer and not gst_dirs:
        raise ValueError("Serein links libgstreamer, but quick-sharun did not deploy any gstreamer-* directory")
    for gst_dir in gst_dirs:
        changed = True
        while changed:
            changed = False
            available = bundled_library_names(appdir)
            for entry in sorted(gst_dir.iterdir()):
                if entry.is_symlink():
                    if not entry.exists():
                        entry.unlink()
                        changed = True
                    continue
                if not entry.is_file() or (not entry.name.endswith(".so") and ".so." not in entry.name):
                    continue
                if not is_elf_file(entry):
                    continue
                needed = elf_needed(entry)
                if any(dep not in available for dep in needed):
                    entry.unlink()
                    changed = True
    if not require_gstreamer:
        return
    present = {
        entry.name
        for gst_dir in gst_dirs
        for entry in gst_dir.iterdir()
        if entry.exists() and (entry.name.endswith(".so") or ".so." in entry.name)
    }
    required = list(CORE_GSTREAMER_PLUGINS)
    if require_pipewire:
        required.append("libgstpipewire.so")
    missing = [name for name in required if name not in present]
    if missing:
        raise ValueError(
            "Required GStreamer plugins are missing or had unresolved dependencies: "
            + ", ".join(missing)
        )


def prune_broken_glycin_loaders(appdir):
    glycin_conf_root = appdir / "share/glycin-loaders"
    if not glycin_conf_root.is_dir():
        return
    conf_files = sorted(
        p for p in glycin_conf_root.rglob("*.conf") if p.parent.name == "conf.d"
    )
    available = bundled_library_names(appdir)
    glycin_lib_root = appdir / "lib/glycin-loaders"
    for conf in conf_files:
        stem = conf.stem
        if not stem.startswith("glycin-"):
            continue
        candidate = appdir / "shared/bin" / stem
        broken = False
        if not candidate.exists() and not candidate.is_symlink():
            broken = True
        elif candidate.is_symlink() and not candidate.exists():
            broken = True
        else:
            resolved = candidate.resolve() if candidate.is_symlink() else candidate
            if is_elf_file(resolved) and any(dep not in available for dep in elf_needed(resolved)):
                broken = True
        if not broken:
            continue
        if candidate.exists() or candidate.is_symlink():
            candidate.unlink()
        bin_wrapper = appdir / "bin" / stem
        if bin_wrapper.exists() or bin_wrapper.is_symlink():
            bin_wrapper.unlink()
        if glycin_lib_root.is_dir():
            for nested in sorted(glycin_lib_root.rglob(stem)):
                if nested.is_file() or nested.is_symlink():
                    nested.unlink()
        conf.unlink()

    present_confs = {
        p.stem for p in glycin_conf_root.rglob("*.conf") if p.parent.name == "conf.d"
    }
    missing = [
        name
        for name in CORE_GLYCIN_LOADERS
        if name not in present_confs
        or not (appdir / "shared/bin" / name).is_file()
        or not (appdir / "bin" / name).is_file()
    ]
    if missing:
        raise ValueError(
            "Required Glycin loaders are missing or had unresolved dependencies: "
            + ", ".join(missing)
        )


def verify_elf_closure(appdir):
    available = bundled_library_names(appdir)
    sharun_path = appdir / "sharun"
    if sharun_path.is_symlink() or not sharun_path.is_file():
        raise ValueError("Expected regular 'sharun' binary inside AppDir")
    sharun_stat = sharun_path.stat()
    sharun_identity = (sharun_stat.st_dev, sharun_stat.st_ino)
    failures = []
    for sub in ("shared/bin", "lib"):
        base = appdir / sub
        if not base.is_dir():
            continue
        for root_dir, _, filenames in os.walk(base):
            for filename in sorted(filenames):
                candidate = Path(root_dir) / filename
                if not is_elf_file(candidate):
                    continue
                candidate_stat = candidate.stat()
                if (candidate_stat.st_dev, candidate_stat.st_ino) == sharun_identity:
                    if sub != "lib" or not (appdir / "shared/bin" / candidate.name).is_file():
                        failures.append(
                            f"{candidate.relative_to(appdir)}: broken or misplaced sharun hardlink"
                        )
                    continue
                needed = elf_needed(candidate)
                missing = [dep for dep in needed if dep not in available]
                if missing:
                    failures.append(f"{candidate.relative_to(appdir)}: {', '.join(missing)}")
    if failures:
        raise ValueError("Unresolved bundled ELF dependencies inside AppDir:\n" + "\n".join(failures))


def verify_appdir_invariants(appdir):
    for required in ("AppRun", "AppRun.sh", "sharun", "dash", "bin/serein", "shared/bin/serein"):
        path = appdir / required
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"Expected executable '{required}' inside AppDir")
    if (appdir / "usr/bin/serein").exists() or (appdir / "usr/bin/serein").is_symlink():
        raise ValueError("Unexpected redundant usr/bin/serein inside Anylinux AppDir")
    stubs_dir = appdir / "lib/sharun-preload/stubs"
    if stubs_dir.exists() or stubs_dir.is_symlink():
        raise ValueError("Unexpected unconsumed quick-sharun stubs directory inside AppDir")
    env_file = appdir / ".env"
    if env_file.is_file():
        env_text = env_file.read_text(encoding="utf-8", errors="replace")
        for forbidden in FORBIDDEN_ENV_TOKENS:
            if forbidden in env_text:
                raise ValueError(f"AppDir .env must not contain '{forbidden}'")
    leaked_drivers = []
    for root_dir, _, filenames in os.walk(appdir):
        for filename in sorted(filenames):
            if (
                filename.startswith(("libLLVM", "libgallium"))
                or "_icd" in filename and filename.endswith(".json")
            ):
                leaked_drivers.append(Path(root_dir, filename).relative_to(appdir).as_posix())
    if leaked_drivers:
        raise ValueError("Host GPU driver libraries or ICDs leaked into AppDir: " + ", ".join(leaked_drivers))


def stage_docs_and_icons(appdir, payload_stage, tools, libraries):
    # Copy hicolor icons preserving directory permissions and regular-file only constraint.
    hicolor_src = Path("packaging/linux/hicolor")
    hicolor_dst = appdir / "share/icons/hicolor"
    for source in sorted(hicolor_src.rglob("*")):
        if source.is_symlink():
            raise ValueError(f"Refusing symlink in icon asset: {source}")
        target = hicolor_dst / source.relative_to(hicolor_src)
        if source.is_dir():
            target.mkdir(parents=True, exist_ok=True)
            target.chmod(0o755)
        elif source.is_file():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            target.chmod(0o644)

    doc_src = payload_stage / "usr/share/doc/serein"
    doc_dst = appdir / "usr/share/doc/serein"
    if doc_dst.exists():
        shutil.rmtree(doc_dst)
    shutil.copytree(doc_src, doc_dst, symlinks=False)
    shutil.copyfile("packaging/appimage/README.md", doc_dst / "AppImage-README.md")
    (doc_dst / "AppImage-README.md").chmod(0o644)
    shutil.copyfile("packaging/appimage/RUNTIME-LICENSE", doc_dst / "licenses/AppImage-runtime.txt")
    (doc_dst / "licenses/AppImage-runtime.txt").chmod(0o644)
    extract_tar_member(
        tools / "cross-libc-dlopen-x86_64.tar",
        "LICENSE",
        doc_dst / "licenses/cross-libc-dlopen-LICENSE.txt",
        0o644,
    )
    (doc_dst / "AppImage-bundled-packages.txt").write_text(
        format_package_inventory(appdir, libraries),
        encoding="utf-8",
    )
    (doc_dst / "AppImage-bundled-packages.txt").chmod(0o644)


def format_package_inventory(appdir, ldd_output):
    lines = ["# Host executable link closure (ldd)", ldd_output.rstrip(), ""]
    if shutil.which("pacman"):
        host_paths = re.findall(r"(?:=>\s+|^\s*)(/\S+)", ldd_output, re.MULTILINE)
        owners = set()
        for path in sorted(set(host_paths)):
            if not Path(path).exists():
                continue
            owner = subprocess.run(
                ["pacman", "-Qqo", path],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                check=False,
            )
            if owner.returncode == 0 and owner.stdout.strip():
                owners.add(owner.stdout.strip())
        if owners:
            pkg_versions = subprocess.check_output(["pacman", "-Q", *sorted(owners)], text=True)
            lines.extend(["# Host packages queried via pacman -Q", pkg_versions.rstrip(), ""])
    bundled = sorted(
        p.relative_to(appdir).as_posix()
        for p in (appdir / "lib").rglob("*")
        if p.is_file() or p.is_symlink()
    )
    lines.extend(["# Bundled AppDir lib/ entries", *bundled, ""])
    return "\n".join(lines)


def ensure_multiarch_loader_symlink(appdir):
    loader = appdir / "lib/ld-linux-x86-64.so.2"
    multiarch_loader = appdir / "lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"
    if not loader.exists() and multiarch_loader.exists():
        if loader.is_symlink():
            loader.unlink()
        os.symlink("x86_64-linux-gnu/ld-linux-x86-64.so.2", loader)


def verify_zsync_control(zsync_path, image_size):
    if not zsync_path.is_file() or not 0 < zsync_path.stat().st_size <= MAX_ZSYNC_BYTES:
        raise ValueError("AppImage zsync metadata is missing, empty or oversized")
    body = zsync_path.read_bytes()
    end = body.find(b"\n\n")
    if end == -1 or end > 4096:
        raise ValueError("Invalid AppImage zsync control header")
    fields = {}
    for line in body[:end].decode("utf-8").splitlines():
        if ": " not in line:
            raise ValueError(f"Malformed AppImage zsync header line: {line!r}")
        key, value = line.split(": ", 1)
        if key in fields:
            raise ValueError(f"Duplicate AppImage zsync header key: {key}")
        fields[key] = value
    if fields.get("zsync") != "0.6.2":
        raise ValueError(f"Unsupported AppImage zsync version: {fields.get('zsync')!r}")
    blocksize = int(fields.get("Blocksize", "0"))
    if not 2048 <= blocksize <= 65536 or blocksize & (blocksize - 1) != 0:
        raise ValueError(f"Invalid AppImage zsync blocksize: {blocksize}")
    if int(fields.get("Length", "-1")) != image_size:
        raise ValueError("AppImage zsync Length does not match produced image size")
    lengths = [int(part) for part in fields.get("Hash-Lengths", "").split(",") if part]
    if len(lengths) != 3 or not 1 <= lengths[0] <= 2 or not 2 <= lengths[1] <= 4 or not 3 <= lengths[2] <= 16:
        raise ValueError(f"Invalid AppImage zsync Hash-Lengths: {fields.get('Hash-Lengths')!r}")
    block_count = (image_size + blocksize - 1) // blocksize
    sums = body[end + 2 :]
    if len(sums) != block_count * (lengths[1] + lengths[2]):
        raise ValueError("AppImage zsync checksum block count does not match image size")


def tree_entries(root):
    entries = {}

    def visit(directory):
        for child in sorted(directory.iterdir()):
            rel = child.relative_to(root).as_posix()
            if child.is_symlink():
                entries[rel] = ("symlink", os.readlink(child))
            elif child.is_dir():
                entries[rel] = ("dir", None)
                visit(child)
            elif child.is_file():
                entries[rel] = ("file", None)
            else:
                raise ValueError(f"Unsupported non-regular filesystem entry in AppImage: {rel}")

    visit(root)
    return entries


def verify_extracted_tree(appdir, extracted):
    appdir_entries = tree_entries(appdir)
    extracted_entries = tree_entries(extracted)
    if extracted_entries != appdir_entries:
        missing = sorted(set(appdir_entries) - set(extracted_entries))
        extra = sorted(set(extracted_entries) - set(appdir_entries))
        raise ValueError(
            f"Extracted AppImage payload differs from staged AppDir (missing={missing[:5]}, extra={extra[:5]})"
        )
    extracted_root = extracted.resolve()
    for rel, (kind, link_target) in appdir_entries.items():
        target = extracted / rel
        if kind == "file":
            if not filecmp.cmp(appdir / rel, target, shallow=False):
                raise ValueError(f"AppImage changed payload bytes: {rel}")
        elif kind == "symlink":
            if Path(link_target).is_absolute():
                raise ValueError(f"AppImage contains absolute symlink '{rel}' -> '{link_target}'")
            resolved = target.resolve()
            if resolved != extracted_root and not resolved.is_relative_to(extracted_root):
                raise ValueError(f"AppImage symlink escapes AppDir '{rel}' -> '{link_target}'")
    for executable in ("AppRun", "AppRun.sh", "sharun", "dash", "bin/serein", "shared/bin/serein"):
        if not os.access(extracted / executable, os.X_OK):
            raise ValueError(f"AppImage lost executable permission: {executable}")


def build(root, version):
    native_elf(root)
    if platform.machine() != "x86_64":
        raise ValueError("AppImage releases currently support Linux x86_64 only")
    name, information, _ = update_metadata(version, os.environ.get("SEREIN_RELEASE_TAG", ""))
    tools = Path("target/appimage-tools").resolve()
    verify_pinned_tools(tools)
    verify_host_tools()

    libraries = subprocess.check_output(["ldd", str(root / "serein")], text=True)
    if "not found" in libraries:
        raise ValueError(f"Missing host runtime libraries:\n{libraries}")
    require_gstreamer = "libgstreamer" in libraries
    deploy_pipewire = should_deploy_pipewire(libraries)
    webkit_bins = discover_webkit_binaries(libraries)

    with tempfile.TemporaryDirectory(prefix="serein-appimage-") as directory:
        temporary = Path(directory).resolve()
        payload_stage = temporary / "payload-stage"
        stage_payload(root, payload_stage)

        appdir = temporary / "Serein.AppDir"
        (appdir / "share/licenses").mkdir(parents=True, exist_ok=True)
        extract_tar_member(tools / "sharun+helper-libs-x86_64.tar", "sharun", appdir / "sharun", 0o755)
        extract_tar_member(tools / "sharun+helper-libs-x86_64.tar", "dash", appdir / "dash", 0o755)
        extract_tar_member(
            tools / "sharun+helper-libs-x86_64.tar",
            "dash-COPYING",
            appdir / "share/licenses/dash-COPYING",
            0o644,
        )

        tool_tmp = temporary / "tool-tmp"
        tool_tmp.mkdir()
        shutil.copyfile(tools / "sharun+helper-libs-x86_64.tar", tool_tmp / "sharun+helper-libs-x86_64.tar")
        shutil.copyfile(tools / "cross-libc-dlopen-x86_64.tar", tool_tmp / "cross-libc-dlopen-x86_64.tar")

        desktop_src = temporary / "cz.viceverse.serein.desktop"
        desktop_src.write_text(
            Path("packaging/linux/serein.desktop").read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        staged_bin_dir = temporary / "staged-bin"
        staged_bin_dir.mkdir()
        staged_exe = staged_bin_dir / "serein"
        shutil.copyfile(payload_stage / "usr/bin/serein", staged_exe)
        staged_exe.chmod(0o755)

        sharun_env = {
            **os.environ,
            "APPDIR": str(appdir),
            "TMPDIR": str(tool_tmp),
            "ARCH": "x86_64",
            "APPIMAGE_ARCH": "x86_64",
            "VERSION": version,
            "DESKTOP": str(desktop_src),
            "ICON": str(Path("packaging/linux/hicolor/256x256/apps/serein.png").resolve()),
            "MAIN_BIN": "serein",
            "OUTPUT_APPIMAGE": "0",
            "STRACE_MODE": "0",
            "USE_HOST_DRIVERS": "1",
            "PATH": f"{tools / 'bin'}:{os.environ.get('PATH', '')}",
        }
        if deploy_pipewire:
            sharun_env["DEPLOY_PIPEWIRE"] = "1"
        else:
            sharun_env.pop("DEPLOY_PIPEWIRE", None)

        subprocess.run(
            [str(tools / "quick-sharun.sh"), str(staged_exe), *webkit_bins],
            check=True,
            cwd=temporary,
            env=sharun_env,
        )

        # quick-sharun names top-level desktop entries cleanly, or re-assert exact base entry
        # before appimagetool appends X-AppImage-* metadata.
        top_desktop = appdir / "cz.viceverse.serein.desktop"
        for stale in list(appdir.glob("*.desktop")):
            if stale != top_desktop:
                stale.unlink()
        top_desktop.write_text(
            Path("packaging/linux/serein.desktop").read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        top_desktop.chmod(0o644)

        ensure_multiarch_loader_symlink(appdir)
        prune_broken_gstreamer_plugins(appdir, require_gstreamer, deploy_pipewire)
        prune_broken_glycin_loaders(appdir)
        subprocess.run([str(appdir / "sharun"), "-g"], check=True, cwd=temporary)
        stage_docs_and_icons(appdir, payload_stage, tools, libraries)
        verify_appdir_invariants(appdir)
        verify_elf_closure(appdir)
        subprocess.run(["desktop-file-validate", str(top_desktop)], check=True)

        runtime_copy = temporary / "runtime-x86_64"
        shutil.copyfile(tools / "runtime-x86_64", runtime_copy)
        runtime_copy.chmod(0o755)
        out_dir = temporary / "out"
        out_dir.mkdir()
        subprocess.run(
            [
                str(tools / "appimagetool"),
                str(appdir),
                "--output",
                str(out_dir),
                "--name",
                name,
                "--runtime",
                str(runtime_copy),
                "--mkdwarfs",
                str(tools / "bin/mkdwarfs"),
                "--update-info",
                information,
                "--dwarfs-comp",
                DWARFS_COMPRESSION,
                "--tmpdir",
                str(tool_tmp),
            ],
            check=True,
            cwd=temporary,
            env={
                **os.environ,
                "ARCH": "x86_64",
                "APPIMAGE_ARCH": "x86_64",
                "VERSION": version,
            },
        )

        candidate = out_dir / name
        if not candidate.is_file():
            raise ValueError(f"Expected AppImage artifact {name} was not created")
        with candidate.open("rb") as stream:
            header = stream.read(20)
        if (
            len(header) != 20
            or header[:7] != b"\x7fELF\x02\x01\x01"
            or header[8:11] != b"AI\x02"
            or struct.unpack("<H", header[16:18])[0] not in (2, 3)
            or header[18:20] != b"\x3e\x00"
        ):
            raise ValueError("Expected a Type 2 x86_64 AppImage")
        image_size = candidate.stat().st_size
        if not 0 < image_size <= MAX_IMAGE_BYTES:
            raise ValueError("AppImage exceeds the in-app updater's 512 MiB limit")
        candidate.chmod(0o755)

        embedded = subprocess.check_output([str(candidate), "--appimage-updateinformation"], text=True)
        if embedded.strip() != information:
            raise ValueError("AppImage update information did not survive packaging")
        zsync = out_dir / f"{name}.zsync"
        verify_zsync_control(zsync, image_size)

        # Execute only the pinned uruntime extraction command, not AppRun or Serein.
        extract_dir = temporary / "extracted-check"
        extract_dir.mkdir()
        subprocess.run(
            [str(candidate), "--appimage-extract"],
            cwd=extract_dir,
            stdout=subprocess.DEVNULL,
            check=True,
        )
        extracted = extract_dir / "AppDir"
        if not extracted.is_dir() or extracted.is_symlink():
            raise ValueError("AppImage extraction did not produce expected AppDir tree")
        verify_extracted_tree(appdir, extracted)

        extracted_desktop = extracted / "cz.viceverse.serein.desktop"
        desktop_lines = extracted_desktop.read_text(encoding="utf-8").splitlines()
        if f"X-AppImage-Version={version}" not in desktop_lines:
            raise ValueError("Extracted desktop entry is missing expected X-AppImage-Version")
        subprocess.run(["desktop-file-validate", str(extracted_desktop)], check=True)

        destination = root / candidate.name
        shutil.copyfile(candidate, destination)
        destination.chmod(0o755)
        shutil.copyfile(zsync, root / zsync.name)
        print(
            f"Unsigned portable AppImage: {destination} ({image_size} bytes); "
            "uses host GPU drivers/portals via cross-libc-dlopen"
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("staged_directory", type=Path)
    parser.add_argument("application_version")
    args = parser.parse_args()
    build(args.staged_directory.resolve(), args.application_version)
