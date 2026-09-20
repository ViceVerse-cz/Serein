import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { appendFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import semanticRelease from 'semantic-release';
import { generateReleaseNotes } from './notes.mjs';

const require = createRequire(import.meta.url);
const mode = process.argv[2];
assert(['plan', 'publish'].includes(mode), 'Use release.mjs plan|publish');
assert.equal(process.env.GITHUB_REF_NAME, 'main', 'Release from main only');
const channel = process.env.RELEASE_CHANNEL;
assert(['nightly', 'production'].includes(channel), 'Choose nightly or production');
const planPath = 'target/release-plan.json';
const plan = mode === 'publish' ? JSON.parse(readFileSync(planPath, 'utf8')) : null;
if (plan) assert.equal(channel, plan.channel, 'Release channel changed');
const caskPath = 'Casks/serein.rb';

function updateCask() {
  const asset = `release-assets/serein-${plan.gitTag}-macOS-ARM64.zip`;
  const checksum = createHash('sha256').update(readFileSync(asset)).digest('hex');
  const source = readFileSync(caskPath, 'utf8');
  assert.match(source, /^  version "[^"]+"$/m, 'Cannot locate Homebrew cask version');
  assert.match(source, /^  sha256 "[0-9a-f]{64}"$/m, 'Cannot locate Homebrew cask checksum');
  writeFileSync(caskPath, source
    .replace(/(^  version ")[^"]+("$)/m, `$1${plan.version}$2`)
    .replace(/(^  sha256 ")[^"]+("$)/m, `$1${checksum}$2`));
}

function commitNightlyCask() {
  const sha = execFileSync('git', ['rev-parse', `HEAD:${caskPath}`], { encoding: 'utf8' }).trim();
  execFileSync('gh', [
    'api', `repos/${process.env.GH_REPO}/contents/${caskPath}`, '--method', 'PUT',
    '--field', `message=chore(release): update Homebrew cask for ${plan.version} [skip ci]`,
    '--field', `content=${readFileSync(caskPath).toString('base64')}`,
    '--field', `sha=${sha}`, '--field', 'branch=main',
  ], { stdio: 'inherit' });
}
if (plan) updateCask();
const conventional = { preset: 'conventionalcommits' };
const plugins = [
  [require.resolve('@semantic-release/commit-analyzer'), conventional],
  [{ generateNotes: (config, context) => generateReleaseNotes(channel, config, context) }, conventional],
];
if (plan && channel === 'production') {
  plugins.push(
    [require.resolve('@semantic-release/git'), {
      assets: ['Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock', 'packaging/macos/Info.plist', caskPath],
      message: 'chore(release): ${nextRelease.version} [skip ci]',
    }],
    [require.resolve('@semantic-release/github'), {
      assets: ['release-assets/*'],
      successComment: false,
      failComment: false,
      releasedLabels: false,
      releaseNameTemplate: 'Serein <%= nextRelease.version %>',
    }],
  );
}
// Nightly assets are published from the immutable plan commit; no branch mutation needs semantic-release.
const result = mode === 'publish' && channel === 'nightly'
  ? { nextRelease: { version: plan.stableVersion, gitHead: plan.gitHead } }
  : await semanticRelease({
    branches: ['main'],
    tagFormat: 'v${version}',
    plugins,
    dryRun: mode === 'plan' || channel === 'nightly',
    verifyRelease: async (_config, { nextRelease }) => {
      if (plan) {
        assert.equal(nextRelease.version, plan.stableVersion, 'Release changed while packages were building');
        assert.equal(nextRelease.gitHead, plan.gitHead, 'Release commit changed while packages were building');
      }
    },
  });
if (mode === 'plan') {
  appendFileSync(process.env.GITHUB_OUTPUT, `release=${Boolean(result)}\n`);
  if (result) {
    const { version: stableVersion, gitHead, notes } = result.nextRelease;
    const date = new Date().toISOString().slice(0, 10).replace(/-/g, '');
    const runNumber = process.env.GITHUB_RUN_NUMBER || '1';
    const version = channel === 'nightly'
      ? `${stableVersion}-nightly.${date}.${runNumber}`
      : stableVersion;
    const gitTag = `v${version}`;
    mkdirSync('target', { recursive: true });
    writeFileSync(planPath, JSON.stringify({ version, stableVersion, gitTag, gitHead, notes, channel }));
    appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\ntag=${gitTag}\n`);
  }
} else {
  assert(result, 'No release published; branch or tags changed after planning');
  if (channel === 'nightly') {
    // Nightlies do not create stable tags or version commits that consume production changes.
    writeFileSync('target/release-notes.md', plan.notes);
    const assets = readdirSync('release-assets')
      .map(name => `release-assets/${name}`);
    execFileSync('gh', ['release', 'create', plan.gitTag, ...assets,
      '--target', plan.gitHead, '--title', `Serein ${plan.version}`,
      '--notes-file', 'target/release-notes.md', '--prerelease', '--latest=false', '--draft'],
    { stdio: 'inherit' });
    execFileSync('gh', ['release', 'edit', plan.gitTag, '--draft=false', '--prerelease', '--latest=false'],
    { stdio: 'inherit' });
    commitNightlyCask();
  }
}
