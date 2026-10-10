"""Exercise piped setup with a real terminal and simulated privileged commands."""

import os
from pathlib import Path
import select
import subprocess
import tempfile
import time
import unittest


@unittest.skipUnless(os.name == "posix", "setup.sh requires a Unix terminal")
class SetupTest(unittest.TestCase):
    def test_distribution_key_and_terminal_handoff(self):
        import pty

        fingerprint = "CA19DA939E9BCAB500751CE480FE95CAD86141A5"
        cases = [
            ("fedora", "43", "", "y", fingerprint, 0, True),
            ("fedora", "44", "", "y", fingerprint, 0, True),
            ("fedora", "43", "", "n", fingerprint, 0, False),
            ("fedora", "43", "", "y", "0" * 40, 1, False),
            ("fedora", "42", "", "y", fingerprint, 1, False),
            ("ubuntu", "24.04", "debian", "y", fingerprint, 1, False),
            ("opensuse-leap", "16.0", "suse", "y", fingerprint, 1, False),
            ("manjaro", "26", "", "y", fingerprint, 1, False),
            ("cachyos", "rolling", "arch", "n", fingerprint, 0, False),
        ]
        cases = [(*case, "x86_64") for case in cases]
        cases.extend([
            ("ubuntu", "26.04", "debian", "n", fingerprint, 0, False, "aarch64"),
            ("ubuntu", "26.04", "debian", "n", fingerprint, 0, False, "arm64"),
            ("ubuntu", "26.04", "debian", "n", fingerprint, 0, False, "x86_64"),
            ("ubuntu", "24.04", "debian", "n", fingerprint, 1, False, "aarch64"),
            ("fedora", "44", "", "n", fingerprint, 1, False, "aarch64"),
        ])
        for distro, version, id_like, answer, key, expected_status, installed, architecture in cases:
            with self.subTest(distro=distro, version=version, architecture=architecture, answer=answer, key=key), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                release = root / "os-release"
                release.write_text(f'ID={distro}\nVERSION_ID={version}\nID_LIKE="{id_like}"\n')
                pacman = root / "pacman.conf"
                pacman.write_text("[options]\nArchitecture = auto\n")
                setup = root / "setup.sh"
                setup.write_text(Path(__file__).with_name("setup.sh").read_text()
                                 .replace("/etc/os-release", str(release)).replace("/etc/pacman.conf", str(pacman)))
                commands = {
                    "id": "echo 0",
                    "uname": f"echo {architecture}",
                    "apt-get": ":",
                    "curl": 'printf "DOWNLOAD %s\\n" "$5"; printf "fixture\\n" > "$7"',
                    "gpg": f'printf "pub::::::::::\\nfpr:::::::::{key}:\\n"',
                    "rpm": 'echo "SIMULATED WRITE rpm $*"',
                    "install": 'echo "SIMULATED WRITE install $*"',
                    "pacman-key": 'echo "SIMULATED WRITE pacman-key $*"',
                    "tee": 'cat > "$(dirname "$0")/source-list"; echo "SIMULATED WRITE tee $*"',
                    "dnf": '[ -t 0 ] || exit 20; printf "Key import [y/N]: "; read -r key_answer; '
                           '[ "$key_answer" = y ] || exit 21; echo "SIMULATED INSTALL"',
                }
                for name, body in commands.items():
                    command = root / name
                    command.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
                    command.chmod(0o755)
                pid, terminal = pty.fork()
                if pid == 0:
                    os.environ["PATH"] = str(root) + ":/usr/bin:/bin"
                    os.execv("/bin/sh", ["sh", "-c", 'cat "$1" | sh', "setup-test", str(setup)])
                transcript = b""
                answered = set()
                status = None
                try:
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline:
                        if not select.select([terminal], [], [], 0.1)[0]:
                            continue
                        try:
                            chunk = os.read(terminal, 8192)
                        except OSError:
                            break
                        if not chunk:
                            break
                        transcript += chunk
                        for prompt, response in [(b"now? [Y/n]:", answer), (b"Key import [y/N]:", "y")]:
                            if prompt in transcript and prompt not in answered:
                                os.write(terminal, (response + "\n").encode())
                                answered.add(prompt)
                    else:
                        self.fail("Setup terminal prompt timed out")
                    _, status = os.waitpid(pid, 0)
                finally:
                    os.close(terminal)
                    # The fixture has no background work; terminate it on a timeout.
                    if status is None:
                        try:
                            os.kill(pid, 9)
                        except ProcessLookupError:
                            pass
                        os.waitpid(pid, 0)
                output = transcript.decode()
                self.assertEqual(os.waitstatus_to_exitcode(status), expected_status, output)
                self.assertEqual("SIMULATED INSTALL" in output, installed, output)
                self.assertNotIn(r"\033[", output)
                if expected_status == 0 and distro == "ubuntu":
                    deb_arch = "amd64" if architecture == "x86_64" else "arm64"
                    self.assertIn(f"/ubuntu-26.04/{deb_arch}/apt/serein.asc", output)
                    source = (root / "source-list").read_text()
                    self.assertIn(f"arch={deb_arch} ", source)
                    self.assertIn(f"/ubuntu-26.04/{deb_arch}/apt ./", source)
                elif expected_status == 0:
                    path = (
                        "/arch/x86_64/arch/serein.asc"
                        if id_like == "arch"
                        else f"/fedora-{version}/x86_64/rpm/serein.asc"
                    )
                    self.assertIn(path, output)
                else:
                    self.assertNotIn("SIMULATED WRITE", output)

    def test_only_pinned_primary_key_may_be_installed(self):
        fingerprint = "CA19DA939E9BCAB500751CE480FE95CAD86141A5"
        primary = f"pub::::::::::\nfpr:::::::::{fingerprint}:\n"
        other = f"pub::::::::::\nfpr:::::::::{'0' * 40}:\n"
        subkey = f"sub::::::::::\nfpr:::::::::{'1' * 40}:\n"
        cases = [
            ("one primary", primary, 0, 0),
            ("primary with subkeys", primary + subkey + subkey, 0, 0),
            ("additional primary", primary + other, 0, 1),
            ("duplicated primary", primary + primary, 0, 1),
            ("only fingerprint", f"fpr:::::::::{fingerprint}:\n", 0, 1),
            ("missing primary fingerprint", "pub::::::::::\n", 0, 1),
            ("failed GPG with partial output", primary, 1, 1),
        ]
        for name, records, gpg_status, expected_status in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                release = root / "os-release"
                release.write_text("ID=ubuntu\nVERSION_ID=26.04\n")
                setup = root / "setup.sh"
                setup.write_text(Path(__file__).with_name("setup.sh").read_text().replace("/etc/os-release", str(release)))
                (root / "key-records").write_text(records)
                commands = {
                    "id": "echo 0",
                    "uname": "echo x86_64",
                    "curl": 'printf "fixture\\n" > "$7"',
                    "gpg": f'cat "{root}/key-records"; exit {gpg_status}',
                    "install": 'echo "SIMULATED WRITE install"',
                    "tee": 'cat >/dev/null; echo "SIMULATED WRITE tee" >&2',
                    "apt-get": ":",
                }
                for command_name, body in commands.items():
                    command = root / command_name
                    command.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
                    command.chmod(0o755)
                environment = dict(os.environ, PATH=f"{root}:/usr/bin:/bin", SEREIN_FINGERPRINT=fingerprint)
                result = subprocess.run(["sh", str(setup)], env=environment, stdin=subprocess.DEVNULL,
                                        capture_output=True, text=True, start_new_session=True, timeout=10)
                transcript = result.stdout + result.stderr
                self.assertEqual(result.returncode, expected_status, transcript)
                self.assertEqual("SIMULATED WRITE" in transcript, expected_status == 0, transcript)

    def test_repository_configurations_are_local_and_rerunnable(self):
        for distro, version in [("fedora", "43"), ("opensuse-tumbleweed", "rolling"), ("arch", "rolling")]:
            with self.subTest(distro=distro), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                release = root / "os-release"
                release.write_text(f"ID={distro}\nVERSION_ID={version}\n")
                pacman = root / "pacman.conf"
                prefix = "[options]\nArchitecture = auto\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n\n"
                suffix = "\n[extra]\nInclude = /etc/pacman.d/mirrorlist\n"
                pacman.write_text(prefix + "[serein]\nSigLevel = Required\nServer = https://old.invalid/nightly\n" + suffix)
                setup = root / "setup.sh"
                setup.write_text(Path(__file__).with_name("setup.sh").read_text()
                                 .replace("/etc/os-release", str(release)).replace("/etc/pacman.conf", str(pacman)))
                untrusted_repo = root / "untrusted.repo"
                untrusted_repo.write_text("[serein-production]\nbaseurl=https://untrusted.invalid/\ngpgcheck=0\nrepo_gpgcheck=0\n")
                commands = {
                    "id": "echo 0",
                    "uname": "echo x86_64",
                    "curl": f'printf "%s\\n" "$5" >> "{root}/downloads"; '
                            f'case "$5" in *.repo) cp "{untrusted_repo}" "$7";; *) printf "verified key fixture\\n" > "$7";; esac',
                    "gpg": 'printf "pub::::::::::\\nfpr:::::::::CA19DA939E9BCAB500751CE480FE95CAD86141A5:\\n"',
                    "rpm": ":",
                    "pacman-key": ":",
                    "zypper": ":",
                    "install": f'case "$3" in */serein.repo) cp "$2" "{root}/installed.repo";; '
                               f'*/pacman.conf) cp "$2" "{pacman}";; */serein.asc) cp "$2" "{root}/installed.asc";; *) exit 99;; esac',
                    "tee": f'if [ "$1" = -a ]; then cat >> "{pacman}"; else cat > "{pacman}"; fi',
                }
                for name, body in commands.items():
                    command = root / name
                    command.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
                    command.chmod(0o755)
                env = dict(os.environ, PATH=f"{root}:/usr/bin:/bin", SEREIN_CHANNEL="production",
                           SEREIN_REPO_BASE_URL="https://selected.invalid/root")
                for _ in range(2):
                    result = subprocess.run(["sh", str(setup)], env=env, stdin=subprocess.DEVNULL,
                                            capture_output=True, text=True, start_new_session=True, timeout=10)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    if distro == "arch":
                        self.assertEqual(pacman.read_text(), prefix + "[serein]\nSigLevel = Required\n"
                                         "Server = https://selected.invalid/root/production/arch/x86_64/arch\n" + suffix)
                    else:
                        config = (root / "installed.repo").read_text()
                        distribution = f"fedora-{version}" if distro == "fedora" else distro
                        self.assertIn("baseurl=https://selected.invalid/root/production/" + distribution + "/x86_64/rpm\n", config)
                        self.assertIn("gpgcheck=1\n", config)
                        self.assertIn("repo_gpgcheck=1\n", config)
                        self.assertIn("gpgkey=file:///etc/pki/rpm-gpg/serein.asc\n", config)
                        self.assertEqual((root / "installed.asc").read_text(), "verified key fixture\n")
                        self.assertNotIn(".repo", (root / "downloads").read_text())
                before_downloads = (root / "downloads").read_bytes()
                for invalid in [{"SEREIN_CHANNEL": "other"}, {"SEREIN_REPO_BASE_URL": "http://selected.invalid"},
                                {"SEREIN_REPO_BASE_URL": "https://selected.invalid\ngpgcheck=0"},
                                {"SEREIN_REPO_BASE_URL": "https://user@selected.invalid"}]:
                    result = subprocess.run(["sh", str(setup)], env={**env, **invalid}, stdin=subprocess.DEVNULL,
                                            capture_output=True, text=True, start_new_session=True, timeout=10)
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertEqual((root / "downloads").read_bytes(), before_downloads)


if __name__ == "__main__":
    unittest.main()
