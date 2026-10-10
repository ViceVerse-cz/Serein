"""Prepare the actual native preview without requiring Linux GTK platform adapters.

Run from either measured checkout. This writes only to the supplied scratch directory.
It adds synthetic long-name reply/zoom flags and fixes tray/startup availability to false.
"""
import pathlib
import shutil
import sys

root = (pathlib.Path(sys.argv[2]).resolve() if len(sys.argv) > 2
        else pathlib.Path(__file__).resolve().parents[3])
output = pathlib.Path(sys.argv[1]).resolve()
output.mkdir(parents=True, exist_ok=True)
source = (root / "apps/desktop/examples/profile_preview.rs").read_text()
for name in ["server_settings_demo", "slash_demo"]:
    source = source.replace(
        f'#[path = "../src/{name}.rs"]',
        f'#[path = "{root}/apps/desktop/src/{name}.rs"]',
    )
source = source.replace('"../../../', f'"{root}/')
source = source.replace("platform::tray::supported()", "false")
source = source.replace("platform::startup::available()", "false")
source = source.replace(
    "ui::fonts::install(&cc.egui_ctx);",
    'cc.egui_ctx.set_zoom_factor(args.iter().find_map(|arg| arg.strip_prefix("--zoom="))'
    '.and_then(|value| value.parse().ok()).unwrap_or(1.0));\n'
    '\t\t\tui::fonts::install(&cc.egui_ctx);',
)
source = source.replace(
    "\t\t\tlet mut messaging = ui::MessagingUi::default();",
    '''\t\t\tif args.iter().any(|arg| arg == "--reply") {
\t\t\t\tlet channel = state.selected.expect("synthetic reply conversation");
\t\t\t\tlet mut message = test_support::message(601, channel);
\t\t\t\tmessage.author.name = "Long synthetic display name · 界🙂 ".repeat(6);
\t\t\t\tmessage.content = "Synthetic long-name reply target".into();
\t\t\t\tmessage.attachments.clear();
\t\t\t\tmessage.embeds.clear();
\t\t\t\tstate.timeline.insert(message, false, false).unwrap();
\t\t\t\tstate.reply = Some(client_core::Reply::to(model::Id(601)));
\t\t\t}
\t\t\tlet mut messaging = ui::MessagingUi::default();''',
)
(output / "main.rs").write_text(source)
manifest = '''[package]
name = "serein-ui-task-preview"
version = "0.1.0"
edition = "2024"
[workspace]
[[bin]]
name = "serein-ui-task-preview"
path = "main.rs"
[dependencies]
'''
for name in ["ui", "client-core", "model", "extensions", "test-support", "discord-protocol"]:
    features = ', features = ["demo"]' if name == "ui" else ""
    manifest += f'{name} = {{ path = "{root}/crates/{name}"{features} }}\n'
manifest += '''eframe = { git = "https://github.com/emilk/egui", rev = "fe6d63efa4a4df6f56ceab814d4f3a6efab69b88", default-features = false, features = ["wgpu", "default_fonts", "system_fonts", "accesskit", "wayland", "x11", "links"] }
image = { version = "=0.25.10", default-features = false, features = ["png"] }
serde_json = { version = "1", features = ["raw_value"] }
[profile.dev]
debug = 0
opt-level = 1
[profile.test]
debug = 0
opt-level = 1
'''
(output / "Cargo.toml").write_text(manifest)
shutil.copyfile(root / "Cargo.lock", output / "Cargo.lock")
print(output / "Cargo.toml")
