# Inline playback of SoundCloud track and audio file links

Baseline: `main` (`6d1fa13f`) plus only this branch's synthetic fixture and its `--demo-soundcloud`
flag, without the embed's `audio` field, which `main` does not read. Measured: this branch at the
commit that adds these images. Both are debug builds with demo support on Windows 11 at 100%
scale, window 1120×1250, captured with `PrintWindow`:

```sh
cargo run --locked -p serein --features demo -- --demo --demo-soundcloud --demo-accent=c084fc
```

The scene holds two messages: a SoundCloud track link (a `video` card whose video is SoundCloud's
hosted player page and whose thumbnail is the square artwork) and a direct link to an `.mp3` file
(an `audio` card).

- `before.png`: the track card shows the provider, uploader, title and description with the
  artwork as an 84 px side thumbnail, then "Video preview · playback opens in your browser" and
  "Open video…". The audio card shows its text and side thumbnail and "Additional embed content is
  not supported". Nothing plays.
- `after.png`: both cards draw the artwork as the card's image with a dark control at its centre
  (play, and open the link) and a player card beneath it, as wide as the artwork, with the play
  button, seek slider, time and volume. Neither browser line nor the "not supported" line is
  drawn. The timeline sits at its end, so the track card is cut off at the top.
- `after-playing.png`: 2.5 s after a native left click on the track artwork's play control.
- `mp3-after-playing.png`: 2.5 s after a native left click on the audio card's play control.

All content is synthetic. In `--demo` a card plays a bundled clip and makes no network request, so
these images show neither SoundCloud nor Discord's media proxy serving audio; the SoundCloud stream
pipeline was exercised against the real service separately, as described in the pull request. The
demo clip's duration is unknown before playback, so an idle card shows `0:00 / --:--`.
