# Desktop audit · November 18, 2026

The starting build was fork `main` at `5e21524c`. The local checks used the
locked workspace and Rust 1.98.1 on Linux x64.

| Area reviewed | Findings and disposition |
| --- | --- |
| Composer, timeline and member state | Match loaded nicknames and usernames before limiting suggestions. Keep exact user IDs selectable and show the nickname in the draft. Reuse role assignments from the readable conversation while member data loads. Live clears and departures take precedence. |
| Model, wire decoding and embed presentation | Keep media with missing dimensions, use approved Reddit and Instagram clips in the native player, and provide an explicit browser action for provider pages. The existing image proxy policy and decoder bounds still apply. |
| SQLite and session caches | Use the account and server scoped ID index for numeric mention searches. Retain search, history and attachment storage limits. |
| REST and Gateway | Bound cooldown admission for unsent HTTP requests. Keep member-query penalties across reconnects and send receipts. |
| Voice, platform and desktop media | Retire stale received video and bound poster work across account changes. Resolve the packaged speech runtime from the real executable path. |
| Scrolling and navigation | Apply a due wheel tail before scheduling another notch. Check anchor, permission and selection behavior with existing regressions. |
| Extensions, native packaging and release workflows | Validate catalog source ports. The baseline ARM64 package contained an x64 `vcruntime140_1.dll` with no importer in the bundled ARM64 files. ARM64 uses the media-v6 archive after a one-time Setup upgrade or fresh portable extraction. |

The embed fixture runs with `profile_preview --demo --page=embeds --social-video-embeds`.
It shows a poster-only OGInstagram reel and a direct Reddit clip. The composer regression
types an older author's nickname, account name and ID, then accepts each suggestion with
Enter. The role-color regression starts with a cached window and applies a departure and
an explicit empty assignment.

The supplied public OGInstagram page returned video and poster metadata. Its signed media
request returned HTTP 403 when checked. Live Discord embed playback and native hardware
checks require owner verification.
