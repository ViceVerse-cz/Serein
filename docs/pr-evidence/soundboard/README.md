# Soundboard evidence

Baseline: `a7d57c6846c3eef41ecd8825b08bb413464b3309` (`origin/main` when work began), built in an
untouched separate worktree. Both are debug builds with `--features demo`, run as real Serein X11
windows on a dedicated 1360 × 1040 Xvfb display at scale 1.0 and cropped to the 1120 × 760 window.
No account, Discord connection, microphone, download or playback was used.

- `before.png`: `--demo --demo-voice`. The Voice Connected card has camera, screen share and
  noise suppression.
- `after-closed.png`: the same scene on this branch. The soundboard button (music notes) sits
  next to noise suppression.
- `after.png`: `--demo --demo-soundboard` opens the panel over the same synthetic call with
  synthetic catalogs: search, Default sounds, the server's sounds (one marked unavailable), a
  speaker button on each sound for a local-only preview, and Soundboard volume. Sounds and
  previews are disabled because the offline preview never plays or downloads anything; hovering
  one says so.
- `after-dm.png`: `--demo --demo-call --demo-soundboard`, the same panel in a private call. A
  private call has no server section; server sounds there count as other servers' sounds.

To reproduce:

```sh
cargo build --locked -p serein --features demo --bin serein
DISPLAY=:99 target/debug/serein --demo --demo-soundboard
```
