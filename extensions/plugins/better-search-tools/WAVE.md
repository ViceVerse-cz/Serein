# WAVE notes — Better Search Tools

Assigned upstream path: `src/plugins/betterSearch`.

Implemented safe subset: a panel that validates the extra search-token shapes and proposes one
exact native search query after Apply. No filter-registry patch, autocomplete injection or
Webpack access.

## Parent steps

- Add `plugins/better-search-tools` to `extensions/Cargo.toml` workspace members.
- Build and package centrally; do not commit hand-built Wasm in this branch.
- Add the packaged entry and implementation note to `extensions/catalog.json` and
  `extensions/equicord-workflows.json` in the parent integration commit.
- Keep the host `search` effect's 256-byte query limit and current-conversation scope. The
  plugin validates the same limit before proposing.

## Deliberate limits

- The extra tokens are unverified pass-through text; no claim of server support or result
  parity.
- No native filter menu, autocomplete, token parser patch or Discord module access.
