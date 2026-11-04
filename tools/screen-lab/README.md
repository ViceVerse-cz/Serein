# Screen-share lab

Offline tools for measuring Serein's Linux screen sharing and stream viewing on a disposable
headless wlroots compositor (developed on labwc's headless backend behind wayvnc, Fedora Asahi,
Apple M1). Nothing here joins Discord, opens a microphone or sends network traffic. Only the
synthetic scenes below are captured; recordings stay in a path you choose.

## Sender: capture and encode

```sh
cargo build --release -p discord-voice -p platform --features winit/wayland \
  --example screen_bench --example stream_view_bench --example screen_probe
tools/screen-lab/lab.sh portal          # unattended xdg-desktop-portal-wlr (first output, no chooser)
tools/screen-lab/lab.sh scene text      # idle | text | motion | stress
tools/screen-lab/lab.sh bench --size 1080 --fps 30 --seconds 15
tools/screen-lab/lab.sh matrix OLD/screen_bench NEW/screen_bench   # scenes x presets table
```

`screen_bench` runs the production capture/encode worker through the portal and reports frame
rate, frame-gap percentiles, bitrate, per-thread CPU, RSS and teardown time. `--out REC.au`
records the encoded stream (little-endian u32 length + Annex B access unit) for the viewer
tools. `screen_probe SECONDS "GSTREAMER TAIL"` runs any pipeline tail on a real portal session.

Scenes: `text` scrolls highlighted source code (sharp, every frame changes), `motion` moves a
ball over a static picture, `stress` moves full-screen bars with a noise block (beyond any
bitrate, so it shows encoder frame skipping) and `idle` leaves the desktop unchanged.

## Viewer: decode, upload and repaint

```sh
target/release/examples/stream_view_bench REC.au --fps 30
target/release/serein --demo --demo-call-stream --demo-stream-file=REC.au --demo-stream-fps=30
```

`stream_view_bench` replays a recording through the platform live decoder used for watched
streams and through OpenH264, reporting decode latency and CPU. The demo replay (build with
`--features demo`) sends the recording through that decoder, the watch slot and the GPU upload
in the real app, and logs `decoded/s`, `uploads/s` and `ui_passes/s` to stderr every 5 s.

## Known portal behaviour

xdg-desktop-portal-wlr 0.8.4 with PipeWire 1.6 negotiates two buffers with Serein. In roughly
one start in eight to one in three, the stream renegotiates during start-up, the consumer is
never scheduled again ("out of buffers" in the backend log) and only the one-second keepalive
arrives. Unchanged `main` does this too. Requesting three or more buffers instead makes the
backend exit on an `ext_image_copy_capture` protocol error. `lab.sh bench` restarts the backend
before every attempt and retries a moving scene that never got going; read results knowing
this is a lab compositor, not GNOME, KDE or a hardware encoder.
