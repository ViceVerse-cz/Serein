"""Build two offline DAVE peers and check cross-version join and media encryption.
Requires cached dependencies and the pinned Rust toolchain. Never uses Discord/audio devices.
"""
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
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
    subprocess.run(['cargo', 'build', '--offline', '--manifest-path', str(directory / 'Cargo.toml'), '--target-dir', str(target), '--jobs', '2'], check=True)
    return target / 'debug/dave-interop-peer'


class Peer:
    def __init__(self, binary):
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def request(self, op, **data):
        line = json.dumps({'op': op, **data}) + '\n'
        assert len(line.encode()) <= LIMIT
        self.process.stdin.write(line)
        self.process.stdin.flush()
        reply = self.process.stdout.readline(LIMIT + 1)
        assert reply.endswith('\n') and len(reply.encode()) <= LIMIT, 'Missing/bounded peer response'
        return json.loads(reply)

    def ok(self, op, **data):
        reply = self.request(op, **data)
        assert 'error' not in reply, (op, reply)
        return reply

    def close(self):
        self.process.stdin.close()
        try:
            assert self.process.wait(timeout=10) == 0
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise


def exchange(legacy, current, legacy_creator):
    old, new = Peer(legacy), Peer(current)
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
    finally:
        old.close()
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
