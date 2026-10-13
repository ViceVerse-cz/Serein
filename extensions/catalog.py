#!/usr/bin/env python3
"""Generate and check the immutable extension catalog; canonical package validation lives in Serein.

Package URLs are pinned to a commit on `main`. Pull requests are squash-merged, so a branch
commit can disappear; pins are therefore written after merge (`--pin`, run by CI) from the
merged commit's own tree, never from uncommitted files.
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent
REPOSITORY = "ViceVerse-cz/Serein"
PACKAGE_DIRS = ("themes", "plugins/packages")
CAPABILITIES = {"selected_message", "composer", "storage", "deleted_messages", "image_sharing", "appearance", "rich_presence", "api_proxy", "voice_messages"}
RESERVED = {"con", "prn", "aux", "nul"} | {f"{prefix}{n}" for prefix in ("com", "lpt") for n in range(1, 10)}
MAX_PREVIEW = 256 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def valid_id(value):
    return isinstance(value, str) and re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", value) and value not in RESERVED


def validate(data):
    require(0 < len(data) <= 16 * 1024 * 1024, "package exceeds 16 MiB")
    package = json.loads(data)
    manifest = package["manifest"]
    require(manifest["api_version"] == 1 and valid_id(manifest["id"]), "invalid API version or ID")
    for key in ("name", "version", "author", "license"):
        value = manifest[key]
        require(isinstance(value, str) and 0 < len(value.encode()) <= 128 and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value), f"invalid {key}")
    source = urlsplit(manifest["source"])
    require(len(manifest["source"].encode()) <= 2048 and source.scheme == "https" and source.hostname and not source.username and source.password is None, "credential-free HTTPS source required")
    capabilities = manifest.get("capabilities", [])
    actions = manifest.get("actions", [])
    require(isinstance(capabilities, list) and len(capabilities) <= 4 and len(set(capabilities)) == len(capabilities) and set(capabilities) <= CAPABILITIES, "invalid capabilities")
    require(isinstance(actions, list) and len(actions) <= 16 and len({a["id"] for a in actions}) == len(actions), "invalid action list")
    require(sum(a["surface"] == "activation" for a in actions) <= 1, "multiple activation actions")
    if "api_proxy" in capabilities:
        require(set(capabilities) <= {"api_proxy", "storage"} and all(a["surface"] in ("panel", "activation") for a in actions), "API proxy requires connection-only actions and capabilities")
    for action in actions:
        require(valid_id(action["id"]) and action["surface"] in ("message", "composer", "panel", "activation"), "invalid action")
        label = action["label"]
        require(isinstance(label, str) and 0 < len(label.encode()) <= 128 and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in label), "invalid action label")
        capability = {"message": "selected_message", "composer": "composer"}.get(action["surface"])
        require(capability is None or capability in capabilities, "action lacks required capability")
    for field, limit in (("wasm", 4 * 1024 * 1024), ("background_image", 2 * 1024 * 1024), ("cover_image", 2 * 1024 * 1024)):
        values = package.get(field, [])
        require(isinstance(values, list) and len(values) <= limit and all(type(b) is int and 0 <= b <= 255 for b in values), f"invalid {field} bytes")
    if manifest["kind"] == "theme":
        require(isinstance(package.get("theme"), dict) and not package.get("wasm") and not capabilities and not actions, "invalid declarative theme")
    else:
        require(manifest["kind"] == "plugin" and actions and package.get("theme") is None and not package.get("background_image") and not package.get("cover_image"), "invalid plugin")
        require(bytes(package.get("wasm", [])).startswith(b"\0asm\x01\0\0\0"), "Wasm v1 module required")
    return manifest


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args], stderr=subprocess.PIPE)


class Tree:
    """Catalog inputs read from one commit, so uncommitted files can never be pinned."""

    def __init__(self, commit):
        self.commit = commit
        # Repository-relative prefix of this directory, e.g. `extensions/`.
        self.prefix = git("rev-parse", "--show-prefix").decode().strip()

    def path(self, relative):
        return self.prefix + relative

    def packages(self):
        listed = git("ls-tree", "-r", "--full-tree", "--name-only", self.commit, "--", *(self.path(d) for d in PACKAGE_DIRS)).decode().splitlines()
        return sorted((p[len(self.prefix):] for p in listed if p.endswith(".serein-extension")), key=lambda p: (not p.startswith("themes/"), p))

    def read(self, relative):
        try:
            return git("show", f"{self.commit}:{self.path(relative)}")
        except subprocess.CalledProcessError:
            return None


class WorkingTree:
    """The same inputs from the checkout, for validating a pull request before it is pinned."""

    prefix = ""

    def packages(self):
        return [path.relative_to(ROOT).as_posix() for directory in PACKAGE_DIRS for path in sorted((ROOT / directory).glob("*.serein-extension"))]

    def read(self, relative):
        path = ROOT / relative
        return path.read_bytes() if path.is_file() else None


def entries(tree, base):
    """Validate every package, its source manifest and preview; return catalog entries."""
    paths = tree.packages()
    require(0 < len(paths) <= 256, "catalog requires 1 to 256 packages")
    result, ids = [], set()
    for relative in paths:
        data = tree.read(relative)
        manifest = validate(data)
        require(manifest["id"] not in ids, "duplicate extension ID")
        ids.add(manifest["id"])
        stem = Path(relative).stem
        entry = dict(manifest=manifest, release_url=base + tree.prefix + relative, sha256=hashlib.sha256(data).hexdigest(), download_bytes=len(data), source_commit=getattr(tree, "commit", ""))
        if manifest["kind"] == "plugin":
            source_manifest = tree.read(f"plugins/{stem}/manifest.json")
            require(source_manifest is not None and json.loads(source_manifest) == manifest, f"plugin source manifest differs from package: {stem}")
        image = tree.read(f"previews/{stem}.png")
        if image is not None:
            require(0 < len(image) <= MAX_PREVIEW, f"invalid preview: {stem}")
            entry["preview"] = dict(url=base + tree.prefix + f"previews/{stem}.png", sha256=hashlib.sha256(image).hexdigest(), download_bytes=len(image))
        result.append(entry)
    return result


def render(catalog_entries):
    output = (json.dumps(dict(api_version=1, entries=catalog_entries), indent=2, ensure_ascii=False) + "\n").encode()
    require(len(output) <= 1024 * 1024, "catalog exceeds 1 MiB")
    return output


def generate(commit, repository):
    require(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit), "explicit full Git commit SHA required")
    require(git("rev-parse", f"{commit}^{{commit}}").decode().strip() == commit, "commit must exist locally")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "repository must be owner/name")
    return render(entries(Tree(commit), f"https://raw.githubusercontent.com/{repository}/{commit}/"))


def content(catalog, repository):
    """What a client verifies, without the commit the URLs point at. None if pinned elsewhere."""
    base = f"https://raw.githubusercontent.com/{repository}/"
    result = []
    for entry in catalog["entries"]:
        urls = [entry["release_url"]] + ([entry["preview"]["url"]] if "preview" in entry else [])
        commit = entry["source_commit"]
        if not all(url.startswith(f"{base}{commit}/") for url in urls):
            return None
        stripped = {key: value for key, value in entry.items() if key not in ("release_url", "source_commit")}
        if "preview" in stripped:
            stripped["preview"] = {key: value for key, value in stripped["preview"].items() if key != "url"}
        stripped["path"] = entry["release_url"][len(base) + len(commit) + 1:]
        result.append(stripped)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--validate", action="store_true", help="validate the checkout's packages without Git (pull requests)")
    mode.add_argument("--pin", metavar="COMMIT", help="write the catalog for this merged commit if package content or the repository changed")
    mode.add_argument("--commit", metavar="COMMIT", help="always rewrite the catalog pinned to this commit")
    mode.add_argument("--check", action="store_true", help="verify the catalog matches its pinned commit (needs that commit locally)")
    parser.add_argument("--repository", default=REPOSITORY)
    args = parser.parse_args()
    catalog = ROOT / "catalog.json"
    if args.validate:
        count = len(entries(WorkingTree(), "https://example.invalid/"))
        print(f"Validated {count} packages in the checkout")
        return
    if args.check:
        commits = {entry["source_commit"] for entry in json.loads(catalog.read_bytes())["entries"]}
        require(len(commits) == 1, "catalog must pin one package commit")
        commit = commits.pop()
        require(catalog.read_bytes() == generate(commit, args.repository), "catalog differs from its pinned commit; regenerate it")
        print(f"Catalog verified at {commit}")
        return
    commit = args.pin or args.commit
    output = generate(commit, args.repository)
    if args.pin and catalog.is_file():
        current = content(json.loads(catalog.read_bytes()), args.repository)
        if current is not None and current == content(json.loads(output), args.repository):
            print("Catalog is already pinned to these package bytes")
            return
    catalog.write_bytes(output)
    print(f"Catalog pinned: {len(json.loads(output)['entries'])} packages at {commit}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        sys.exit(f"Catalog error: {error}")
