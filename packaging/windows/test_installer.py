"""Offline NSIS install/upgrade/uninstall check. Run only in a disposable Windows user."""
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import uuid
if os.name == "nt":
    import winreg


class GUID(ctypes.Structure):
    _fields_ = [("bytes", ctypes.c_ubyte * 16)]

    def __init__(self, value):
        super().__init__((ctypes.c_ubyte * 16).from_buffer_copy(uuid.UUID(value).bytes_le))


# Invoke native COM without pywin32 or compiling C# during the test.
def method(pointer, index, result, *arguments):
    table = ctypes.cast(pointer, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
    return ctypes.WINFUNCTYPE(result, ctypes.c_void_p, *arguments)(table[index])


def check_shortcut(shortcut, executable):
    ole = ctypes.OleDLL("ole32")
    shell = ctypes.OleDLL("shell32")
    ole.CoInitializeEx(None, 2)
    link, persist, store = ctypes.c_void_p(), ctypes.c_void_p(), ctypes.c_void_p()
    try:
        ole.CoCreateInstance(ctypes.byref(GUID("00021401-0000-0000-c000-000000000046")), None, 1,
                             ctypes.byref(GUID("000214f9-0000-0000-c000-000000000046")), ctypes.byref(link))
        method(link, 0, ctypes.HRESULT, ctypes.c_void_p, ctypes.c_void_p)(
            link, ctypes.byref(GUID("0000010b-0000-0000-c000-000000000046")), ctypes.byref(persist))
        method(persist, 5, ctypes.HRESULT, ctypes.c_wchar_p, ctypes.c_uint)(persist, str(shortcut), 0)
        target = ctypes.create_unicode_buffer(1024)
        method(link, 3, ctypes.HRESULT, ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint)(
            link, target, len(target), None, 4)
        assert Path(target.value).samefile(executable), target.value
        working = ctypes.create_unicode_buffer(1024)
        method(link, 8, ctypes.HRESULT, ctypes.c_void_p, ctypes.c_int)(link, working, len(working))
        assert Path(working.value).samefile(executable.parent), working.value
        icon_path = ctypes.create_unicode_buffer(1024)
        icon_index = ctypes.c_int()
        method(link, 16, ctypes.HRESULT, ctypes.c_void_p, ctypes.c_int, ctypes.POINTER(ctypes.c_int))(
            link, icon_path, len(icon_path), ctypes.byref(icon_index))
        assert Path(icon_path.value).samefile(executable), icon_path.value
        assert icon_index.value == 0, icon_index.value
        shell.SHGetPropertyStoreFromParsingName(ctypes.c_wchar_p(str(shortcut)), None, 0,
            ctypes.byref(GUID("886d8eeb-8cf2-4446-8d02-cdba1dbdcf99")), ctypes.byref(store))
        key = GUID("9f4c2855-9f79-4b39-a8d0-e1d42de1d5f3").bytes[:] + list((5).to_bytes(4, "little"))
        key = (ctypes.c_ubyte * 20)(*key)
        value = ctypes.create_string_buffer(24 if ctypes.sizeof(ctypes.c_void_p) == 8 else 16)
        method(store, 5, ctypes.HRESULT, ctypes.c_void_p, ctypes.c_void_p)(store, key, value)
        try:
            assert ctypes.c_ushort.from_buffer(value).value == 31
            app_id = ctypes.c_void_p.from_buffer(value, 8).value
            assert ctypes.wstring_at(app_id) == "cz.viceverse.serein"
        finally:
            ole.PropVariantClear(value)
    finally:
        for pointer in (store, persist, link):
            if pointer:
                method(pointer, 2, ctypes.c_ulong)(pointer)
        ole.CoUninitialize()


def wait_for(predicate):
    for _ in range(100):
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError("Timed out waiting for fixture/cleanup")


def main():
    if os.name != "nt":
        raise SystemExit("This behavioral check requires Windows; it never runs the real app.")
    repository = Path(__file__).resolve().parents[2]
    compiler = shutil.which("makensis") or r"C:\Program Files (x86)\NSIS\makensis.exe"
    programs = ctypes.create_unicode_buffer(1024)
    ctypes.OleDLL("shell32").SHGetFolderPathW(None, 2, None, 0, programs)
    shortcut = Path(programs.value) / "NuSerein.lnk"
    legacy_shortcut = Path(programs.value) / "Serein.lnk"
    default_install = Path(os.environ["LOCALAPPDATA"]) / "Programs" / "Serein"
    uninstall_key = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Serein"
    assert not shortcut.exists() and not legacy_shortcut.exists(), "Refusing to overwrite an existing shortcut"
    assert not default_install.exists(), "Refusing to overwrite the default installation directory"
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key):
            raise AssertionError("Refusing to overwrite an existing Serein installation")
    except FileNotFoundError:
        pass
    with tempfile.TemporaryDirectory(prefix="serein-installer-offline-") as temporary:
        root = Path(temporary)
        payload, output = root / "payload", root / "output"
        payload.mkdir()
        output.mkdir()
        fixture = root / "fixture.rs"
        fixture.write_text('fn main() { if let Some(p) = std::env::args().nth(1) { std::fs::write(p, b"ready").unwrap(); std::thread::sleep(std::time::Duration::from_secs(120)); } }')
        # The fixture runs outside the toolchain environment after installation.
        subprocess.run(["rustc", "--edition=2024", "-C", "target-feature=+crt-static", "-O",
                        str(fixture), "-o", str(payload / "serein.exe")], check=True)
        (payload / "LICENSE-MIT").write_text("Synthetic installer fixture")
        for name in ("avcodec-serein-61.dll", "avutil-serein-59.dll", "avformat-serein-61.dll", "avfilter-serein-10.dll", "swscale-serein-8.dll", "swresample-serein-5.dll", "openh264.dll"):
            (payload / name).write_bytes(b"synthetic DLL; never loaded")
        (payload / "ffmpeg-source/source").mkdir(parents=True)
        (payload / "ffmpeg-source/COPYING.GPLv2").write_text("Synthetic FFmpeg license fixture")
        (payload / "ffmpeg-source/camera-filter").mkdir()
        for name in ("denoise.rs", "gpu.rs", "camera_filter.wgsl"):
            (payload / "ffmpeg-source/camera-filter" / name).write_text("synthetic camera filter source")
        (payload / "ffmpeg-source/source/ffmpeg-7.1.5.tar.xz").write_bytes(b"synthetic FFmpeg source")
        (payload / "voice-enhancement").mkdir()
        for name in ("onnxruntime.dll", "backbone.onnx", "spec_head.onnx", "denoiser_core.onnx",
                     "ONNXRuntime-LICENSE", "ONNXRuntime-ThirdPartyNotices.txt", "MSVC-LICENSE.txt"):
            (payload / "voice-enhancement" / name).write_bytes(b"synthetic speech payload; never loaded")
        crt_files = ["msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll"]
        extra_crt = "vcruntime140_1.dll"
        if os.environ.get("PROCESSOR_ARCHITECTURE", "").upper() != "ARM64":
            crt_files.append(extra_crt)
        for name in crt_files:
            (payload / name).write_bytes(b"synthetic CRT; never loaded")
        for name in ("install-notifications.ps1", "setup.ps1"):
            (payload / name).write_text("throw 'Installer must not run or install this optional script'")
        subprocess.run([compiler, "-V2", "-NOCD", "-DVERSION=0.0.0-offline", f"-DDIST_DIR={payload}",
                        f"-DOUTPUT_DIR={output}", str(repository / "packaging/windows/installer.nsi")],
                       cwd=repository, check=True, timeout=120)
        setup = output / "serein-0.0.0-offline-setup.exe"
        icon_check = repository / "packaging/windows/check-brand-icon.ps1"
        subprocess.run(["powershell", "-NoProfile", "-File", str(icon_check), str(setup)],
                       check=True, timeout=30)
        installed = root / "Installed Serein žluťoučký"
        uninstaller = installed / "uninstall.exe"
        copied_uninstaller = root / "uninstall-test.exe"

        def uninstall():
            # NSIS's normal bootstrap detaches a temporary child and returns 0.
            # Run an external copy with _?= to wait for the actual uninstaller.
            shutil.copy2(uninstaller, copied_uninstaller)
            result = subprocess.run(f'"{copied_uninstaller}" /S _?={installed}', timeout=30)
            # Emulation/scanning can briefly retain the exited fixture's image.
            def remove_copy():
                try:
                    copied_uninstaller.unlink()
                    return True
                except PermissionError:
                    return False
            wait_for(remove_copy)
            return result

        timings = []
        running = None
        try:
            # A different legacy shortcut must not be removed by the rebranded installer.
            legacy_shortcut.write_bytes(b"synthetic legacy shortcut; preserve")
            # Case-insensitive process recognition, with no GUI, Discord or audio.
            running_exe = root / "SEREIN.EXE"
            shutil.copy2(payload / "serein.exe", running_exe)
            ready = root / "ready"
            running = subprocess.Popen([str(running_exe), str(ready)])
            wait_for(ready.exists)
            # NSIS requires /D= last and unquoted, including paths with spaces.
            result = subprocess.run(f'"{setup}" /S /D={installed}', timeout=30)
            assert result.returncode == 1, result.returncode
            assert not installed.exists()
            running.terminate()
            running.wait(timeout=10)
            running = None
            # A pre-FFmpeg install has no media DLLs; a manual installer upgrade
            # must replace its executable and deliver the entire new runtime.
            installed.mkdir()
            unrelated = installed / "unrelated.txt"
            unrelated.write_text("Owner data must survive install, upgrade and uninstall")
            unrelated_directory = installed / "owner-data"
            unrelated_directory.mkdir()
            unrelated_nested = unrelated_directory / "keep.txt"
            unrelated_nested.write_text("Unrelated nested owner data")
            (installed / "serein.exe").write_bytes(b"legacy synthetic client; never executed")
            (installed / "LICENSE-MIT").write_text("Legacy installer fixture")
            with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, uninstall_key, 0,
                                    winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY) as key:
                winreg.SetValueEx(key, "DisplayName", 0, winreg.REG_SZ, "Serein")
                winreg.SetValueEx(key, "InstallLocation", 0, winreg.REG_SZ, str(installed))
            for name in ("install-notifications.ps1", "setup.ps1"):
                (installed / name).write_text("legacy fixture")
            (installed / extra_crt).write_bytes(b"legacy x64 CRT")
            assert not (installed / "avcodec-serein-61.dll").exists()
            for attempt in range(3):
                start = time.perf_counter()
                # The first upgrade must discover the pre-existing registered location.
                command = f'"{setup}" /S' if attempt == 0 else f'"{setup}" /S /D={installed}'
                subprocess.run(command, check=True, timeout=30)
                timings.append(round((time.perf_counter() - start) * 1000, 3))
                assert (installed / "serein.exe").read_bytes() == (payload / "serein.exe").read_bytes()
                for name in ("avcodec-serein-61.dll", "avutil-serein-59.dll", "avfilter-serein-10.dll", "openh264.dll", "ffmpeg-source/COPYING.GPLv2", "ffmpeg-source/camera-filter/camera_filter.wgsl", "ffmpeg-source/source/ffmpeg-7.1.5.tar.xz", "voice-enhancement/denoiser_core.onnx", "voice-enhancement/onnxruntime.dll", "msvcp140_1.dll"):
                    assert (installed / name).read_bytes() == (payload / name).read_bytes()
                assert uninstaller.is_file()
                subprocess.run(["powershell", "-NoProfile", "-File", str(icon_check), str(uninstaller)],
                               check=True, timeout=30)
                assert unrelated.read_text() == "Owner data must survive install, upgrade and uninstall"
                assert unrelated_nested.read_text() == "Unrelated nested owner data"
                assert not (installed / "install-notifications.ps1").exists()
                assert not (installed / "setup.ps1").exists()
                for name in crt_files:
                    assert (installed / name).read_bytes() == (payload / name).read_bytes()
                assert (installed / extra_crt).exists() == (extra_crt in crt_files)
                check_shortcut(shortcut, installed / "serein.exe")
                assert legacy_shortcut.read_bytes() == b"synthetic legacy shortcut; preserve"
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key, 0,
                                    winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
                    assert winreg.QueryValueEx(key, "DisplayName")[0] == "NuSerein"
                    assert winreg.QueryValueEx(key, "DisplayIcon")[0] == f"{installed / 'serein.exe'},0"
                    assert winreg.QueryValueEx(key, "DisplayVersion")[0] == "0.0.0-offline"
                    assert Path(winreg.QueryValueEx(key, "InstallLocation")[0]).samefile(installed)
                # Exercise removal of files left by an older script-based installer.
                for name in ("install-notifications.ps1", "setup.ps1"):
                    (installed / name).write_text("legacy fixture")
            ready.unlink()
            running = subprocess.Popen([str(installed / "serein.exe"), str(ready)])
            wait_for(ready.exists)
            result = uninstall()
            assert result.returncode == 1, result.returncode
            assert (installed / "serein.exe").exists() and shortcut.exists()
            running.terminate()
            running.wait(timeout=10)
            running = None
            uninstall().check_returncode()
            wait_for(lambda: not uninstaller.exists())
            assert unrelated.read_text() == "Owner data must survive install, upgrade and uninstall"
            assert unrelated_nested.read_text() == "Unrelated nested owner data"
            remaining = sorted(path.relative_to(installed).as_posix() for path in installed.rglob("*") if path.is_file())
            assert remaining == ["owner-data/keep.txt", "unrelated.txt"], remaining
            assert not shortcut.exists()
            assert legacy_shortcut.read_bytes() == b"synthetic legacy shortcut; preserve"
            try:
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key, 0,
                                    winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
                    raise AssertionError("Uninstall registration was not removed")
            except FileNotFoundError:
                pass
            print(json.dumps({"fixture": "synthetic/offline", "architecture": os.environ.get("PROCESSOR_ARCHITECTURE"),
                              "installer_bytes": setup.stat().st_size, "install_ms": timings,
                              "checks": "running guard, Unicode paths, pre-FFmpeg manual migration, payload, shortcut target/working directory/AUMID, upgrade, owned-only uninstall, unrelated file preservation"}))
        finally:
            if running is not None:
                running.terminate()
                running.wait(timeout=10)
            if uninstaller.exists():
                uninstall().check_returncode()
                wait_for(lambda: not uninstaller.exists())
            if shortcut.exists():
                shortcut.unlink()
            if legacy_shortcut.exists():
                assert legacy_shortcut.read_bytes() == b"synthetic legacy shortcut; preserve"
                legacy_shortcut.unlink()
            try:
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key, 0,
                                    winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
                    location = winreg.QueryValueEx(key, "InstallLocation")[0]
                if Path(location) == installed:
                    winreg.DeleteKeyEx(winreg.HKEY_CURRENT_USER, uninstall_key,
                                       winreg.KEY_WOW64_64KEY)
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    main()
