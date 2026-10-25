// Offline policy fixtures only. Node is never a messaging runtime dependency.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const config = path.resolve(__dirname, '..', 'deny.toml');
assert.ok(fs.existsSync(config), 'Repository deny.toml must exist');
const tempParent = fs.realpathSync(os.tmpdir());
const fixtureRoot = fs.mkdtempSync(path.join(tempParent, 'serein-license-policy-'));

function cargo(args, cwd) {
  const result = spawnSync('cargo', args, {
    cwd,
    encoding: 'utf8',
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 1024 * 1024,
    env: { ...process.env, CARGO_NET_OFFLINE: 'true' },
  });
  assert.ifError(result.error);
  assert.equal(result.signal, null, 'Cargo must finish normally');
  return { status: result.status, output: `${result.stdout}\n${result.stderr}` };
}

const cases = [
  ['permissive-or', 'fixture-permissive', '1.0.0', 'MIT OR Apache-2.0', true],
  ['restrictive-and', 'fixture-conjunctive', '1.0.0', 'MIT AND GPL-3.0-only', false],
  ['missing-license', 'fixture-unlicensed', '1.0.0', null, false],
  ['gpl-license', 'fixture-gpl', '1.0.0', 'GPL-3.0-only', false],
  ['exact-exception', 'hpke-rs', '0.8.0', 'MPL-2.0', true],
  ['wrong-exception-version', 'hpke-rs', '0.8.1', 'MPL-2.0', false],
];

try {
  for (const [label, name, version, license, allowed] of cases) {
    const dir = path.join(fixtureRoot, label);
    const dependency = path.join(dir, 'dependency');
    fs.mkdirSync(path.join(dir, 'src'), { recursive: true });
    fs.mkdirSync(path.join(dependency, 'src'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'src', 'lib.rs'), '// Synthetic offline root.\n');
    fs.writeFileSync(path.join(dependency, 'src', 'lib.rs'), '// Synthetic offline dependency.\n');
    fs.writeFileSync(path.join(dir, 'Cargo.toml'), `[package]
name = "license-policy-root"
version = "1.0.0"
edition = "2021"
license = "MIT"
publish = false

[workspace]
exclude = ["dependency"]

[dependencies]
candidate = { package = "${name}", path = "dependency", optional = true }

[features]
policy-fixture = ["dep:candidate"]
`);
    fs.writeFileSync(path.join(dependency, 'Cargo.toml'), `[package]
name = "${name}"
version = "${version}"
edition = "2021"
${license === null ? '' : `license = "${license}"\n`}`);
    // No LICENSE file is created: the missing-license case cannot pass via file discovery.
    const lock = cargo(['generate-lockfile', '--offline'], dir);
    assert.equal(lock.status, 0, `${label}: offline lockfile failed\n${lock.output.slice(0, 12000)}`);
    const result = cargo([
      'deny', '--locked', '--offline', '--all-features', '--manifest-path', path.join(dir, 'Cargo.toml'),
      '--config', config, 'check', 'licenses',
    ], dir);
    const detail = `${label}: unexpected license policy result\n${result.output.slice(0, 12000)}`;
    if (allowed) {
      assert.equal(result.status, 0, detail);
    } else {
      assert.notEqual(result.status, 0, detail);
      assert.match(result.output, /error\[(rejected|unlicensed)\]/, detail);
      assert.ok(result.output.includes(name) && /licen[sc]e/i.test(result.output), detail);
    }
  }
  console.log('License policy: six offline fixtures passed, including optional path dependencies and the exact MPL exception.');
} finally {
  const resolved = fs.realpathSync(fixtureRoot);
  assert.equal(path.dirname(resolved), tempParent, 'Cleanup must stay inside the temporary parent');
  assert.ok(path.basename(resolved).startsWith('serein-license-policy-'), 'Cleanup must target this fixture directory');
  fs.rmSync(resolved, { recursive: true, force: true });
}
