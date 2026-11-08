"""Run with python test_catalog.py; no external test dependencies."""
import json

import catalog
import pack

theme = (catalog.ROOT / "themes/ocean.serein-extension").read_bytes()
plugin = (catalog.ROOT / "plugins/packages/message-delete-protector.serein-extension").read_bytes()
assert catalog.validate(theme)["id"] == "serein-ocean"
assert catalog.validate(plugin)["kind"] == "plugin"
rpc = (catalog.ROOT / "plugins/packages/custom-rpc.serein-extension").read_bytes()
assert catalog.validate(rpc)["capabilities"] == ["rich_presence", "storage"]
proxy = (catalog.ROOT / "plugins/packages/api-proxy.serein-extension").read_bytes()
assert catalog.validate(proxy)["capabilities"] == ["api_proxy", "storage"]
for data, mutate in (
    (proxy, lambda p: p["manifest"].update(capabilities=["api_proxy", "composer"])),
    (proxy, lambda p: p["manifest"]["actions"][0].update(surface="message")),
    (theme, lambda p: p["manifest"].update(id="../escape")),
    (theme, lambda p: p["manifest"].update(capabilities=["composer"])),
    (theme, lambda p: p.update(cover_image=[256])),
    (plugin, lambda p: p["manifest"].update(source="https://user:secret@example.com")),
    (plugin, lambda p: p.update(wasm=[0, 1])),
    (plugin, lambda p: p["manifest"].update(capabilities=["unknown"])),
    (plugin, lambda p: p["manifest"]["actions"][0].update(surface="composer")),
):
    invalid = json.loads(data)
    mutate(invalid)
    try:
        catalog.validate(json.dumps(invalid).encode())
    except ValueError:
        pass
    else:
        raise AssertionError("invalid extension accepted")
typed_capability = json.loads(plugin)
typed_capability["manifest"].update(
    capabilities=["selected_message", "clipboard_write"],
    actions=[{"id": "copy", "label": "Copy", "surface": "message"}],
)
assert catalog.validate(json.dumps(typed_capability).encode())["capabilities"] == ["selected_message", "clipboard_write"]
event_plugin = json.loads(plugin)
event_plugin["manifest"].update(
    capabilities=["message_events"],
    actions=[{"id": "on-message", "label": "Message event", "surface": "message_event"}],
)
assert catalog.validate(json.dumps(event_plugin).encode())["capabilities"] == ["message_events"]
event_plugin["manifest"]["capabilities"].append("data_events")
try:
    catalog.validate(json.dumps(event_plugin).encode())
except ValueError:
    pass
else:
    raise AssertionError("data events accepted without app events")
# Equicord-derived packages distribute their full GPL text in an inert custom section.
expected_gpl = (catalog.ROOT / "licenses/Equicord-GPL-3.0.txt").read_bytes()
for stem in [
    "offline-composer-tools", "text-commands", "view-raw-message", "external-search-tools",
    "text-codecs", "message-composition-tools", "identity-tools", "loaded-role-members",
    "status-presets", "message-history", "message-reference-tools", "draft-style-tools",
    "command-palette", "ingtoninator", "own-message-sed", "social-link-tools",
    "voice-panel-tools", "server-info-tools", "equicord-community-tools", "permissions-tools",
    "plus-reacts-tools", "message-inspector-tools", "voice-rejoin-tools", "account-navigation-tools",
    "message-navigation-tools", "music-link-tools", "more-quick-reactions-tools", "linked-message-tools",
    "greet-sticker-tools", "better-search-tools", "voice-and-chat-tools",
]:
    packaged = json.loads((catalog.ROOT / f"plugins/packages/{stem}.serein-extension").read_bytes())
    wasm = bytes(packaged["wasm"])
    licenses = [payload for name, payload in pack.custom_sections(wasm) if name == pack.LICENSE_SECTION]
    assert licenses == [expected_gpl], stem
    assert pack.embed_license(wasm, expected_gpl) == wasm

# Pins are compared by content; a catalog pinned to another repository is always re-pinned.
pinned = json.loads((catalog.ROOT / "catalog.json").read_bytes())
for entry in pinned["entries"]:
    commit = entry["source_commit"]
    for value in [entry] + ([entry["preview"]] if "preview" in entry else []):
        key = "release_url" if value is entry else "url"
        value[key] = value[key].replace("ViceVerse-cz/Serein-extensions/" + commit + "/", "ViceVerse-cz/Serein/" + commit + "/extensions/")
moved = json.loads(json.dumps(pinned))
for entry in moved["entries"]:
    old = entry["source_commit"]
    entry["source_commit"] = "f" * 40
    for value in [entry] + ([entry["preview"]] if "preview" in entry else []):
        key = "release_url" if value is entry else "url"
        value[key] = value[key].replace(old, "f" * 40)
assert catalog.content(pinned, "ViceVerse-cz/Serein") == catalog.content(moved, "ViceVerse-cz/Serein") is not None
assert catalog.content(pinned, "ViceVerse-cz/Other") is None
moved["entries"][0]["sha256"] = "0" * 64
assert catalog.content(pinned, "ViceVerse-cz/Serein") != catalog.content(moved, "ViceVerse-cz/Serein")
class OversizedCatalog:
    prefix = ""

    def packages(self):
        return [f"plugins/packages/package-{index}.serein-extension" for index in range(513)]

try:
    catalog.entries(OversizedCatalog(), "https://example.invalid/")
except ValueError:
    pass
else:
    raise AssertionError("513-entry catalog accepted")
coverage = json.loads((catalog.ROOT / "equicord-workflows.json").read_bytes())
assert coverage["upstream"] == {
    "repository": "https://github.com/Equicord/Equicord",
    "commit": "200dba124066146a8f043de7fc8b05e3dc5e68e",
}
assert len(coverage["plugins"]) == 373
assert len({row["path"] for row in coverage["plugins"]}) == 373
assert all(row["path"].startswith(("src/plugins/", "src/equicordplugins/")) for row in coverage["plugins"])
for row in coverage["plugins"]:
    assert row["disposition"] in {"internal", "already_native", "already_ported", "safe_candidate", "bounded_candidate", "unsuitable"}
    if "implementation" in row:
        implemented = row["implementation"]
        assert implemented["status"] in {"packaged", "review_pending", "native"}
        if implemented["status"] != "native":
            assert (catalog.ROOT / implemented["package_path"].removeprefix("extensions/")).is_file(), row["path"]
package_workflows = sum(row.get("implementation", {}).get("status") in {"packaged", "review_pending"} for row in coverage["plugins"])
native_workflows = sum(row.get("implementation", {}).get("status") == "native" for row in coverage["plugins"])
mapped = [row["implementation"] for row in coverage["plugins"] if row.get("implementation", {}).get("status") in {"packaged", "review_pending"}]
assert coverage["coverage_totals"]["mapped_packages"] == len({item["package"] for item in mapped})
assert coverage["coverage_totals"]["review_pending_packages"] == len({item["package"] for item in mapped if item["status"] == "review_pending"})
assert coverage["coverage_totals"]["mapped_upstream_workflows"] == package_workflows
assert coverage["coverage_totals"]["native_workflows"] == native_workflows
assert coverage["coverage_totals"]["total_workflow_mappings"] == package_workflows + native_workflows
print("Theme/plugin identity, byte, capability, action, URL, catalog bounds and 373-workflow inventory checks passed.")
