// Fork release planner: every push to main ships a nightly. Upstream's semantic-release
// flow only cuts releases for conventional feat/fix commits; fork users want every
// delivered change, so the nightly run number keeps each build's version unique.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';

assert.equal(process.env.GITHUB_REF_NAME, 'main', 'Release from main only');
const git = args => execFileSync('git', args, { encoding: 'utf8' }).trim();
const cargoVersion = readFileSync('Cargo.toml', 'utf8')
	.match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)?.[1];
assert(cargoVersion, 'Cannot read the workspace version');

// Manifests only move on production releases, so tags are the source of truth for the
// highest stable base already shipped; a stale manifest must not produce older nightlies.
const tags = git(['tag', '--list', 'v*', '--sort=-v:refname', '--merged', 'HEAD'])
	.split('\n')
	.filter(Boolean);
const stableOf = tag => tag.replace(/^v/, '').replace(/-nightly\.\d+\.\d+$/, '');
const parts = version => version.split('.').map(Number);
const newer = (a, b) => {
	for (let index = 0; index < 3; index++) {
		const diff = (parts(a)[index] ?? 0) - (parts(b)[index] ?? 0);
		if (diff !== 0) return diff > 0;
	}
	return false;
};
let stable = cargoVersion;
for (const tag of tags) {
	const candidate = stableOf(tag);
	if (/^\d+\.\d+\.\d+$/.test(candidate) && newer(candidate, stable)) stable = candidate;
}

const date = new Date().toISOString().slice(0, 10).replace(/-/g, '');
const run = process.env.GITHUB_RUN_NUMBER || '1';
const version = `${stable}-nightly.${date}.${run}`;
const gitTag = `v${version}`;
const gitHead = git(['rev-parse', 'HEAD']);

const range = tags[0] ? `${tags[0]}..HEAD` : 'HEAD';
const lines = git(['log', range, '--no-merges', '--format=%s%x09%h', '-n', '100'])
	.split('\n')
	.filter(Boolean);
const notes = lines.length
	? `### Changes\n\n${lines
			.map(line => {
				const tab = line.indexOf('\t');
				const subject = tab === -1 ? line : line.slice(0, tab);
				const sha = tab === -1 ? '' : line.slice(tab + 1);
				return `* ${subject}${sha ? ` (${sha})` : ''}`;
			})
			.join('\n')}\n`
	: '### Changes\n\n* Nightly build\n';

mkdirSync('target', { recursive: true });
writeFileSync(
	'target/release-plan.json',
	JSON.stringify({ version, stableVersion: stable, gitTag, gitHead, notes, channel: 'nightly' }),
);
if (process.env.GITHUB_OUTPUT) {
	appendFileSync(process.env.GITHUB_OUTPUT, `tag=${gitTag}\nversion=${version}\n`);
}
console.log(`Planned ${gitTag} at ${gitHead.slice(0, 12)}`);
