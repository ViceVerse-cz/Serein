# Synthetic server settings polish

Rendered with the native egui/WGPU offline preview using the repository's demo
fixture. No account session is loaded and no Discord request is made.

```sh
cargo run --locked -p serein --example profile_preview --features demo -- \
  --demo --page=server-safety --output=after-safety.png --width=1320 --height=860
```

Pages: `server`, `server-engagement`, `server-safety`, `server-emoji`,
`server-members`, `server-invites`, `server-audit-log` (add `--light` for the
light theme). The `before-*` images come from base commit `eab1961` with only
the preview harness extended to reach those pages; Safety Setup did not exist
there, so it has no before image. Images were downscaled to 1600 px wide. They
verify native layout only, not live Discord interoperability. The rail, switch,
sidebar and page-fade animations are motion and are not captured in stills.
