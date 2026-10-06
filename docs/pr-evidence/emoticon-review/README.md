# Emoticon review fixes — offline evidence

Baseline: rebased PR commit `20c2bb5fb6940e8f2ac3c84a25e9263457e658e1`, based on main
`ec75f729b11d5f3acca36b3563ee8db492b220aa`. The changed runtime files are identified
by SHA-256 in `measurements.json`. All data is synthetic; no Discord account,
network adapter, message, upload, microphone or camera was used.

The before/after PNGs show the same successful converted edit (`Hello :)` sent as
`Hello 🙂`) in the native WGPU/Metal `profile_preview` harness. Before, the raw
editor remains open with save requested; after, the editor closes. Both are
1120 × 760 logical-pixel requests at the host display scale. The framebuffer is
1920 × 1303 pixels. The temporary fixture uses the production composer Enter
path and a correlated synthetic `Event::Edited` response; it changes no
production logic. The capture hook is removed byte-exactly before packaging.
The after capture was inspected and its idle process stopped explicitly.

To reproduce in disposable baseline and changed worktrees, for each worktree:

```sh
python3 /absolute/path/capture-hooks.py WORKTREE
cd WORKTREE
cargo build --locked -p serein --no-default-features --features demo --example profile_preview
./target/debug/examples/profile_preview --demo --page=emoticon-edit --width=1120 --height=760 --output=/absolute/path/result.png
python3 /absolute/path/capture-hooks.py WORKTREE restore
```

The hook exists solely to reproduce these captures. Do not apply it to a checkout
with unrelated edits to the two files it temporarily changes. Restoring uses
adjacent backups and removes them.

The standalone release converter benchmark compiles each worktree's exact
`emoticons.rs` with the pinned `rustc -O`, `black_box`, and `Instant`. It has no
external dependencies. One warmup and five alternating measured batches each
convert 100,000 messages; inputs are 1280, 1710 and 1160 bytes. It measures
submission conversion, not UI frames, process memory or Discord latency. The
recorded samples were taken on a shared host with compiler activity; they are
observational timings and support no performance improvement claim.

```sh
python3 docs/pr-evidence/emoticon-review/benchmark.py BASELINE_WORKTREE CHANGED_WORKTREE
```

Run `cargo xtask package` on both revisions with identical normal release
settings and no default/demo features. This includes voice. Packaging was stopped during baseline linking at the user’s request; no package
size comparison or successful standard release-package verification is claimed.
For future measurements, sum regular files in each complete `dist` directory. Distribution ZIPs include
all those files in sorted order using `ZIP_DEFLATED` level six. Local ad-hoc
signing is not Developer ID signing or notarization.

Verification: workspace/fuzz formatting, strict workspace/all-target Clippy,
policy, seven emoticon UI/converter tests, three existing inline-edit tests and
one SQLite preference test pass. Full `cargo xtask check` stops at inherited UI
failures. The Appearance missing-label failure and download-cancel assertion /
texture-cleanup SIGABRT also reproduce on the rebased baseline before these
fixes; this work does not repair those unrelated tests. Synthetic captures do
not establish live Discord compatibility.
