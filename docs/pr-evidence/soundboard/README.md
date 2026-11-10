# Soundboard evidence

These screenshots are the original feature evidence carried from commit
`2b3eb170ac2e2814c11d0149571f0468f14efa91`; they were inspected when rebasing the feature onto
`bedb57c0c8def62e736e8a4f03e0a5d248b22126`. They are not new captures of the rebased patch.
The deadline and live-volume fixes do not change the panel layout. Their regression tests use
a paused Tokio clock with loopback HTTP admission and synthetic PCM mixer ticks respectively;
neither fixture accesses Discord or physical audio devices.

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

To reproduce the review regressions offline:

```sh
cargo test --locked -p discord-api soundboard::tests::expired_sends_cancel_admission_without_sending_later
cargo test --locked -p discord-voice soundboard::tests::current_volume_controls_playing_and_newly_admitted_clips
```

The first case holds REST work behind a shared cooldown, exhausted permits and same-route
admission, advances the clock beyond the original send deadline, then removes each blocker.
The expired send must release its capacity and leave the loopback listener without a POST.
The second plays PCM, changes the level to zero during playback, admits a newly available clip
while muted, then restores the level. Participant audio is unchanged by the soundboard slider;
muted clips advance and cannot resume from an old position later.
