# Custom Rich Presence

A native Serein editor inspired by Vencord CustomRPC. Configure Playing, Streaming,
Listening, Watching or Competing activities, text links, artwork and image links,
two buttons, party size, and elapsed/countdown timers without editing JSON.

1. Install the reviewed package in **Settings → Extensions** and enable its rich
   presence capability.
2. Choose **Edit rich presence**. Enter your Discord application ID and activity
   name, then any optional fields. Artwork accepts application asset keys or
   public HTTPS image URLs.
3. **Preview** validates and displays a local activity card. **Apply presence**
   saves the fields and submits the activity. Draft edits never change presence.
4. **Stop presence** clears this plugin's activity and disables automatic resume;
   your last applied fields remain saved. **Reset draft fields** only clears the
   editor draft. Reopening the editor restores the last applied fields.

Saved enabled presence resumes on plugin activation. Disabling the plugin removes
its activity. Account visibility, activity sharing, connection state and Discord's
handling of unofficial fields can affect what other users see. The preview is a
local representation, not live cross-client proof. It does not load remote images.

Custom times use `YYYY-MM-DD HH:mm` in UTC; provide a start for elapsed time or an
end for a countdown. “Since starting” measures from the first application of the
presence and keeps that start when you apply edits; stopping and starting again
resets it. “Since local midnight” uses the host's local day. Timers and other
fields are validated before publication.

Requires a host with the `rich_presence` capability (Serein PR #465). It builds
against the in-repository SDK; `crates/extensions/tests/sandbox.rs` runs the
committed package through the real host sandbox.

Build from `extensions/`:

```sh
cargo test --locked -p custom-rpc
cargo build --locked --release --target wasm32-unknown-unknown -p custom-rpc
python pack.py plugins/custom-rpc/manifest.json target/wasm32-unknown-unknown/release/custom_rpc.wasm plugins/packages/custom-rpc.serein-extension
```

The capability contributes one bounded activity through Serein's existing
presence pipeline. The plugin has no network, filesystem or account credentials.
