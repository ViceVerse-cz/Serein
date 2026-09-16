# DM and server voice

The standard build implements native audio calls in existing one-to-one and group Discord DMs and guild voice channels. It uses the owner's existing account, Discord signaling/voice servers, Opus and DAVE version 1. There is no bot, project relay, separate account, recording service or webview call UI. **Live Discord interoperability and physical microphone/speaker behavior have not been tested; milestone 4 has not passed.**

```sh
cargo run --locked
cargo run --locked -- --demo  # offline UI; calling/device access disabled
cargo xtask package                     # standard artifact under dist
```

Voice is included in every build without a feature flag. Source builds require CMake for bundled static libopus; Linux needs ALSA development headers. See [platform requirements](platform-support.md) and the [voice adapter README](../crates/discord-voice/README.md) for dependencies, exact resource limits and protocol tests.

## Implemented behavior and limits

Guild voice channels have a chat icon with a **Show chat / Hide chat** tooltip in the channel header.
Chat uses the existing message timeline, composer, drafts and permission checks without
requiring a voice connection. Wide windows place chat beside the stage; narrow windows
show chat in the main area until Hide chat restores the stage. The existing bounded
history/cache and message transports are shared. Normal-account interoperability remains
unofficial and live-unverified. The offline debug check is
`cargo run --locked -p ui --example voice_chat`.

The Audio menu provides session-only Microphone gain and Speaker volume controls from 0% to
200%, initially 100%. Reset levels restores both to 100%. Changes apply to the active call
without reopening devices and carry across calls/device changes in the same session; logout
or preview reset clears them. Zero silences that signal; values above 100% boost and may clip.
These are software levels, not system mixer settings or automatic gain control. Existing mute,
deafen, push-to-talk, permission and encrypted-readiness gates continue taking precedence.
Opening settings or changing a level never starts a call or opens a microphone.

Right-click another participant's voice-row name, avatar, or stage card for
**User volume**, from 0% to 200%, and **Reset volume** (100%). Keyboard users can focus
an avatar/name and press Shift+F10. This changes only that person's voice playback before
mixing; the global speaker level and deafen still apply. Speaking indicators remain based on
the received signal. Screen-share audio has its own mix and is unaffected. Overrides carry
across calls and device changes in the current session and clear on logout/preview reset.
At most 64 custom levels are retained; a full table replaces its first retained entry.
Nothing is sent to Discord or saved to disk. Boosting may clip; physical listening and
native slider interaction remain unverified.

Device-free tests cover gain, clipping, invalid PCM, independent live changes and gates. Actual
gain perception, microphone/speaker hardware, native slider interaction and callback latency
remain unverified. Owner-controlled live checks should include 0/100/200% on each control,
Reset levels, mute/deafen/PTT precedence and changing devices while custom levels are selected.

Start calls the selected existing DM; incoming calls require Answer or Decline. One active call is retained while navigating text conversations. Start rings once after Discord voice transport allocation is confirmed; Answer never rings. Required DAVE group readiness and native device readiness precede the connected-audio state. An allocation with no endpoint waits within the deadline; incompatible states fail visibly. Hangup closes local audio immediately and sends departure; another call waits for the service's departure acknowledgment. No uncertain ring write or failed main Gateway session automatically starts another call.

Opening a one-to-one or group DM also requests its existing call state. An ongoing call shows a
**Call in progress** banner and **Join call**, even after ringing stops or this device leaves.
Join uses the existing connection flow without ringing again; browsing never joins or opens
audio devices. Incoming ringing retains Answer/Decline. Join is disabled while offline, in a
voice-unavailable session, or while another local call still exists. Ended/unavailable calls disappear.
This uses the existing unofficial Gateway opcode 13 and CALL_CREATE/UPDATE/DELETE contract,
checked against [discord.py-self's Gateway implementation](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py)
and [call dispatch handling](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py)
on September 11, 2026. Local WebSocket and reducer/UI tests establish the implementation;
discovery of a real existing Discord call remains unverified.

`cargo run --locked -- --demo --demo-existing-call` shows a synthetic
ongoing DM call with no local media session. The preview Join button is deliberately disabled.
For the owner-controlled live gate, leave the peer connected in a private DM call, open that
DM in Serein, wait for the banner, then explicitly Join. Verify no new ring, actual two-way
audio, leaving/rejoining while the peer stays, and disappearance after the peer ends the call.

Mute/deafen, session-local input/output selection and focused V push-to-talk are implemented. Push-to-talk releases when focus is lost and is disabled while text entry has focus. It is not a global hotkey. Devices are initialized only following an explicit call and encrypted readiness; no microphone test runs at startup. Acoustic echo cancellation is enabled automatically; see below for its limits. A microphone that fails to open or start, reports a callback error, or delivers no audio callbacks for three seconds is disabled with a visible warning. The call and speaker playback remain connected, and selecting another input retries microphone setup. Ordinary silence does not trigger the warning. Selected speaker failures can fall back to the default output; an unusable output can still fail the call.

One-to-one DM calls accept only their expected peer. Group DM and server calls support up to 64 total participants, with independent bounded decoder/jitter state and mixed mono playback. Only DAVE version 1 is accepted; encryption downgrades and group identities outside the authenticated participant roster fail closed. Stage channels and recording are unsupported. Outgoing screen sharing and macOS camera support is described below. Voice WebSocket resumption has a finite retry budget; failed resumption or main Gateway disconnect requires an explicit new call. Voice credentials, ephemeral DAVE identities and audio stay in bounded session memory. The displayed privacy code applies to the current group epoch; identities are not remembered across calls. Comparing codes does not establish long-term identity verification or text-message encryption.

## Group DM calls

Existing group conversations expose the same Start/Answer/Decline/Join controls, call stage,
mute/deafen, audio device and gain controls, focused push-to-talk, noise suppression,
privacy code, camera, screen sharing and stream viewing as one-to-one calls. Opening the
conversation only requests call presence. Starting a new call rings the group once; joining
or answering an existing call does not ring again. Group avatars identify incoming calls.
The shared grid fits the participant tiles in the stage while text chat remains available.

Group membership uses the authenticated voice-server roster and the existing DAVE transition
validation, rather than pinning the first recipient as a one-to-one peer. Call metadata admits
only this account and known group recipients, rejects duplicate participants, and is bounded
to 64 participants per call. Recipient removal drops stale participant/watch UI state; removal
of this account closes local media and revokes subsequent call actions.

The offline debug command is `cargo run --locked -p serein --example group_call`.
Group signaling remains unofficial and live-unverified. For owner-controlled verification,
repeat the one-to-one gate below in a private group whose participating clients the owner
controls, including simultaneous speech, additions/removals, ringing/decline/join, camera,
sharing/viewing, last-peer departure/rejoin, and removal of this account. This local `!fast`
implementation pass does not establish production readiness or physical media behavior.

## Protocol classification

| Area | Evidence / classification | Verification here |
|---|---|---|
| DM entry, incoming call events and ringing | [discord.py-self Gateway](https://github.com/dolfies/discord.py-self/blob/master/discord/gateway.py), [dispatch](https://github.com/dolfies/discord.py-self/blob/master/discord/state.py), [HTTP](https://github.com/dolfies/discord.py-self/blob/master/discord/http.py): unofficial normal-user behavior | Real local WebSocket op13/op4 join/leave and local HTTP ring/decline tests; no Discord call |
| Voice WebSocket, UDP discovery, RTP and codec negotiation | [Discord voice documentation](https://docs.discord.com/developers/topics/voice-connections): documented transport, not an approval of normal-user clients | Synthetic loopback voice event loop, authenticated RTP and Opus tests |
| Required end-to-end encryption | [Discord DAVE protocol](https://daveprotocol.com/): documented; [Davey](https://github.com/Snazzah/davey): unofficial implementation, not an independent security-audit claim | Synthetic two-party MLS/DAVE exchange, tamper/replay rejection and encrypted audio across local sockets |
| Microphone, playback, resampling and devices | CPAL/native platform APIs | Device-free capture/resampling tests only; physical audio and permission dialogs unverified |

The [compatibility matrix](discord-compatibility.md) distinguishes this from restricted OAuth/RPC capabilities. No OAuth voice grant or bot connection substitutes for the user's session.

## Owner-controlled live gate

Run only when the owner explicitly elects to test and controls both sides of a private one-to-one DM. Ordinary tests/CI never access Discord or audio devices. Do not put credentials in chat, command-line arguments, screenshots, fixtures or reports.

1. Build `cargo run --locked`. Complete the [normal-user text gate](authentication.md) with the owner's Serein session and an official Discord client. Prefer Serein's own official-login webview; do not extract another application's credential.
2. With headphones on both sides, open the existing private DM and deliberately select Start. Verify that the official client rings, Answer there, and wait for encrypted audio readiness. Compare the displayed current-epoch privacy codes where available. If login, transport, DAVE or permissions fail, record the redacted failure and stop that attempt; do not bypass it.
3. Speak short test phrases in both directions. Confirm intelligibility, latency and absence of unexpected echo. A connected label, participant list or socket handshake alone is not success. Check mute, deafen, focused V press/release, focus loss and navigation to another text conversation.
4. Hang up and verify both clients leave, microphone access ends and another call can start after departure acknowledgment. Reverse direction: call from the official client, test Decline, then a separate Answer. Confirm there is no automatic answer or retry.
5. Deliberately test selected device changes/loss, network interruption and bounded resume, logout during a call, normal exit and repeated join/leave. Check audio-device/task cleanup and retained memory. An abrupt process exit still needs real service departure and platform-cleanup verification.
6. Record date, OS/hardware/build features, which cases passed, redacted failures and measured resource use. Do not retain voices or private conversation contents. Record only observed behavior in the task PR description; Windows, macOS and Linux require separate physical tests.

Until actual two-way official-client audio and the relevant encryption/teardown cases pass, the release voice gate remains blocked. Offline encrypted transport tests are useful implementation evidence, not that gate.


## Server channel workflow and live gate

Select an existing server voice channel to inspect its roster, then explicitly Join. Browsing alone never opens media devices. Participant rows show names/avatars and separate mute/deafen states; the connected channel shows elapsed local connection time. Mute/deafen, audio settings and Leave remain available while reading other channels. Server-enforced mute/deafen cannot be overridden locally. To switch rooms, leave the current room and join the next after departure is acknowledged. A rejected/full/inaccessible room fails visibly after the bounded allocation deadline.

An authenticated empty room displays “Connected · waiting for others”; audio devices stay closed until another participant joins and DAVE is secured. The client does not transmit unencrypted microphone audio to make an empty room appear connected. A server move, changed voice endpoint/session or main Gateway failure requires an explicit rejoin. The roster is session-only, bounded to 4,096 entries and 1 MiB, and is cleared on fresh login/resync and relevant access invalidation; during a resumable disconnect it is labeled last-known until missed events replay. Missing user details use a fallback identity rather than fetching a whole guild directory.

`cargo run --locked -- --demo --demo-voice` shows a separately labeled synthetic roster/call scene, including long names and mute/deafen states. It cannot connect, ring, or access devices. The ordinary `--demo` fixture remains the before/after comparison scenario.

For live verification, the owner must explicitly enable `voice` and control a private guild voice channel and the participating official clients. In addition to the DM gate above: join empty then add two official-client participants; verify actual intelligible audio in every direction and simultaneous speech; exercise encrypted joins/leaves and the last peer leaving/rejoining; check self mute/deafen, server mute/deafen, denied Connect/Speak, full room, deliberate switching, a server move/disconnect and voice region migration; verify devices/keys/tasks are released on Leave/logout/exit. Never record participants or publish private account/channel data. None of these live outcomes is established by the synthetic roster screenshot.

Permission-aware continuation: known VIEW_CHANNEL and CONNECT are required for guild joining;
SPEAK is required before initial or sustained microphone capture, and missing/denied USE_VAD
requires enabled, focused, held push-to-talk. A listen-only join stays muted. Lost CONNECT
ends the active call; merely regaining access never rejoins. The failed-call state remains
visible after Gateway disconnect. These gates are covered by offline permission and
device-free capture tests; owner-operated live permission changes/audio remain unverified.


## Connection and playback recovery

A voice-server crash (WebSocket close 4015) uses the existing two-attempt resume budget,
retaining the UDP connection, acknowledged signaling cursor and encrypted group. Terminal
closes, including 4014, still require an explicit new call. Bounded proposals arriving before
DAVE has a local group are ignored as required by its initial-group procedure; established
groups retain strict proposal validation and no early proposal enables audio.

Device readiness belongs to the current device configuration. A rapid encryption pause while
devices are opening invalidates old readiness even when both events reach one UI frame;
late readiness cannot mark the call connected. Once opened, devices stay open across a brief
rekey while callbacks are silenced and old PCM is discarded.

Received short Opus packets are combined into the normal 20 ms playback frame. The encoded
reorder queue remains bounded to eight packets per speaker; a full queue starts playout early
instead of overflowing while waiting for its usual two-tick startup delay.

These behaviors have synthetic regression coverage. Physical devices and Discord calls still
require the owner-operated live gate above.

Denied SPEAK now opens playback without selecting or initializing a microphone. Regaining
SPEAK prepares input under the current encryption and mute/PTT gates; mute/PTT alone does not
reopen devices. Lost VIEW_CHANNEL drops the stored roster and hides participant rows, including
when no call is active; late updates cannot repopulate an inaccessible channel.


## Diagnosing a call that never opens audio

Windows call playback uses the selected speaker's default shared-mode mix format,
with the existing resampler converting 48 kHz call audio when needed. This avoids
choosing a converted format solely by enumeration order. Speaker-open errors
distinguish busy, disconnected, unsupported-format and permission failures when
the audio backend identifies them. This fast local change still needs a retry on
an affected Windows device; it does not establish that every reported failure is fixed.

Call progress distinguishes requesting allocation, connecting to the voice server, checking the
UDP network path, securing audio, and opening audio devices. Negotiation errors identify the
missing server Hello/Ready, transport key, DAVE group or transition execution. These are bounded,
static status messages; no identifiers, tokens, audio or raw signaling are logged.

Audio-device opening has a 20-second deadline after encrypted readiness, including device changes.
If a system device API stalls, local audio is disabled and departure is requested. The existing
worker must retire before another call can open devices; a driver that never returns can require
restarting Serein. The watchdog cannot forcibly cancel an operating-system driver call.

The outgoing DAVE key-package encoding was corrected to match reference implementations; see
[the adapter's source comparison](../crates/discord-voice/README.md#key-package-interoperability-correction).
Actual two-way audio still requires the owner-operated test above.

## Investigating high CPU during a call

Failed calls show their first safe failure reason, with a **Copy failure reason**
button, in both the sidebar and call stage. The reason remains until the call is
dismissed, including after departure acknowledgments or a later gateway disconnect.
Audio/transport worker failures use a separate fixed slot so a full progress queue
cannot discard the cause. Copying includes only the failure text, not participants,
channel identifiers, credentials or media. The timing summaries below do not explain
a terminal failure; copy the failed-call reason as well when troubleshooting.
`cargo run --locked -p serein -- --demo --demo-voice-failed` previews a synthetic
failure and checks that subsequent cleanup/progress events retain its original reason.

Set `SEREIN_VOICE_DIAGNOSTICS=1` before launching Serein to get aggregate voice
timings on stderr every five seconds and a best-effort final summary on teardown.
For example, launch an already-built macOS app from a terminal:

```sh
SEREIN_VOICE_DIAGNOSTICS=1 SEREIN_FRAME_DIAGNOSTICS=1 /Applications/Serein.app/Contents/MacOS/serein 2> serein-voice.log
```

On Windows PowerShell, set `$env:SEREIN_VOICE_DIAGNOSTICS="1"` and
`$env:SEREIN_FRAME_DIAGNOSTICS="1"`, then launch `serein.exe 2> serein-voice.log`.
On Linux, use the same environment assignments as macOS with the installed executable.
Quit an already-running instance first. Join/leave the call yourself; diagnostics never
enable capture, join a call or send media. Quit normally to obtain the existing UI frame
summary. Remove the environment variables to disable diagnostics on the next launch.

Each voice stage reports `[calls, total_us, max_us]` over `window_ms`:
`echo_render` processes speaker reference; `echo_capture` includes AEC and optional
noise suppression; `noise` isolates the RNNoise suppression part of `echo_capture`;
`encode` includes Opus and outgoing encryption; `mix` includes remote Opus decoding; `receive` measures accepted packet decryption/queueing.
`noise_frames` identifies capture frames processed with suppression enabled.
Audio `wakes` counts worker iterations; Transport `wakes` counts 20 ms timer ticks.
`resets` counts AEC resets from mute transitions or callback overruns; `drops` counts
full capture/playback worker queues; `stalls` counts transport gaps of at least 80 ms.
Stage timings exclude device callbacks, socket waits, device opening and UI rendering.
These are elapsed times, including scheduler preemption, **not process CPU percentages**.
`debug=true` identifies a build with debug assertions. Development builds optimize the
Sonora echo-processing crates, RNNoise (`nnnoiseless` and its FFT chain) and libopus
while keeping application code unoptimized and debuggable; release builds remain the reference for overall performance. Rebuild and
restart to apply this change. Compare speaking, muted and noise-suppression-on/off windows to narrow
the cause; UI frame diagnostics help identify excessive rendering separately.

Logging is off by default. Fixed numeric reports go through an eight-slot queue to a
separate writer; media workers never wait for stderr. Output stops after 128 reports
or 64 KiB per process, shared by all calls, so restart for another capture. A full queue
drops summaries. No IDs, device names, endpoints, keys, audio or signaling payloads
are logged; upstream cryptographic tracing remains disabled. No files are created by
Serein. Shell redirection is owner-managed and may include unrelated framework logs.
The device-free check is `cargo run --locked -p discord-voice --example voice_diagnostics`.
Instrumentation alone does not establish the cause of a reported CPU spike or a speedup.

## Acoustic echo cancellation

The voice build automatically runs Sonora 0.2.0 (a Rust port of WebRTC AEC3) on
the audio worker, before microphone gain and Opus encoding. It uses mixed
speaker output after software volume and resampling, including silence on
underrun/deafen, as the echo reference. Devices and encryption changes recreate
the processor; mute transitions and dropped callback frames reset its history.
No settings, SDK account, model downloads or additional device access are needed.
Echo cancellation stays enabled independently of optional noise suppression.
Automatic gain control is off. Krisp SDK embedding requires a
[commercial license](https://sdk-docs.krisp.ai/docs/licensing-information).

The device-free debug command is `cargo run --locked -p discord-voice --example echo`.
It checks synthetic delayed/reflected echo reduction and preservation of a local
signal. It does not establish real-room quality or Discord interoperability.
AEC needs time to adapt after resets. Bluetooth latency, separate device clocks,
very loud/clipped speakers, simultaneous speech and non-48 kHz hardware need
owner-operated listening checks. The existing linear resampling fallback remains.

## macOS microphone permission

Before opening a microphone for an explicitly joined, secured call, Serein checks
AVFoundation authorization and requests access if undecided. Denied/restricted
access produces a visible error directing the owner to System Settings > Privacy
& Security > Microphone. The worker waits at most 20 seconds, checks call/device
cancellation while waiting, and never opens input after an obsolete grant.
Listen-only calls do not request microphone access. Windows/Linux are unchanged.

Voice-enabled macOS executables embed the same Info.plist used by the packaged
app, including NSMicrophoneUsageDescription, so cargo-run builds also supply the
required explanation. Terminal-launched builds can have permission attributed to
the launching terminal; an existing denial must be changed by the owner in macOS
settings. Restart a rebuilt app before retrying. Actual prompt/capture behavior
still requires the owner-operated check; a successful build is not that check.

## Microphone packet pacing

Outgoing audio preserves callback batches in order instead of retaining only
the newest frame each network tick. One 20 ms lookahead frame smooths normal
callback/worker scheduling variation. Mute, deafen, encryption pauses and a
transport stall of at least 80 ms discard queued capture rather than replaying
stale speech. The existing eight-frame capture channel plus lookahead retains
at most nine frames (34,560 PCM bytes). This adds 20 ms of intentional buffering.
The offline echo example also checks alternating two-frame/no-frame arrivals
and mute/stall flushing; physical cutout resolution still needs a listening check.

Speaking indicators use post-processing outgoing microphone audio and each remote
participant's decoded playout audio. A display-only −45 dBFS level threshold with
200 ms release lights the avatar ring and sound glyph in DM calls, guild tiles,
and the channel roster. This detects sound, not speech, and never gates packets.
Activity snapshots contain at most 64 IDs (512 bytes), update at most ten times per
second, and replace the previous value rather than queueing UI events. Mute,
deafen, permission and call lifecycle gates hide ineligible activity.

## Optional noise suppression

Voice settings → Noise suppression enables the bundled nnnoiseless 0.5.2 RNNoise
model, off by default and session-local like the other audio settings. Processing
runs locally after AEC and before input gain/Opus, in 480-sample mono 48 kHz blocks.
The signed-i16 float scale is converted at this boundary. Its fixed model/history
and 10 ms overlap add no growing queue, file recording, network service or download.
Only the microphone is denoised; playback remains unchanged. Toggling replaces or
drops the worker-owned denoiser without reopening devices or resetting AEC.
Mute/device/security resets discard denoiser history with AEC history.

This reduces background noise rather than guaranteeing voice-only output. Typing,
breathing, strong wind and clipping require owner listening checks; another human
voice may remain audible. There is no extra hard speech gate to cut quiet syllables.
The offline echo example checks synthetic hiss/click reduction, retention of a
synthetic voiced vowel, finite output and exact AEC-only behavior after disabling
suppression. These signals do not establish Krisp-equivalent real-world quality.

## Screen sharing

In a connected call, select **Share your screen**, choose a display/window, 720p or 1080p, 15/30/60 fps, cursor visibility and optional **Share system audio**, then select **Share screen**. The screen button changes to **Stop sharing** while starting/sharing; it also remains available in the compact call controls. Call microphone controls remain independent. All presets are selectable without Nitro, but Discord acceptance and sustained frame rate are not guaranteed.

Capture uses macOS 14+ ScreenCaptureKit (screen-recording permission in System Settings) or Windows Graphics Capture. Source discovery alone does not start streaming. Closing or minimizing a selected source may pause frames or end capture, according to the native API. The initial Windows adapter accepts source dimensions up to 3840×2160. Changes to screen-server metadata, lost video permission, leaving the call and logout stop sharing. The sender never starts itself after reconnection.

Linux uses the desktop ScreenCast portal and PipeWire. Share Screen opens the system
screen/window picker after the quality dialog; source discovery never opens that picker.
The default is 720p30. The worker tries VA-API, NVENC with GPU scaling, NVENC with CPU
scaling, then the existing OpenH264 software encoder. The call stage identifies the
active encoder and software fallback. GPU buffers stay native where driver/plugin
negotiation permits; zero-copy is not guaranteed, especially across GPUs. Local preview
is capped at 640×360/10 fps and suspended when minimized or viewing another channel.
ScreenCast-capable portal backends are required on both Wayland and X11; there is no
separate X11 capture fallback. AV1/H.265 sending is not included.

System audio defaults off on Linux and Windows. It shares other applications' playback,
even when sharing one window, and excludes Serein's own audio, including call playback
and watched streams. Exclusion happens at capture on the sender: a viewer cannot remove
their voice once another sender has mixed it into stream audio. Other apps' notifications
and audio remain included. macOS retains ScreenCaptureKit's current-process exclusion.

Linux captures individual playback streams through PulseAudio's per-stream monitor API,
also implemented by PipeWire's PulseAudio server. It excludes Serein and streams whose
application identity cannot be established; it never falls back to a whole-output monitor
or microphone. The ScreenCast portal grants video only; audio uses the existing desktop
audio access, including Flatpak's PulseAudio socket permission. A separate bounded worker
handles discovery, capture and mixing outside video encoding and rendering. No applications
are moved between outputs and no virtual device is installed.

Windows uses native process loopback with `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`,
excluding Serein and its child processes across outputs. This requires Windows build 20348+
(Windows 11 or Windows Server 2022; ordinary Windows 10 22H2 is older). Unsupported systems
or failed isolation report an audio error; turn audio off to share video alone. There is no
whole-output fallback. Neither adapter records to disk.

Native contracts: [PulseAudio per-stream monitoring](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/Developer/Clients/WritingVolumeControlUIs/)
and [Microsoft process-loopback capture](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/).

Both feed the existing stream RTC connection with 48 kHz stereo Opus at 128 kbps and
DAVE/transport encryption, independently of microphone mute. Capture output is gated
by secure readiness. The transport keeps at most 100 ms pending, sends one 20 ms frame
per tick, and clears queued/pending PCM on encryption transitions or a 100 ms stall.
Capture buffers carry their encryption generation; late buffers from an earlier
generation are discarded even if a pause and restart happen between worker iterations.
Stop sharing and call teardown release audio together with the screen capture.

The offline debug command is `cargo run --offline --locked -p discord-voice --example linux_screen`.
It compiles the actual portal/pipeline/worker modules on Linux or macOS with GStreamer,
checks pre-cancellation without D-Bus, and exercises synthetic preview, the secure-readiness
gate, stereo audio, bounded slow-consumer behavior, oversized-buffer rejection and software
H.264. It never captures a desktop or opens an audio device. Native Linux portal interaction,
VA-API/NVENC, package installation, performance and Discord viewing remain unverified.
Windows process exclusion, Linux application selection and actual remote sound still
require owner-controlled tests; synthetic samples do not establish those outcomes.
Use two clients with headphones, enable audio on the sender, play another app and speak
from the viewer: the app should be audible without the viewer's voice returning in the
stream. Repeat while starting/stopping apps, changing outputs, rekeying and stopping sharing.

The native demo (`cargo run --locked -p serein -- --demo --demo-voice`) exposes a synthetic picker without OS source discovery or capture. Live screen sharing requires the same owner-controlled login gate as voice testing. See [compatibility and limits](discord-compatibility.md#outgoing-screen-sharing--september-11-2026).

## Camera in calls (macOS, Windows and Linux)

The voice build can send a native camera after an explicit camera-on click in a
connected DM or guild call. The camera button remains available in narrow call controls.
A local preview replaces your avatar in both DM and guild call tiles, including when
you are alone or the participant roster has not arrived. Camera and screen-share
controls are available in the connected, waiting-for-others state.
Permission/device errors appear in the call stage.
Guild camera use requires STREAM permission. Camera-off, permission loss, call failure,
leave and logout stop capture. Peer join/leave rekeys preserve an enabled local camera;
outgoing media remains disabled until DAVE is ready. A transport reconnection stops
capture and requires another click. Demo mode never requests camera or microphone access.

An explicitly started screen share also shows a separate local tile while alone.
The worker retains at most one 640×360 RGBA preview (921,600 bytes), updated at
most ten times per second, plus the bounded UI texture and upload copy. Preview
resizing and color conversion run on the capture worker. Native raw-frame limits
still apply; encoding and network transmission wait for secure media readiness.
Stop, source failure, permission loss, leaving and logout release the preview.
These paths have synthetic coverage; native camera/screen capture and live Discord
viewing still require owner-operated validation.

AVFoundation on macOS, Media Foundation on Windows and V4L2 on Linux capture
640×480 frames, capped at 15 encoded frames/second, encoded on a worker with a
600 kbit/s target (not a measured bandwidth guarantee). The worker prefers the platform
hardware H.264 encoder, the same VideoToolbox and Media Foundation encoders screen sharing
uses, and VA-API or NVENC through a private GStreamer pipeline on Linux. OpenH264 remains
the fallback when no hardware encoder is available and when one fails mid-capture, which
switches the remaining capture to software rather than ending it. Every path requests the
Baseline profile and codes each picture as an IDR, so the wire format is unchanged; a
hardware encoder whose output is not independently decodable is rejected in favor of the
software one. macOS retains one pending
BGRA frame (1,228,800 bytes). Windows validates each native buffer against a 3,194,880-byte
ceiling (including row padding), requests one source buffer and queues at most one
921,600-byte RGB frame. Linux requests two mapped buffers, accepts at most four of
4 MiB each, and decodes YUYV or MJPEG on the worker with a 4 MiB JPEG allocation limit.
One RGB preview, one encoded frame (128 KiB), and up to 256 RTP
packets from one bounded frame are retained. Frames are independently decodable to tolerate
drops. DAVE H264 frame encryption precedes RTP fragmentation and the existing authenticated
UDP transport. No camera recording or cache is created; the codec is included on supported platforms.

Video SSRC assignment, H264 selection and opcode 12 announcements follow the
[public interoperability implementation](https://github.com/dank074/Discord-video-stream/blob/master/src/client/voice/BaseMediaConnection.ts)
(checked September 11, 2026); these normal-user video extensions remain unofficial and
live-unverified. This initial sender has no adaptive
bitrate or RTP retransmission. Physical
permission/device behavior, delivery to the official client and network-loss performance
require the owner-controlled live gate; an offline launch does not establish those results.

Windows exposes a camera picker in Voice & Audio settings and beside both call
camera controls. Discovery runs on a worker without activating a camera. Up to
32 device IDs (4 KiB each) and names (256 bytes each) are retained. The selected
ID is session-local. Refresh discovers added/removed devices; a missing selected
device is reported rather than silently opening another camera. Changing selection
stops active capture and requires another camera-on click.

Media Foundation devices use a native 640×480 mode convertible to RGB32.
DirectShow discovery/capture additionally covers virtual cameras such as OBS and
NVIDIA Broadcast, which may not appear in Media Foundation enumeration.
Its native input is limited to 1920×1080, converted to RGB24 and fitted into
640×480 with aspect-preserving nearest-neighbor scaling and black bars. It retains
one callback frame (at most 8,294,400 bytes) and validates the negotiated input
allocator against eight buffers of at most that size each. Vendor-driver and
upstream decoder allocations remain separate from those application limits.
Default selection falls back to DirectShow when Media Foundation lists no devices.
Allow desktop camera access in Windows
Settings > Privacy & security > Camera; Windows N may require the Media Feature
Pack. Linux tries `/dev/video0` through `/dev/video63` and uses the first accessible
progressive, single-plane 640×480 YUYV/MJPEG streaming camera. The session or sandbox
must already permit access to its device node; this implementation does not request
camera access through a desktop portal or change device permissions. These fixed-mode
adapters can reject cameras that only offer other resolutions or formats.

Frame waits time out after five seconds without a usable frame; stop is checked at
most every 100 ms while waiting. Native driver initialization/teardown has no hard
deadline. The process-wide worker slot stays occupied until cleanup finishes so rapid
toggles cannot accumulate blocked capture workers. Native driver/codec allocations
are separate from application queue limits and are not a measured whole-process bound.

Windows uses the documented [asynchronous source reader](https://learn.microsoft.com/en-us/windows/win32/medfound/using-the-source-reader-in-asynchronous-mode)
and [bounded 2D buffer locking](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imf2dbuffer2-lock2dsize).
The virtual-camera fallback uses the documented DirectShow
[device enumerator](https://learn.microsoft.com/en-us/windows/win32/directshow/selecting-a-capture-device)
and [sample grabber](https://learn.microsoft.com/en-us/windows/win32/directshow/using-the-sample-grabber).
Linux follows the kernel's [V4L2 capture interface](https://www.kernel.org/doc/html/latest/userspace-api/media/v4l/capture.c.html).
Both feed the existing camera transport; no Discord wire behavior changed in this extension.

### Screen-share audio diagnostics

The existing opt-in `SEREIN_VOICE_DIAGNOSTICS=1` reporter now also emits `StreamSend`
and `StreamReceive` summaries. `StreamSend` encode calls count captured 20 ms audio
frames encoded, encrypted and sent. `StreamReceive` receive calls count accepted
DAVE audio packets; mix calls count decoded frames offered to the parent call's output.
StreamReceive drops count audio/video DAVE decryption failures or a full playback/
decoder handoff queue. `video_send` counts complete encrypted H.264 frames sent;
`video_receive` counts decrypted frames accepted by the decoder queue, not displayed frames.
Each stream report also counts ticks with the transport key, DAVE ready, group ready,
pending transition, waiting, announced, capture ready and audio enabled flags set,
plus audio chunks observed in the capture queue before draining. These distinguish
closed security gates, missing capture data and active outbound video. Tick counts
reset every report; receiver capture-ready and queued-audio counts are always zero.
Windows also emits `ScreenAudio`: `capture_read` counts successful native buffer reads,
`capture_queue` counts chunks handed to the stream, `stalls` counts 50-ms event
timeouts, and `drops` counts packets rejected by the ready/epoch/queue gates. Initial
start and epoch restarts emit bounded checkpoints before/after the native calls;
`capture_restart` counts completed starts/restarts, while `resets` counts attempted
epoch restarts. This separates missing wakeups, empty native buffers and blocked
restart calls without logging audio. Dropped packets before stream readiness are expected.
On an encryption epoch change, Windows recreates the process-loopback client instead
of restarting it with Stop/Reset/Start. Each client's captured audio retains its original
epoch until disposal, so queued samples cannot cross the encryption transition.
Zero sender encode calls means no PCM reached the secure sender; sender activity with
zero receiver receive calls narrows the failure to forwarding, mapping or decryption.
Successful receives/mixes with no sound narrows it to silent source PCM or parent
playback/device gates. These counts do not prove audible sound and do not contain PCM,
participant IDs, credentials or signaling contents. The existing eight-report queue and
8,192-report / 8-MiB process-lifetime limits still apply. Enable it on both endpoints
only for the owner's deliberate test, then disable it after collecting the summaries.

### Remote video diagnostics

Every report line carries `at_ms`, milliseconds since the first reporter started, so
lines from different scopes can be ordered. `Transport` (call camera video) and
`StreamReceive` (watching a screen share) lines append a `video:` group whenever any
remote video counter is non-zero. Each counter names one place a picture can be lost
between the UDP socket and the display, so a frozen viewer is diagnosed from one line:

- `packets` / `rtx`: video RTP packets (payload 101) accepted by the transport cipher,
  and retransmission packets (payload 102), which Serein does not yet use. Many `rtx`
  packets mean the media server sees loss on the path.
- `open_failed`: packets of any payload rejected by the transport AEAD.
- `not_ready`: video packets received before the DAVE session was ready or from a
  user outside the group; expected briefly after joining or an epoch change.
- `unknown_ssrc`: packets on a video SSRC no sender announced.
- `incomplete` / `complete`: pictures the RFC 6184 depacketizer discarded (sequence gap
  or missing marker) versus intact encrypted access units.
- `decrypt_failed`: access units that failed DAVE decryption.
- `gated`: predicted pictures rejected because a keyframe is still owed after loss.
- `queue_full`: frames dropped because the decoder thread was behind.
- `keyframes` / `keyframes_without_params`: keyframes handed to the decoder, and how
  many lacked inline SPS/PPS. A rebuilt decoder cannot start from those.
- `pli_sent`: Picture Loss Indications sent (at most one per owed sender per 500 ms).
- `awaiting_ticks`: 20 ms ticks spent waiting for at least one sender's keyframe.
- `decoder_errors`: decoder failures reported by the decoder thread.
- `pictures`: decoded pictures delivered to the display sink.
- `picture_gap_ms`: the longest gap between delivered pictures in the window (a
  maximum, not a sum). Zero until the first picture is delivered.
- `stall_ticks`: ticks where a source was announced but no picture had arrived for a
  second. Each of those ticks asks every announced sender for a keyframe.

A `signal:` group appears on the same lines whenever any signaling counter is non-zero.
It records opcodes only, never their contents, so a handshake that never completes names
its own missing step: `text` and `binary` count all received events, `other` counts text
opcodes without a dedicated slot, and `ready` (2), `session` (4), `clients` (11),
`sender` (12), `prepare_transition` (21), `execute_transition` (22), `prepare_epoch` (24),
`external_sender` (binary 25), `proposals` (binary 27) and `commit` (binary 29/30) count
the negotiation. Outbound messages are `key_package_sent`, `transition_ready_sent` (23),
`subscribe_sent` (12) and `sink_wants_sent` (15). Only `execute_transition` makes DAVE
ready, so a stream that reports a transport key with `dave_ready=0` and no
`execute_transition` stalled in the MLS handshake rather than in media.

Two recovery paths depend on these counters. A viewer refreshes its video sink wants
every five seconds, and every second while stalled, because Discord stops forwarding
video when that subscription lapses. Separately, video that stops cleanly leaves nothing
marked lost, so no per-picture signal would ever request recovery; after a second without
a decoded picture every announced sender is asked for a keyframe until one arrives.
Both are visible as `sink_wants_sent`, `stall_ticks` and `pli_sent`.

When Discord itself ends a stream, its `STREAM_DELETE` carries a `reason`. Both the
sharer's status and the viewer's notice now show a message derived from it (for example
"Discord reported the stream as ended") instead of the same text a local stop shows, and
an unrecognised value still reads "Discord ended the stream". A `user_requested` deletion
stays silent because it is the local stop acknowledging. With the diagnostics variable set,
the bounded raw value is also printed as `[Serein voice Stream] discord_delete_reason=…`.

Linux application audio reports under `ScreenAudio` as well: `capture_restart` counts
completed application enumerations, `capture_read` monitor reads, `capture_queue` chunks
handed to the stream, `resets` epoch or roster resets, `stalls` 100 ms stalls, and
`app_inputs`, `app_captures`, `app_excluded`, `app_chunks` count the applications the
enumeration allowed, monitor captures started, applications excluded after a failed
capture, and mixed chunks sent. `app_inputs=0` across a share means no other application
exposed a PulseAudio stream with `application.process.id` and
`application.process.binary`, so there was nothing to capture; `app_excluded` rising with
`app_captures` means the server refuses the isolated monitor for that application.

Reading a freeze: `packets` rising with `pictures` at zero and `picture_gap_ms` growing
confirms the viewer is starved, not the display. High `incomplete` with `gated` and
`awaiting_ticks` near the full window and `pli_sent` rising but `keyframes` at zero
means the sender is not honoring keyframe requests. `keyframes` rising alongside
`keyframes_without_params` and `decoder_errors` means the decoder cannot restart from
the sender's keyframes. `packets` at zero with `awaiting_ticks` high means the media
server stopped forwarding. These are counts only; no video, identifiers or signaling
contents are recorded.

On Windows, use `Start-Process` to attach stderr to the GUI executable; direct shell
redirection can leave an empty file. After closing the previous test instance, run
the intended build on each endpoint with separate output files:

```powershell
$env:SEREIN_VOICE_DIAGNOSTICS="1"
Start-Process .\dist\serein.exe -RedirectStandardError "$PWD\stream-debug-retest.log" -Wait
```

Start sharing promptly after launch so the bounded diagnostic budget covers the test.
Keep these local logs out of commits.
