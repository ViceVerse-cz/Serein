// Offline release smoke; never invokes semantic-release publishing or Apple services.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { load } from 'js-yaml';
import { analyzeCommits } from '@semantic-release/commit-analyzer';
import { generateNotes } from '@semantic-release/release-notes-generator';
import getLastRelease from './node_modules/semantic-release/lib/get-last-release.js';

const logger = { log() {} };
for (const [message, expected] of [
  ['fix(ui): fix scrolling', 'patch'], ['feat(ui): add search', 'minor'],
  ['feat!: change storage format', 'major'], ['docs: update guide', null],
]) {
  assert.equal(await analyzeCommits({ preset: 'conventionalcommits' }, {
    cwd: process.cwd(), logger, commits: [{ message, hash: 'abcdef1' }],
  }), expected);
}
assert.equal(getLastRelease({
  branch: { type: 'release', tags: [
    { version: '1.1.0-nightly.9.1', gitTag: 'v1.1.0-nightly.9.1', channels: [null] },
    { version: '1.0.0', gitTag: 'v1.0.0', channels: [null] },
  ] }, options: { tagFormat: 'v${version}' },
}).version, '1.0.0');
const notes = await generateNotes({ preset: 'conventionalcommits' }, {
  cwd: process.cwd(), logger, options: { repositoryUrl: 'https://github.com/example/serein' },
  commits: [{ message: 'fix(ui): fix scrolling', hash: 'abcdef1' }],
  lastRelease: { gitTag: 'v1.0.0' }, nextRelease: { version: '1.0.1', gitTag: 'v1.0.1' },
});
assert(notes.includes('fix scrolling'));
const workflow = load(readFileSync('.github/workflows/release.yml', 'utf8'));
assert.deepEqual(Object.keys(workflow.on), ['workflow_dispatch']);
for (const job of Object.values(workflow.jobs)) {
  for (const step of job.steps ?? []) {
    if (step.run && step.shell !== 'pwsh') execFileSync('bash', ['-n'], { input: step.run });
  }
}
execFileSync('bash', ['-n', 'packaging/macos/sign-release.sh']);
execFileSync('bash', ['packaging/macos/test_sign_release.sh'], { stdio: 'inherit' });
execFileSync(process.env.PYTHON || 'python3', ['-c', `
import glob, os, pathlib, shutil, subprocess, tempfile, tomllib
root = pathlib.Path.cwd()
manifest = tomllib.loads((root / 'Cargo.toml').read_text())
with tempfile.TemporaryDirectory() as directory:
    fixture = pathlib.Path(directory)
    paths = ['Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock', 'packaging/macos/Info.plist']
    for member in manifest['workspace']['members']:
        paths.extend(glob.glob(member + '/Cargo.toml'))
    for path in paths:
        (fixture / path).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / path, fixture / path)
    os.chdir(fixture)
    for version in ['1.2.3-nightly.9.1', '1.2.3']:
        subprocess.run([os.sys.executable, str(root / '.github/release/version.py'), version], check=True)
        assert tomllib.loads(pathlib.Path('Cargo.toml').read_text())['workspace']['package']['version'] == version
        for lock in ['Cargo.lock', 'fuzz/Cargo.lock']:
            packages = tomllib.loads(pathlib.Path(lock).read_text())['package']
            original = tomllib.loads((root / lock).read_text())['package']
            assert [p for p in packages if 'source' in p] == [p for p in original if 'source' in p]
            assert next(p['version'] for p in packages if p['name'] == 'model' and 'source' not in p) == version
    import plistlib
    assert plistlib.loads(pathlib.Path('packaging/macos/Info.plist').read_bytes())['CFBundleShortVersionString'] == '1.2.3'
`], { stdio: 'inherit' });
console.log('Release smoke passed: semantic bumps/notes, nightly isolation, manifest/lock versions, YAML and shell syntax.');
