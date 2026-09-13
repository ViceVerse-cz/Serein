# Notification sounds

Suno-generated tracks supplied by the repository owner on September 13, 2026
for Serein's three notification settings. These are embedded in the executable;
playback does not read Downloads or fetch remote assets.

| Bundled track | Supplied filename | Source SHA-256 |
| --- | --- | --- |
| `message.mp3` | Serein Notification Chime.mp3 | `3735466410f8c9dc193fca7216908d1b9c635857fbc82964a0491bb76753c715` |
| `current-channel.mp3` | In-Chat Droplet Notification.mp3 | `8b15c10ba197d1e625fa4bd0200fab4d55969fd220c8012db55e6b61ee0f63be` |
| `incoming-ring.mp3` | Serein Incoming Call Notification.mp3 | `610ede278fab8fe2c50b2150e7dfb73762083c56018b79875208f923adb17c13` |

Embedded artwork and metadata were removed without re-encoding or trimming audio:

```sh
ffmpeg -i input.mp3 -map 0:a:0 -c:a copy -map_metadata -1 -id3v2_version 0 -write_id3v1 0 output.mp3
```

FFmpeg decoded-PCM SHA-256 comparisons matched each original and bundled track.
The three MP3 files total 106,608 bytes. All use 48 kHz stereo; the incoming ring
is approximately four seconds and must play in full before repeating.
