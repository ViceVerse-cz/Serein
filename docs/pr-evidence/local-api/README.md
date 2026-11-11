# Local API evidence

Baseline: `09bee5a069f15f3ec7cb617ef8dd8694ed10be31` (`origin/main` when work began), built
from an untouched separate worktree. The candidate is this PR. Both use the pinned Rust
1.99.1 MSVC toolchain and the lockfile. No account, Discord connection, microphone or live
media was used; every client below is a synthetic script on the same computer.

Environment: Windows 11 Pro 10.0.26200, Intel Core i9-14900KS, 95.7 GiB RAM, NVIDIA RTX PRO
6000, display scale 1.0.

## Why

`motivation.png` is the owner's Stream Deck + in the Stream Deck app. A voice-channel
mixer plugin is assigned to the keys, one of them set to mute a person in the call, and
with Serein as the client it has nothing to connect to, so every key reads "No bridge".
This is the only image here that is not Serein's synthetic fixture; it contains no
account or conversation data.

## Native synthetic screenshots

Debug builds with `--features demo`, launched with `--demo --demo-settings=storage`, the
window sized to 1280 × 900 and Data & Privacy scrolled to its end. Only the app window
is captured. The fixture ignores saved preferences and never persists these edits.

- `before.png`: the baseline page ends with Saved login and Your privacy.
- `after-off.png`: the new Local API group in its default state, both switches off.
- `after.png`: launched with `--demo-local-api`: listening, with the address and key rows.
  The fixture never uses the credential store, so the key row says it is not saved.
- `after-light-narrow.png`: 840 × 650, light theme, `--demo-local-api=control`.

Both palettes, the narrow layout, the disabled second switch and the Copied feedback were
inspected. The first version of this change added a sidebar page instead; the baseline
comparison showed that the extra row pushed the sidebar's "Unofficial · not endorsed by
Discord" line out of view at the tallest modal height, so the controls moved into the
existing page. Linux and macOS surfaces are untested.

To reproduce, build each revision and launch it:

```sh
cargo build --locked -p serein --features demo --bin serein
target/debug/serein --demo --demo-settings=storage --demo-local-api
```

## Behavior checked against the running fixture

A Python script speaking the documented protocol over `\\.\pipe\serein-api`, against the
candidate launched with `--demo --demo-voice --demo-local-api=control`. The key was taken
with the page's own Copy key button.

| Check | Result |
| --- | --- |
| Wrong key | `{"op":"error","code":"unauthorized"}`, then closed |
| Right key | `ready`, and Serein's proof verified by the client |
| `subscribe` to `session` and `voice` | The fixture's account and its three call members |
| `voice.set_self` | Accepted, then a `self` event with the new mute state |
| `voice.set_user` volume 150 for a call member | Accepted, then `member_update` with `volume: 150` |
| `voice.set_user` for an ID not in the call | `unknown_user` |
| **Allow apps to control voice** switched off | A `session` event with `control: false` |
| **Enable the local API** switched off | The connection closes and the pipe no longer exists |
| Connected app | Listed by name in the group while connected |
| API switched off and on again | The pipe goes and returns; the unsaved key is the same |
| **Reset key** | A new key is accepted; the old one no longer connects |

This found one defect, fixed before delivery: switching the API off first told subscribers
the owner had signed out. The end-to-end test now covers it. The last two rows, and the
control rows again, were run on the review follow-up commit.

## Measurements

### Package size

Standard release packages from `cargo xtask package` with the unchanged fat-LTO profile.
`makensis` is not installed here, so no installer was produced; the compressed figure is
the package folder as a Deflate level 9 ZIP (Python `zipfile`, fixed timestamps) made the
same way for both.

| Metric | Baseline | Candidate | Delta |
| --- | ---: | ---: | ---: |
| Release executable (`dist/serein.exe`) | 98,461,184 B | 98,733,056 B | +271,872 B (+0.276%) |
| `dist/` regular-file payload (233 files) | 102,834,845 B | 103,107,005 B | +272,160 B (+0.265%) |
| ZIP of `dist/` | 54,772,411 B | 54,863,954 B | +91,543 B (+0.167%) |

The change adds one direct dependency, `hmac` 0.12.1, which the lockfile already contained
through the TLS and voice stacks; no new crate is built.

### Idle process and request latency

These use the debug demo builds above, not release binaries, launched with
`--demo --demo-voice --demo-settings=storage` at 1280 × 900. Each run is one 10-second
warmup and 20 samples at one-second intervals; three runs per row in mixed order, median
shown with the range. CPU is the process's processor time over elapsed time (100% is one
core); memory is working set and private bytes from the same samples.

| Scenario | CPU | Working set | Private bytes |
| --- | --- | --- | --- |
| Baseline | 12.1% (0.0–17.6) | 362.9 MiB (355.2–363.2) | 421.7 MiB (414.4–421.8) |
| Candidate, API off (default) | 14.5% (0.9–17.2) | 365.2 MiB (356.4–379.5) | 421.3 MiB (412.3–437.4) |
| Candidate, API on, one subscribed client | 2.6% (0.5–6.9) | 365.6 MiB (365.2–365.6) | 422.3 MiB (421.7–422.5) |

Idle CPU fell into two groups (about 0–3% and 12–18%) in every scenario, baseline
included, depending on something outside this change such as where the window opened.
The spread within each row is larger than any difference between rows, so these runs show
no measurable CPU or memory cost and support no claim of an improvement either.

Request latency, same debug build with the fixture's three-member call: a `get` of the
`voice` topic, 2,000 requests after 200 warmup requests, measured by the client around
write and read on one connection.

| Run | Median | p99 | Max |
| --- | --- | --- | --- |
| 1 | 108 µs | 271 µs | 433 µs |
| 2 | 55 µs | 175 µs | 531 µs |
| 3 | 57 µs | 148 µs | 378 µs |

This is a debug build on one machine with uncontrolled background load, including the
Python client's own overhead. It is not a release figure, a throughput figure or a
measurement of event delivery, which also waits for the next UI frame. Release CPU and
RSS, frame latency, Linux and macOS were not measured.
