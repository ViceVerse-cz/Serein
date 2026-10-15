// Exercise the prebuilt xtask with synthetic Cargo output; no compiler or native app runs.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

if (process.platform !== 'linux') {
  console.log('xtask packaging command fixture requires Linux; no packaging performed.');
  process.exit(0);
}
const repo = path.resolve(__dirname, '..');
const binary = path.resolve(process.env.SEREIN_XTASK_BINARY || path.join(
  process.env.CARGO_TARGET_DIR || path.join(repo, 'target'), 'debug', 'xtask'));
assert.ok(fs.existsSync(binary), 'Build xtask first (cargo xtask check)');
const parent = fs.realpathSync(os.tmpdir());
const fixture = fs.mkdtempSync(path.join(parent, 'serein-xtask-package-'));
try {
  const commands = path.join(fixture, 'commands');
  fs.mkdirSync(commands);
  fs.writeFileSync(path.join(fixture, 'Cargo.toml'), '[workspace]\n[workspace.package]\nversion="9.8.7"\n');
  fs.mkdirSync(path.join(fixture, '.cargo'));
  fs.writeFileSync(path.join(fixture, '.cargo/config.toml'), '[build]\ntarget-dir="configured target"\n');
  for (const name of ['README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_NOTICES.md',
    'assets/sounds/README.md', 'assets/fonts/NotoSansCJK-LICENSE.txt', 'assets/fonts/NotoSansArabic-OFL.txt',
    'assets/fonts/NotoSansMath-OFL.txt', 'assets/fonts/NotoSansSymbols2-OFL.txt',
    'assets/fonts/Inter-OFL.txt', 'assets/twemoji/LICENSE-GRAPHICS',
    'assets/twemoji/LICENSE-UNICODE', 'assets/icons/LICENSE', 'assets/icons/LICENSE-SIMPLE-ICONS']) {
    fs.mkdirSync(path.dirname(path.join(fixture, name)), { recursive: true });
    fs.copyFileSync(path.join(repo, name), path.join(fixture, name));
  }
  fs.cpSync(path.join(repo, 'assets/licenses'), path.join(fixture, 'assets/licenses'), { recursive: true });
  const source = path.join(fixture, 'configured target', 'x86_64-unknown-linux-gnu', 'release', 'serein');
  fs.mkdirSync(path.dirname(source), { recursive: true });
  fs.writeFileSync(source, 'CURRENT Cargo-reported executable');
  const staleTarget = path.join(fixture, 'target');
  fs.mkdirSync(path.join(staleTarget, 'release'), { recursive: true });
  fs.writeFileSync(path.join(staleTarget, 'release/serein'), 'STALE previous executable');
  const packageId = `path+file://${fixture}/apps/desktop#serein@9.8.7`;
  const metadata = path.join(fixture, 'metadata.json');
  fs.writeFileSync(metadata, JSON.stringify({ workspace_members: [packageId],
    packages: [{ id: packageId, name: 'serein', version: '9.8.7' }] }));
  const messages = path.join(fixture, 'messages.jsonl');
  const pythonLog = path.join(fixture, 'python.log');
  const artifact = { reason: 'compiler-artifact', package_id: packageId, target: { name: 'serein', kind: ['bin'] },
    profile: { test: false }, executable: source };
  const foreign = { ...artifact, package_id: 'path+file:///unrelated#serein@1.2.3',
    executable: path.join(staleTarget, 'release/serein') };
  const cargo = path.join(commands, 'cargo');
  fs.writeFileSync(cargo, '#!/bin/sh\nset -eu\ncase "$1" in\n'
    + 'locate-project) printf "%s\\n" "$SEREIN_FIXTURE_MANIFEST";;\n'
    + 'metadata) cat "$SEREIN_FIXTURE_METADATA";;\n'
    + 'build) printf "%s\\n" "$*" > "$SEREIN_FIXTURE_BUILD_ARGS"; cat "$SEREIN_FIXTURE_MESSAGES"; exit "${SEREIN_FIXTURE_BUILD_EXIT:-0}";;\n'
    + '*) exit 99;;\nesac\n', { mode: 0o755 });
  fs.writeFileSync(path.join(commands, 'python3'), '#!/bin/sh\nset -eu\nprintf "%s\\n" "$*" >> "$SEREIN_FIXTURE_PYTHON_LOG"\n', { mode: 0o755 });
  const env = { ...process.env, PATH: commands + path.delimiter + process.env.PATH,
    CARGO_NET_OFFLINE: 'true', SEREIN_FIXTURE_MANIFEST: path.join(fixture, 'Cargo.toml'),
    SEREIN_FIXTURE_METADATA: metadata, SEREIN_FIXTURE_MESSAGES: messages,
    SEREIN_FIXTURE_BUILD_ARGS: path.join(fixture, 'build-args'), SEREIN_FIXTURE_PYTHON_LOG: pythonLog };
  function check(records, overrides = {}) {
    fs.rmSync(path.join(fixture, 'dist'), { recursive: true, force: true });
    fs.rmSync(pythonLog, { force: true });
    fs.writeFileSync(messages, records.map(record => JSON.stringify(record)).join('\n') + '\n');
    const childEnv = { ...env, ...overrides };
    for (const key of Object.keys(childEnv)) if (childEnv[key] === null) delete childEnv[key];
    const result = spawnSync(binary, ['package', '--format', 'dir'], { cwd: fixture,
      env: childEnv, encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 });
    assert.ifError(result.error);
    assert.equal(result.signal, null);
    return { ...result, output: result.stdout + result.stderr };
  }
  for (const label of ['explicit native target', 'configured output directory']) {
    const overrides = label === 'explicit native target'
      ? { CARGO_TARGET_DIR: staleTarget, CARGO_BUILD_TARGET: 'x86_64-unknown-linux-gnu' }
      : { CARGO_TARGET_DIR: null, CARGO_BUILD_TARGET: null };
    const result = check([foreign, artifact], overrides);
    assert.equal(result.status, 0, result.output);
    assert.equal(fs.readFileSync(path.join(fixture, 'dist/serein'), 'utf8'), 'CURRENT Cargo-reported executable', label);
    assert.match(fs.readFileSync(pythonLog, 'utf8'), /package\.py dist 9\.8\.7 --format dir/);
    assert.match(fs.readFileSync(path.join(fixture, 'build-args'), 'utf8'), /--message-format=json-render-diagnostics/);
  }
  for (const [label, records, overrides] of [
    ['missing executable', [foreign], {}],
    ['test executable', [{ ...artifact, profile: { test: true } }], {}],
    ['duplicate executable', [artifact, artifact], {}],
    ['failed build', [artifact], { SEREIN_FIXTURE_BUILD_EXIT: '1' }],
  ]) {
    const result = check(records, overrides);
    assert.equal(result.status, 1, label + '\n' + result.output);
    assert.ok(!fs.existsSync(path.join(fixture, 'dist/serein')), label);
    assert.ok(!fs.existsSync(pythonLog), label);
  }
  console.log('Cached xtask packages the reported artifact and runtime version; missing/ambiguous/failed builds rejected (synthetic).');
} finally {
  assert.equal(path.dirname(fs.realpathSync(fixture)), parent);
  fs.rmSync(fixture, { recursive: true });
}
