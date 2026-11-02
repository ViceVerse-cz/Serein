# Preload on hover setting

Baseline `2df8256c` (main) and this PR's branch, each a debug `profile_preview --features demo`
build with the `chat` fixture page (added by this PR; the baseline build had the same one-line
page addition, uncommitted):

```sh
target/debug/examples/profile_preview --demo --width=1440 --height=900 --page=chat --scroll=1600 --output=after.png
```

These are framebuffer exports of the synthetic Chat settings page. No account data or network
requests are involved. Preloading itself has no visible change beyond opening sooner.


## Review repairs - October 14, 2026

The original PR head was `08a9ae01`; the repair also integrates `main` at `cf5452cd`.
All regressions use synthetic state and local pending tasks:

- Start a profile preload, replace the connection or wait 30 seconds, then open that card:
  the click sends a new request. READY for the same account also clears flight ownership.
- Load a short chat, receive more than 500 rows or 4 MiB, switch away and return:
  trusted reopen still permits fetching before the oldest retained row.
- Fill all four speculative task slots with pending reads: the next history/profile read
  receives its own capacity error and every accepted task remains alive. A clicked history
  receiving that error leaves Loading without ending the account session.
- Adopt then close a profile preload, enable frames/effects, then deliver its old response:
  the next open requests fresh cosmetic data. A valid closed card still populates the cache.
- Repeat failed GIF-home loads with close/hover: the thirteenth request within a minute is
  suppressed; cache hits and explicit picker retries still work.

The repairs change loading/cache behavior, without a new UI layout. The original synthetic
settings screenshots above remain feature evidence from the original head. No new native
interaction capture is claimed: the available computer-use surface has no native app API,
and the `orca` executable is unavailable. No live Discord account, call or microphone was used.
