"""Run with python test_catalog.py; no external test dependencies."""
import json

import catalog

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
print("Theme/plugin identity, byte, capability, action and URL checks passed.")
