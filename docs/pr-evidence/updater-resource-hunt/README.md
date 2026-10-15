# Historical resource cleanup — October 8, 2026

Historical evidence from the original combined PR, separated by scope. The recorded revision labels, hashes and measurements are unchanged; these results do not validate the new branch heads.

Baseline: `98158b4106588df7a14bcc1d9ecc90d71ac1f931`. After source SHA-256 labels, original measurements and test logs are in [measurements.json](measurements.json) and [checks.log](checks.log). Debian 13 x86_64, pinned Rust 1.98.1, synthetic offline inputs; no account, capture device, physical GPU or live installer was used.

A successfully queued updater result lost cleanup ownership if the app closed before polling. The result guard cleans staged files or stops/reaps the helper off the UI thread unless the result was accepted. Baseline regressions left one directory and one helper; both cleanup checks pass after the fix. Registered-game normalization now rejects Unicode expansion over 256 bytes. The xtask packaging fixture includes the upstream Symbols2 font license.

Reproduce with `python3 docs/pr-evidence/updater-resource-hunt/updater-scope.py test` (or `clippy`), locked model/core/UI tests and `node tests/xtask-package.cjs`. The updater harness compiles production updater/delta/install modules, replacing only eframe’s rendering boundary with an egui re-export. It cannot establish full desktop integration. Historical eight-test counts cover both scopes before the split. Full executable/package/CPU/RSS and native integration were unavailable because GLib development metadata was missing.
