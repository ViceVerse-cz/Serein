"""Exercise piped setup with a real terminal and simulated privileged commands."""

import os
from pathlib import Path
import select
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
        for distro, version, id_like, answer, key, expected_status, installed in cases:
            with self.subTest(distro=distro, version=version, answer=answer, key=key), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                release = root / "os-release"
                release.write_text(f'ID={distro}\nVERSION_ID={version}\nID_LIKE="{id_like}"\n')
                setup = root / "setup.sh"
                setup.write_text(Path(__file__).with_name("setup.sh").read_text().replace("/etc/os-release", str(release)))
                commands = {
                    "id": "echo 0",
                    "uname": "echo x86_64",
                    "curl": 'printf "DOWNLOAD %s\\n" "$5"; printf "fixture\\n" > "$7"',
                    "gpg": f'printf "fpr:::::::::{key}:\\n"',
                    "rpm": 'echo "SIMULATED WRITE rpm $*"',
                    "install": 'echo "SIMULATED WRITE install $*"',
                    "pacman-key": 'echo "SIMULATED WRITE pacman-key $*"',
                    "tee": 'echo "SIMULATED WRITE tee $*"',
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
                if expected_status == 0:
                    path = (
                        "/arch/x86_64/arch/serein.asc"
                        if id_like == "arch"
                        else f"/fedora-{version}/x86_64/rpm/serein.repo"
                    )
                    self.assertIn(path, output)
                else:
                    self.assertNotIn("SIMULATED WRITE", output)


if __name__ == "__main__":
    unittest.main()
