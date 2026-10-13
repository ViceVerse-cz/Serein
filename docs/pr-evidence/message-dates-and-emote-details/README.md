# Message dates and emote details

Synthetic, offline Windows native evidence. Baseline is main `9433275fa339a5a609ec8d0ed59c22e0494ec2d4`; changed application source is `4f90d6c4`. Builds use the pinned Rust 1.98.1 lockfile. Native screenshots use `cargo build --locked -p serein --features demo`, followed by `serein.exe --demo --demo-chat`.

Both date screenshots and both emoji hover screenshots use the same 1120x760 native window and fixture. The custom emoji example is typed and sent only into the local synthetic chat: `🧂 <:serein_wave:9001> <a:serein_party:9002>`. The changed tooltip renders artwork and shortcode; animation was inspected across captures. Orange/teal squares are deliberately synthetic fixture artwork.

Additional native inspection covered a narrower chat with the member pane collapsed and the light sticker picker/detail. Selecting a local fixture sticker and clicking View More Stickers opened the known synthetic server's picker. Long names/descriptions at 280x520 were verified by existing automated pointer interaction in both themes, rather than a claimed native minimum-size screenshot.

`package.json` records fresh standard voice-inclusive release packages built with `cargo xtask package` on both sources; no demo/developer feature. NSIS is unavailable, so these are unsigned directory/ZIP packages. ZIP sizes use the same .NET Optimal compressor over all installed files. These include the shared ruzstd patch and cannot isolate its effect from UI changes.

The final UI suite with ruzstd 0.9.1 passed 482 tests, 8 ignored; the full local `cargo xtask check` passed. Synthetic fixtures and native screenshots do not prove live Discord interoperability. CPU/private-memory comparison is pending matched release-demo builds; frame and startup latency are unmeasured.
