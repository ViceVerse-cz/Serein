# Diagnostics and issue reports

Diagnostics are optional and off by default. They are local: Serein does not upload reports,
collect telemetry, or submit issues automatically. Recording does not join a call, enable a
microphone or camera, capture a screen, or send a Discord message.

## Collecting a report

1. Open **Settings → Updates → Support & diagnostics**.
2. Enable **Record diagnostics**. All supported categories, including voice and performance
   summaries, are included. Recording starts now; earlier events cannot be recovered.
3. Reproduce the problem briefly, if you are comfortable doing so.
4. Choose **Copy diagnostic report** to paste the report into a GitHub issue, or **Save report…**
   to export the same report as a text file. Review it before posting publicly.
5. Turn recording off when finished. **Clear diagnostic history** removes collected history.

For crashes or a frozen interface, enable **Save diagnostic history to disk** before reproducing
the problem. This is a separate opt-in and is also off by default. Previously written files may
help when the application cannot export a report; the final records may not have reached disk.
Use **Open folder** to find retained files from earlier runs. Copy and Save export the current
memory history only; they do not import files from previous runs. A normal report still works
without disk storage. The bug and crash issue forms recommend a
report but allow submission without one. Explain what happened even if diagnostics were disabled,
the problem is unsafe to reproduce, or you prefer not to collect a report.

While recording is disabled there is no allocated diagnostic history or periodic diagnostic
task. After switching it off, a bounded cleanup worker may finish removing managed files before
it exits. There is no diagnostic writer in steady-state disabled operation. Settings and disabled-state checks still exist; this does not mean
that adding diagnostics has literally zero binary or static-memory cost. An on-demand report
can include broad system information without enabling recording.

## What reports contain

Reports include broad operating-system, architecture, application-build and packaging categories,
fixed operation/failure categories, elapsed durations, and bounded numeric summaries. Times are
relative to recording rather than wall-clock dates. They can reveal approximate technical activity
and timing; an anonymous technical report is not information-free.

Reports exclude credentials, authentication headers, cookies, session and crypto material,
account/user/server/channel/message identifiers (including hashes), names, message/draft text,
searches, filenames, paths, URLs, IP addresses, clipboard contents, input characters, device names,
process names, extension identities, exact hardware models, custom theme names, raw server errors,
panic text, backtraces, images, audio, video, and signaling payloads.

The collector is not a terminal recorder. Native libraries and framework output may contain
sensitive text and do not become part of a report. Do not attach browser logs, raw terminal output,
process memory dumps or screenshots of private conversations as a substitute. Diagnostics use
a closed, reviewed event/key/tag vocabulary and approved numeric measurements. Events with
unknown names or field keys are dropped. Unknown text field values are replaced with an
unclassified category rather than retained. A Rust string lifetime or
numeric type alone does not prove that a value is nonpersonal. New instrumentation must be reviewed at
its source, including values passed indirectly through helper functions.

## Storage and limits

The collector and file writer are bounded by both item count and bytes. The detailed limits and
file-safety behavior are specified in [the storage policy](storage-policy.md#opt-in-diagnostic-log).
Disk history is separate from the account database. Disabling disk history schedules removal
of managed log files; disabling recording also clears memory immediately. Cleanup is asynchronous
and can fail, for example if filesystem permissions changed. A reported cleanup failure may
require manual removal through **Open folder**. Manually exported copies belong to the owner and
are not removed by Serein.

Only one export is active at a time. Exporting does not authorize any network action. The user
chooses a destination using a native dialog; cancelling leaves no exported file.

## Coverage and exclusions

The implemented hooks below are broader than the original call-lifecycle log, but they do not
observe every `Result`, ignored callback, or detached worker. A report records failures observed
at the listed boundaries; absence of a record is not proof of success. No test or fixture report
establishes live Discord interoperability.

| Subsystem | Implemented shared coverage | Remaining gaps or exclusions |
| --- | --- | --- |
| Composer and input | Fixed blocked-submit reason, image preparation request and dispatcher handoff | Complete focus/IME state-transition tracing is not present; typed keys, text and clipboard contents are excluded |
| Message actions | Classified send/edit/pin failures and nested reaction/poll/read-ack action failures | No per-message target or content; not every successful reconciliation decision has a record |
| History, search, profiles and server/account actions | Central direct/nested result-failure classification, including invites, onboarding and interaction actions | Not every stale-result discard or permission decision is instrumented individually |
| REST, uploads and downloads | REST read/write flag, completion outcome and latency; desktop download failure | No routes, headers, proxy details, payloads or file paths; not every upload substage has its own event |
| Gateway and member synchronization | Existing fixed-label member/Gateway diagnostics, session ready/disconnect/resync/terminal failure | Unknown event names, raw payloads, list identities and every dispatch are excluded |
| Voice and audio processing | Call lifecycle, dynamically enabled stage summaries, transport/signaling/capture counters, jitter/drops and media failure | No device/participant identities, endpoints, audio or cryptographic material |
| Screen share, camera and call video | Video counters, stream-state counters, codec backend categories and existing capture/media failures | No source names/handles, application titles, pictures or media bytes |
| Images, avatars, GIFs and attachment playback | Image decode-limit rejection, image worker failure and download failure | Full low-level fetch/decode/playback failure coverage remains incomplete |
| Authentication and verification | Session failure category and credential-store operation failure | Full webview/verification lifecycle tracing remains incomplete; browser contents and credentials are excluded |
| Local store, settings and session cache | Cache operation failure and queue rejection, surfaced persistence errors | Not every internal database/migration or eviction decision emits a separate event; no rows, SQL parameters or paths |
| Extensions and themes | Host invocation error categories, including fuel/memory/trap/validation outcomes | No extension identity, package text, Wasm output or invocation values; theme validation is not exhaustively instrumented |
| Desktop integrations | Notification/compositor/window-effect failure; hotkey setup, registration and portal failures; clipboard paste failure; startup-setting failure; tray loss and initialization failure | Native copy completion and some save-dialog errors are not exposed by the underlying APIs; cancellation is not treated as failure; raw OS error text is excluded |
| Game activity, Spotify and local RPC | Game-activity task failure/exit and linked-playback polling failure | Full RPC tracing is not present; games, tracks, executable paths, process names and payloads are excluded |
| Updater and installer | Surfaced update operation failure and worker disconnection | Full helper-process/staging tracing is not present; installation paths and raw helper output are excluded |
| Runtime and shutdown | Fixed panic marker and selected worker exits, cleanup/export failure feedback | Detached tasks are not all supervised individually; no arbitrary thread names, panic text or backtraces |
| Rendering and performance | Frame callback/reflow summaries and broad graphics backend in environment report | GPU execution time, exact hardware and raw driver logs are unavailable |
| Models, parsing and cache primitives | Rejection/capacity results observed by owning callers | No automatic instrumentation of every low-level parser or cache return; no entity dumps |
| Test support and developer tools | Synthetic privacy, classification, bounds and collector-lifecycle scenarios | Tool/benchmark console output is not collected as runtime diagnostics |

When extending coverage, record a failure at its owning boundary rather than repeatedly at every
layer. Distinguish expected cancellation from failure. Add approved vocabulary and review all
numeric values as well as text; numeric Discord identifiers are still personal identifiers.

These gaps remain even with recording enabled:

- A native crash, out-of-memory kill, power loss or forced termination may bypass Rust handling.
- A fixed Rust panic marker can identify a failure without explaining the panic. Arbitrary panic
  messages and memory/backtrace dumps are excluded from the shared report for privacy. The normal
  Rust panic hook may independently print raw text to stderr; terminal output is not a safe report.
- A fully frozen interface cannot operate its copy button. Disk history is useful only if enabled
  beforehand and flushed; there is no always-running diagnostic watchdog.
- Failures before minimal preferences and diagnostic initialization may be unavailable.
- OS services, GPU/audio drivers and webview or installer subprocesses expose only API outcomes
  observed by Serein. Their private logs are not collected.
- A logic bug that returns success can be invisible. Targeted safe state summaries or additional
  instrumentation may be needed for that particular bug.
- Disabled, cleared, dropped or evicted history cannot be reconstructed. Bounded collection favors
  recent events and reports loss rather than expanding indefinitely.

Detailed voice measurement definitions remain in [the voice guide](voice.md#investigating-high-cpu-during-a-call).
Protocol observability does not change the [compatibility classification](discord-compatibility.md).
