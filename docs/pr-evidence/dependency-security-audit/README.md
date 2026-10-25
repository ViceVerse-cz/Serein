# Dependency security evidence — October 25, 2026

Base: freshly fetched upstream main `38e46b8f5ec694002ef515fbc2d8dc69297e4f5f`.
Use the repository's pinned Rust 1.98.1, cargo-audit 0.22.2 and cargo-deny 0.20.2.

`audit.json` summarizes original/final audits against the same RustSec database
revision `21d909e4c8bd4e14c7aea58bf980c881a0053ab1` (1,352 advisories).
To reproduce, check out that advisory database revision and run, from this repo:

```sh
cargo audit --deny warnings --db <advisory-db-checkout> --no-fetch
```

For the baseline, extract `Cargo.lock` from the base commit and repeat with
`--file <baseline-lockfile>`, preserving this repository's unchanged audit config.
The baseline fails with five vulnerabilities and two denied warnings; the final
lockfile passes with neither. A future database or registry yank state can add
findings. Existing maintenance exceptions are recorded explicitly in the JSON;
none were added for this change.

`measurements.json` records newly built standard packages from this task, including
voice, using `cargo xtask package --format arch`. Installed-tree bytes sum regular
files produced by `python3 packaging/linux/package.py dist 0.1.0 --format dir`.
Each revision has one artifact sample; archive timestamps/compression introduce
noise. See [performance notes](../../performance.md#dependency-security-migration--october-25-2026)
for the delta and limits. No timing or memory benchmark is inferred from file size.

The [legacy DAVE peers](../../../crates/discord-voice/tests/fixtures/legacy-dave/README.md)
provide reproducible old/new offline wire/media compatibility evidence. Ordinary
workspace tests include the signed legacy key-package/own-commit regression and
synthetic local encrypted voice/video exchange. No live Discord, credentials,
audio devices or independent cryptographic assessment are involved.
