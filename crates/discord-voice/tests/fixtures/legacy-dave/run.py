"""Build two offline DAVE peers and check cross-version join and media encryption.
Requires cached dependencies and the pinned Rust toolchain. Never uses Discord/audio devices.
"""
from contextlib import suppress
import hashlib
import io
import json
from pathlib import Path
from queue import Empty, Queue
import shutil
import subprocess
import tarfile
import tempfile
from threading import Thread
import tomllib

BASE = '38e46b8f5ec694002ef515fbc2d8dc69297e4f5f'
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
LIMIT = 64 * 1024


def build(directory, legacy):
    directory.mkdir()
    (directory / 'src').mkdir()
    shutil.copyfile(HERE / 'peer.rs', directory / 'src/main.rs')
    if legacy:
        archive = subprocess.check_output(['git', 'archive', BASE, 'vendor/davey', 'vendor/hpke-rs'], cwd=ROOT)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(directory, filter='data')
        patch = {'davey': {'path': str(directory / 'vendor/davey')}, 'hpke-rs': {'path': str(directory / 'vendor/hpke-rs')}}
        versions = ('=0.8.1', '=0.5.0', '=0.5.1')
        lock = subprocess.check_output(['git', 'show', f'{BASE}:Cargo.lock'], cwd=ROOT)
    else:
        config = tomllib.loads((ROOT / 'Cargo.toml').read_text())
        patch = {name: value for name, value in config['patch']['crates-io'].items() if name.startswith('openmls') or name in {'davey', 'hpke-rs'}}
        for value in patch.values():
            if 'path' in value:
                value['path'] = str(ROOT / value['path'])
        versions = ('=0.9.0', '=0.6.0', '=0.6.0')
        lock = (ROOT / 'Cargo.lock').read_bytes()
    manifest = '[package]\nname="dave-interop-peer"\nversion="0.0.0"\nedition="2024"\n\n[workspace]\n\n[dependencies]\ndavey="=0.1.4"\nserde_json="1"\n'
    for name, version in zip(['openmls', 'openmls_basic_credential', 'openmls_rust_crypto'], versions):
        manifest += f'{name}="{version}"\n'
    manifest += '\n[patch.crates-io]\n'
    for name, value in patch.items():
        manifest += name + ' = { ' + ', '.join(key + '=' + json.dumps(val) for key, val in value.items()) + ' }\n'
    (directory / 'Cargo.toml').write_text(manifest)
    (directory / 'Cargo.lock').write_bytes(lock)
    target = ROOT / 'target/dave-interop' / ('legacy' if legacy else 'current')
    subprocess.run(['cargo', 'build', '--offline', '--manifest-path', str(directory / 'Cargo.toml'), '--target-dir', str(target), '--jobs', '2'], check=True, timeout=1800)
    return target / 'debug/dave-interop-peer'


class Peer:
    def __init__(self, command, name, reply_timeout=30):
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.name = name
        self.reply_timeout = reply_timeout
        self.aborted = False

    def request(self, op, **data):
        line = json.dumps({'op': op, **data}) + '\n'
        assert len(line.encode()) <= LIMIT
        result = Queue(maxsize=1)

        def communicate():
            try:
                self.process.stdin.write(line)
                self.process.stdin.flush()
                result.put((self.process.stdout.readline(LIMIT + 1), None))
            except Exception as error:
                result.put((None, error))

        # Include writes in the deadline: a stuck peer can stop reading its pipe too.
        worker = Thread(target=communicate, daemon=True)
        worker.start()
        try:
            reply, error = result.get(timeout=self.reply_timeout)
        except Empty:
            raise TimeoutError(f'{self.name} peer timed out during {op} after {self.reply_timeout}s') from None
        if error is not None:
            raise error
        assert reply.endswith('\n') and len(reply.encode()) <= LIMIT, 'Missing/bounded peer response'
        return json.loads(reply)

    def ok(self, op, **data):
        reply = self.request(op, **data)
        assert 'error' not in reply, (op, reply)
        return reply

    def stop(self):
        self.aborted = True
        if self.process.poll() is None:
            try:
                self.process.kill()
            except ProcessLookupError:
                pass

    def close(self):
        if not self.aborted:
            with suppress(BrokenPipeError):
                self.process.stdin.close()
        try:
            status = self.process.wait(timeout=10)
            if not self.aborted:
                assert status == 0, f'{self.name} peer exited with status {status}'
        except subprocess.TimeoutExpired:
            self.stop()
            self.process.wait(timeout=10)
            raise
        finally:
            with suppress(BrokenPipeError):
                self.process.stdin.close()
            self.process.stdout.close()


def exchange(legacy, current, legacy_creator):
    old, new = Peer([str(legacy)], "legacy"), Peer([str(current)], "current")
    try:
        creator, joiner = (old, new) if legacy_creator else (new, old)
        external = old.ok('external')['bytes']
        creator.ok('init', user=1, external=external)
        joiner.ok('init', user=2, external=external)
        package = joiner.ok('package')['bytes']
        proposal = old.ok('proposal', bytes=package)['bytes']
        result = creator.ok('proposals', bytes=proposal)
        creator.ok('commit', bytes=result['commit'])
        joiner.ok('welcome', bytes=result['welcome'])
        assert creator.ok('privacy') == joiner.ok('privacy'), 'Epoch/exporter/privacy mismatch'
        for sender, receiver, user in [(creator, joiner, 1), (joiner, creator, 2)]:
            plain = list(b'synthetic opus bytes')
            packet = sender.ok('encrypt', bytes=plain)['bytes']
            assert packet != plain
            damaged = packet.copy()
            damaged[0] ^= 1
            assert 'error' in receiver.request('decrypt', user=user, bytes=damaged)
            assert receiver.ok('decrypt', user=user, bytes=packet)['bytes'] == plain
            assert 'error' in receiver.request('decrypt', user=user, bytes=packet)
        if not legacy_creator:
            expected = HERE / 'key-package.bin'
            # This public synthetic package belongs to user 2/channel 3; no private key is stored.
            if not expected.exists():
                expected.write_bytes(bytes([26, *package]))
        return {'creator': 'legacy' if legacy_creator else 'current', 'join': True, 'own_commit': True, 'bidirectional_media': True, 'tamper_rejected': True, 'replay_rejected': True, 'matching_privacy_code': True}
    except BaseException:
        # Stop both before waiting/closing pipes, including a worker blocked on I/O.
        old.stop()
        new.stop()
        raise
    finally:
        try:
            old.close()
        finally:
            new.close()


if __name__ == '__main__':
    version = subprocess.check_output(['rustc', '--version'], text=True).split()[1]
    assert version == '1.98.1', f'Use the pinned Rust 1.98.1, found {version}'
    with tempfile.TemporaryDirectory(prefix='serein-dave-interop-') as directory:
        temp = Path(directory)
        legacy = build(temp / 'legacy', True)
        current = build(temp / 'current', False)
        results = [exchange(legacy, current, creator) for creator in [True, False]]
    config = tomllib.loads((ROOT / 'Cargo.toml').read_text())
    print(json.dumps({
        'legacy_source': BASE,
        'current_openmls_revision': config['patch']['crates-io']['openmls']['rev'],
        'application_lockfile_sha256': hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest(),
        'peer_source_sha256': hashlib.sha256((HERE / 'peer.rs').read_bytes()).hexdigest(),
        'rust': version,
        'scenarios': results,
        'fixture_sha256': hashlib.sha256((HERE / 'key-package.bin').read_bytes()).hexdigest(),
    }, indent=2))
