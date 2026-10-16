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
const tapRepo = 'ViceVerse-cz/homebrew-serein';
const caskPath = 'Casks/serein.rb';
const tapToken = process.env.HOMEBREW_TAP_TOKEN;
if (plan) assert(tapToken, 'HOMEBREW_TAP_TOKEN is required to publish the Homebrew cask');

function checkTapAccess() {
  const push = execFileSync('gh', ['api', `repos/${tapRepo}`, '--jq', '.permissions.push'], {
    encoding: 'utf8', env: { ...process.env, GH_TOKEN: tapToken },
  }).trim();
  assert.equal(push, 'true', `HOMEBREW_TAP_TOKEN cannot push to ${tapRepo}`);
}

// A rerun after a failed cask update must not try to publish the release a second time.
function releaseExists() {
  try {
    execFileSync('gh', ['release', 'view', plan.gitTag], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}
const resumed = plan !== null && releaseExists();
if (plan && !resumed) checkTapAccess();

// The cask lives in its own tap repository so users do not clone this one. The tap file is the
// single source of truth; releases only rewrite its version and checksum.
function publishCask() {
  const asset = `release-assets/serein-${plan.gitTag}-macOS-ARM64.zip`;
  const checksum = createHash('sha256').update(readFileSync(asset)).digest('hex');
  const gh = args => execFileSync('gh', args, {
    encoding: 'utf8', env: { ...process.env, GH_TOKEN: tapToken },
  });
  const endpoint = `repos/${tapRepo}/contents/${caskPath}`;
  const current = JSON.parse(gh(['api', `${endpoint}?ref=main`]));
  const source = Buffer.from(current.content, 'base64').toString('utf8');
  assert.match(source, /^  version "[^"]+"$/m, 'Cannot locate Homebrew cask version');
  assert.match(source, /^  sha256 "[0-9a-f]{64}"$/m, 'Cannot locate Homebrew cask checksum');
  const updated = source
    .replace(/(^  version ")[^"]+("$)/m, `$1${plan.version}$2`)
    .replace(/(^  sha256 ")[^"]+("$)/m, `$1${checksum}$2`);
  if (updated === source) return;
  gh([
    'api', endpoint, '--method', 'PUT',
    '--field', `message=chore(release): update cask for ${plan.version}`,
    '--field', `content=${Buffer.from(updated).toString('base64')}`,
    '--field', `sha=${current.sha}`, '--field', 'branch=main',
  ]);
}
const conventional = { preset: 'conventionalcommits' };
const plugins = [
  [require.resolve('@semantic-release/commit-analyzer'), conventional],
  [{ generateNotes: (config, context) => generateReleaseNotes(channel, config, context) }, conventional],
];
if (plan && channel === 'production') {
  plugins.push(
    [require.resolve('@semantic-release/git'), {
      assets: ['Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock', 'packaging/macos/Info.plist'],
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
const result = mode === 'publish' && (channel === 'nightly' || resumed)
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
    if (!resumed) {
      writeFileSync('target/release-notes.md', plan.notes);
      const assets = readdirSync('release-assets')
        .map(name => `release-assets/${name}`);
      execFileSync('gh', ['release', 'create', plan.gitTag, ...assets,
        '--target', plan.gitHead, '--title', `Serein ${plan.version}`,
        '--notes-file', 'target/release-notes.md', '--prerelease', '--latest=false', '--draft'],
      { stdio: 'inherit' });
    }
    execFileSync('gh', ['release', 'edit', plan.gitTag, '--draft=false', '--prerelease', '--latest=false'],
    { stdio: 'inherit' });
  }
  publishCask();
}
