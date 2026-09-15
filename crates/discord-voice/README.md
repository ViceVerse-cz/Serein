# Discord voice adapter

Native media for an explicitly joined one-to-one or group Discord DM or server voice channel (up to 64 participants including yourself). The desktop owns call intent, main Gateway/REST signaling and the audio-device lifetime. This crate owns the separate voice WebSocket, UDP transport, codec and ephemeral DAVE group. It never records audio, persists voice keys or contacts a project relay.

Protocol evidence checked September 10, 2026:

- [Discord voice connections](https://docs.discord.com/developers/topics/voice-connections): voice WebSocket version 8, heartbeat acknowledgements, Identify/Resume, UDP discovery, RTP, Opus, the required XChaCha20-Poly1305 RTP-size transport mode, and DAVE enforcement.
- [Discord DAVE whitepaper](https://daveprotocol.com/): version 1 group setup, external proposals, MLSMessage framing, welcome/commit transitions, invalid-group recovery, ephemeral identity keys and frame encryption.
- [Davey 0.1.4 source](https://github.com/Snazzah/davey): OpenMLS-based interoperable primitives. This is an unofficial implementation, not Discord's libdave or an assertion of an independent security audit. Its raw key package is wrapped in the MLSMessage envelope required by opcode 26. Additional guards reject unexpected channel group IDs, users outside the authenticated voice participant list, duplicate membership, non-external proposals and proposal types other than Add/Remove. One-to-one DM membership remains restricted to its original two users; group DMs use the authenticated voice-server roster. All `tracing` output is compiled out because upstream trace statements can contain cryptographic secrets.
- [Opus2](https://github.com/cijiugechu/opus2), bundled libopus and [CPAL](https://github.com/RustAudio/cpal): native codec and device bindings. The audio engine accepts 48 kHz mono frames; outgoing Opus uses duplicated stereo at 64 kbit/s and 20 ms, while received audio is decoded to mono.

Only DAVE version 1 is negotiated. No media is sent before an authenticated group and its transition have completed. Group DM and guild membership follows bounded voice WebSocket opcodes 11/13; joins and departures pause media until the next validated MLS transition. An empty server room stays joined with audio devices disabled while its epoch-zero group waits for a peer. Encryption downgrades, unannounced group members and unsupported control states fail closed. The displayed privacy code compares the current group epoch; ephemeral identities are regenerated on rejoin and are not remembered or vouched for across sessions.

Budgets: 64 KiB voice WebSocket frames/messages; 4 KiB UDP receive limit; 1,275-byte encoded Opus frames; eight PCM frames per device queue; one decoder per remote participant (at most 63); a reorder buffer per speaker with eight encoded packets (10,200 encoded bytes), a 40 ms startup shortened when the queue fills, and up to three consecutive packet-loss concealment packets (previous packet duration, at most 120 ms each, streamed in 20 ms ticks); a fixed 5,760-sample mono buffer per speaker (23,040 bytes, at most 120 ms) for packet durations longer than one playback tick; three invalid-group recoveries; 1,024 group transitions/reinitializations before requiring rejoin; 256 control messages per second before disconnect. Initial connection is limited to 15 seconds, UDP discovery to eight seconds and group negotiation to 90 seconds (30 seconds for a subsequent transition). Authenticated empty-room waiting has no negotiation deadline; a participant arrival restarts it. Normal sole-member resets use the transition budget, not the invalid-group recovery budget. Two voice WebSocket resumes are allowed per call, preserving live UDP/MLS state and the acknowledged cursor. Rejected/failed resumption requires explicit rejoin. Voice socket destinations must be Discord media hosts with normal certificate validation; UDP destinations must be public addresses supplied by the authenticated voice server.

`cargo test -p discord-voice --lib` uses synthetic keys and local sockets only. It exercises real MLS group creation, DAVE encryption/decryption and tamper/replay rejection; RTP authentication/truncation/nonce exhaustion; Opus encode/decode; bounded reorder/loss handling; and the actual voice event loop through local WebSocket + UDP discovery and two-way encrypted audio, followed by socket resumption. The three-party tests cover join/removal, post-removal decryption rejection, independent SSRC decoder state, simultaneous mixing, empty-room waiting, guild Identify/Resume IDs, and media resumption. The audio module separately tests its device-free resampling and capture gates. `cargo test --locked -p discord-voice --release synthetic_mix_workload -- --ignored --nocapture` measures device-free 1/8/63-speaker decoding/mixing (one warmup and five 1,000-tick runs); it does not measure live audio, callback latency or process RSS. A test-only path module checks the exact vendored HPKE SHAKE adapter against independent 32/64-byte output vectors.

These tests do not establish Discord compatibility or microphone/speaker quality. Actual two-way audio with an official Discord client remains an owner-operated live gate. No hardware audio access is performed by default tests. Receive streams mix into one 20 ms playback frame with hard clipping to the valid sample range; this is not an automatic gain controller. This implementation has fixed jitter buffering, automatic AEC3 echo cancellation, system-default fallback for selected devices absent when opening audio, and no globally captured push-to-talk. Use headphones for live validation. Windows/Linux audio and macOS microphone permission remain unverified until explicitly exercised on those systems.

The HPKE dependency has a small [source security backport](../../vendor/hpke-rs/SEREIN-PATCH.md) replacing its affected SHAKE dependency with RustCrypto sha3. All modified MPL-2.0 component source ships in voice packages. Current dependency findings and remediation are recorded in [the audit](../../docs/dependency-audit.md).

`Audio::set_gain(input_percent, output_percent)` adjusts session-only software levels without
opening/restarting devices. Values are clamped to 0..=200%, with 100% defaults. Two integer
atomics survive device replacement. Capture gain is applied on the worker after AEC;
playback gain is read by the callback and applied after resampling and mixing. Finite samples
are clipped to [-1, 1]; invalid PCM becomes silence. Callbacks retain preallocated lock-free rings; codec/protocol behavior is unchanged. Already-captured resampler endpoints and queued frames keep
their existing latency; gain is not a privacy substitute for mute/deafen. Existing gates reset
the callback buffers. Synthetic helper tests exercise the same processing used by CPAL, without
constructing a host/device/stream. Physical audio quality and real-time timing remain unverified.


Device configurations and encrypted-readiness pauses invalidate a monotonic audio revision.
Only streams acknowledged for the current revision enable callbacks or the connected UI state;
late readiness/errors from replaced streams cannot acknowledge a newer configuration.
Listen-only guild calls with denied SPEAK open only an output stream. Permission changes update
input availability independently of mute and focused push-to-talk, which never reopen streams.
Incoming 2.5/5/10 ms Opus packets fill one 20 ms playback tick with at most eight decodes per
speaker; longer packets retain their remaining PCM for subsequent ticks. No queue budget grew.
Voice close 4015 can use the existing resume budget; terminal closes still stop the call.
DAVE proposals before local group creation are bounded and ignored, following the
[initial-group procedure](https://daveprotocol.com/#initial-group-creation).


### Key-package interoperability correction

Opcode 26 now carries the raw TLS KeyPackage after the opcode byte, matching
[libdave serialization](https://github.com/discord/libdave/blob/5cb8952a8e6f08071d8c24edae2496199d7a9197/cpp/src/mls/session.cpp#L625-L635)
and the [reference client send path](https://github.com/dolfies/discord.py-self/blob/2ba64a9a997e151a9c259984e0a179b1fdf4aff4/discord/voice_state.py#L329-L340).
The written [whitepaper](https://daveprotocol.com/#dave_mls_key_package-26) instead describes an
MLSMessage wrapper. We follow those implementation paths here; the previous extra four bytes
also appeared in our test server's assumed format. The fixture now independently deserializes
and cryptographically validates a raw KeyPackage. This resolves a reference-format mismatch,
but does not prove that the owner's live no-audio report is resolved.

AEC uses Sonora 0.2.0 with its default adaptive delay estimator and high-pass filter,
in two 10 ms blocks per 20 ms mono frame. AGC remains off; optional RNNoise suppression follows AEC.
The rendered-reference ring holds eight frames (30,720 PCM bytes / 160 ms), in
addition to existing input/output rings. A single worker owns the processor;
its fixed mono/rate configuration bounds internal filter/render history. Full
callback rings discard new frames and signal a worker reset/drain; no growing
queue or AEC processing runs in CPAL callbacks. Mute/deafen transitions discard processor
history without reopening devices. New device/security revisions recreate it.
The offline debug example uses the same AEC wrapper as the worker. Hardware
delay, long-term clock drift and echo quality remain unverified.

macOS microphone authorization uses a small isolated Objective-C boundary in
`src/audio/permission_macos.rs`. Only that module permits unsafe code; the rest
of this crate denies it. The two AVFoundation class calls use the framework audio
media constant and an owned completion block with a one-item result channel.
Authorization runs on the device worker, before input creation; no callback or
render-thread blocking and no camera/system-audio permissions are requested.
The completion handler cannot open devices; the worker rechecks the active
revision after permission is granted.

Outgoing capture is paced FIFO with one 20 ms lookahead frame. Callback batches
are preserved during ordinary ticks; the previous drain-to-latest policy could
drop valid speech. The channel stays capped at eight frames, plus one lookahead
frame (34,560 PCM bytes combined). Mute/security gates and gaps of at least 80 ms
between transport ticks flush it. `cargo run --locked -p discord-voice --example echo`
checks batched capture continuity and gate/stall flushing alongside synthetic AEC.

Noise suppression is an independent, session-local toggle (default off), using
nnnoiseless 0.5.2 with its bundled model and no default crate features. One
worker-owned denoiser consumes the existing 10 ms AEC output blocks before gain
and Opus. It does not gate packets or reset AEC when toggled. The offline echo
example covers synthetic noise reduction, voiced-signal retention and bypass;
actual keyboard/breath/wind rejection still requires owner-operated listening.

## Optional outgoing screen video

`screen::Worker` owns native capture and encoding outside rendering. Linux uses the
ScreenCast portal/PipeWire/GStreamer path with hardware H.264 attempts and OpenH264
fallback. `run_stream` owns a separate Discord RTC connection, shares the parent
call's ephemeral `Identity`, and enables outgoing media only after DAVE is ready.
`video` handles bounded Annex-B/FU-A RTP packetization after DAVE frame encryption.

Optional system audio uses macOS ScreenCaptureKit, Windows process loopback excluding
Serein's process tree (build 20348+), or Linux PulseAudio/PipeWire per-application monitors
excluding Serein and unknown identities. Windows/Linux still include other applications
when sharing one window. Linux has a separate bounded audio worker so video encoding
cannot delay its sampling. Audio reaches the existing
stereo Opus sender through four bounded chunks (up to 38,400 PCM bytes each), plus
100 ms / 38,400 bytes pending at the sender. Buffers are tagged before queueing with
the capture generation so a rekey rejects late old audio. Native and wire behavior,
platform limits and the unverified live gate are in [the compatibility addendum](../../docs/discord-compatibility.md#outgoing-screen-sharing--september-11-2026).
