# noBlockedMessages native preference — synthetic evidence

These captures use the same 1120×760, dark, C.UTF-8 Xvfb application window and isolated HOME/XDG
configuration, launched with `--demo --demo-settings=chat`. No saved session, live message,
relationship write or Discord request was used.

- `before.png`: the fork `main` tree at `2e673c47`, captured before the relationship-filter
  settings existed.
- `after.png`: this branch, with both new preferences default off and **Also hide ignored users'
  messages** disabled until the primary preference is enabled.
- `after-enabled.png`: native clicks on each toggle, enabling the primary filter and then its
  dependent ignored-author option.

The captures were taken while this work was based on the fork's `main`. It has since been rebased
onto `ViceVerse-cz/Serein:main` at `29a390ce`, and the surrounding Chat settings page has not
changed in that range, so the images still show the same controls and layout; they were not
re-captured after the rebase.

The screenshots establish the settings layout and interaction, not live filtering. The focused
regression test constructs known blocked, known ignored and unknown authors through the real
state events, then verifies the visible IDs under both options. It also proves that all three rows
remain in the retained timeline. Main and thread-starter rows use the same presentation predicate,
and the virtual-list iterator excludes filtered rows instead of leaving blank slots.

The two choices are persisted as booleans in the existing bounded device preferences singleton;
there is no hidden-message, author-ID or relationship cache. Unknown relationship state remains
visible, unread/read state is unchanged, and the existing Friends restrictions manager and catalog
identity are untouched. The behavior is an original native adaptation of Equicord's
`noBlockedMessages`, not JavaScript renderer patches or another blocked-accounts manager.

The pre-rebase implementation passed `cargo xtask check` with **1,659 passed, 0 failed, 14
ignored**. The rebased branch passed `cargo check` for the affected crates and the focused
`local-store`, `serein` and `ui` tests.
