# Voice default-device host reuse — issue #569

The attached diagnostics contain 89 audio and 89 transport reports. At
`at_ms=275160`, audio resets rise to five per five-second report and continue
at that cadence. Transport mixing continues with no transport resets. This
matches the one-second default-device poll, but does not identify the exact
native device/server error. Only numeric aggregates are retained here.

Serein's baseline `884fb73f` creates a CPAL host on every poll. CPAL 0.18.2's
PulseAudio host creates a `pulseaudio::Client`; the pinned `pulseaudio` 0.3.1
reactor can remain blocked in `poll(None)` after the last client is dropped.
The channel-disconnection check runs only when the reactor wakes. The included
standalone probe reproduces this using synthetic Unix socket pairs and protocol
replies, without an audio server, devices, credentials or Discord connection.

```sh
cargo run --locked --release --manifest-path docs/pr-evidence/voice-host-reuse/Cargo.toml
```

The probe compares 300 fresh clients performing one query each with one retained
client performing 300 queries. Each reply is followed by a 2 ms pause to let the
reactor park, approximating the idle gap between actual one-second polls. It
counts peer sockets without EOF, then explicitly shuts down all synthetic peers.
This is a scheduling-sensitive dependency experiment, not a CI test or the
actual CPAL audio worker. One warmup and five measured runs on macOS 27/Apple M1,
16 GiB RAM, Rust 1.98.1 all retained 300 sockets for the original pattern versus
one for reuse. Raw output is in `results.json`. Elapsed values include deliberate
pauses and concurrent builds; they support no speed claim.

The application fix retains the host with active streams and reuses it for
polling and microphone retries. Missing default metadata no longer triggers
reopening; a confirmed changed default or actual stream failure still does.
Device-free regression tests cover the decision and readiness across 602
synthetic observations, not ten elapsed minutes of audio.

Limitations: the probe still observes one retained connection after the reused
client's final drop. This patch removes polling-driven accumulation; it does not
repair the upstream idle-drop behavior. No actual PulseAudio client limit,
Linux/PipeWire hardware, Discord guild call, process RSS or physical latency was
measured. The reporter's failure needs a live retest.
