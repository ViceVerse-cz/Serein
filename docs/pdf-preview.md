# Native PDF attachment preview

PDF attachments stay file cards until the user selects **Preview**. The native
egui modal shows one bounded raster page, Previous/Next, Left/Right arrow keys,
Fit page, fit-relative zoom (1–2×), scrolling, Download and explicit Retry after
an error. Escape/backdrop/close dismiss it. There is no automatic PDF fetch or
browser viewer, and the renderer cannot activate PDF links/actions/JavaScript,
forms or media. Text selection/search and password entry are not implemented;
password-protected or unsupported files retain the original-download fallback.

## Data and worker boundary

- Metadata selects PDF by exact MIME or case-insensitive final `.pdf` suffix.
  A PDF cannot also select the audio/video/image preview path.
- Fetch accepts only the existing validated original attachment CDN URL for its
  attachment ID. No token/cookie/auth header, media proxy, redirect, user-selected
  proxy or content decompression is used. Fetch is cancellation-aware with a
  ten-second total timeout. Declared and streamed lengths must match within 16 MiB;
  content type must be PDF/generic binary (or absent), and bytes need PDF magic.
- One lazy latest-value worker retains one document and one current result.
  Rapid page requests coalesce; there is no growing queue or speculative render.
- The worker passes at most 16 MiB to a short-lived same-executable
  `--serein-pdf-helper` instance before GUI/auth/voice startup. A fixed protocol
  validates lengths/page counts/dimensions before parent allocations. Pipe work,
  parsing, rasterization and ColorImage conversion never run in UI/audio callbacks.
- The helper uses pinned experimental hayro 0.8.0, bundled standard fonts/CMaps
  and no custom system/file/network font resolver. It admits 256 pages, finite
  positive page boxes, rotation/crop, at most 1,536 pixels per edge and 1,572,864
  pixels per page. Raster scale is at most two pixels per PDF point.
- Helper CPU is capped at eight seconds. Parent and independent helper watchdogs
  each enforce ten-second wall lifetime. Linux/Windows limit helper memory to
  768 MiB; macOS uses 20-ms resident-memory polling with possible overshoot.
  See [platform support](platform-support.md#native-pdf-attachments).
- Cancellation kills/reaps and joins pipe I/O before replacement. Failed cleanup
  permanently retires that worker, not a detached or overlapping renderer.
  Close/logout/source changes release RAM state; originals/rasters are not persisted.
  See [storage policy](storage-policy.md#native-pdf-preview-october-11-2026).

Resource isolation is **not an OS sandbox**, a security certification or a guarantee
against parser vulnerabilities. Upstream unsafe code and renderer compatibility
remain risks. Documents above these limits should use a dedicated reader.

## Offline reproduction and tests

Use the repository-pinned toolchain and lockfile:

```sh
cargo test --locked -p model -p ui -p serein pdf
cargo test --locked -p serein --test pdf_helper
cargo xtask check
cargo xtask package
cargo run --release --locked -p serein --no-default-features --features demo -- --demo --demo-pdf
```

The two tiny fixtures in `apps/desktop/tests/fixtures` are generated synthetic
documents. The ordinary fixture has two pages: text/vector colors, and a 90-degree
rotated page with a 20-point crop margin. The password fixture is derived from it
with Ghostscript (synthetic `synthetic-reader` / `synthetic-owner` passwords only).
No personal PDF, Discord session, message, network or microphone is needed for
the demo/helper tests.

Regressions cover type precedence, URL/MIME/stream bounds, helper protocol/pixels,
crop/rotation/password/page limits, actual pre-GUI helper invocation, Linux
resource limits, timeout/cancel/reaping, coalesced navigation/cache retirement,
stale result rejection and UI source/account/permission/spoiler invalidation.
Modal tests cover keyboard consumption and light/dark short/narrow layouts.

Before/after evidence and measurements are in
[`pr-evidence/native-pdf-preview`](pr-evidence/native-pdf-preview/README.md).
They are synthetic Linux evidence, not proof of Discord interoperability,
arbitrary PDF fidelity, Windows/macOS runtime coverage or screen-reader support.
