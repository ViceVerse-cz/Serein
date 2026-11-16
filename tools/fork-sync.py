#!/usr/bin/env python3
"""Merge upstream main into the fork and resolve the conflicts we knowingly create.

The fork deletes upstream's Linux/macOS packaging, rewrites README.md and adds a
few lines to shared files, so a plain `git merge upstream/main` stops on the same
conflicts after almost every upstream change. This script keeps the merge moving
with a narrow, auditable policy and refuses to guess when a conflict is a real
overlap of edits:

* a path the fork deleted stays deleted;
* README.md keeps the fork version; upstream's copy is refreshed in
  README.upstream.md;
* Cargo.lock takes the upstream version and is regenerated with `cargo metadata`
  (the fork adds the `zapret` crate, so the lock has to include it);
* every other text conflict is accepted only when both sides *inserted* lines at
  the same place (a new locale key, a new module declaration, ...). If the base
  side of the conflict is not empty, the edits genuinely overlap: the merge is
  aborted and the workflow fails with the list of files to resolve by hand.

Run it from the repository root: `python3 tools/fork-sync.py`. It writes
`changed=true|false` to $GITHUB_OUTPUT when that variable is set.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

UPSTREAM_REF = "upstream/main"
README = "README.md"
README_UPSTREAM = "README.upstream.md"
LOCKFILE = "Cargo.lock"
# Regenerate the lockfile when the merge touched any manifest it depends on.
LOCK_TRIGGERS = ("Cargo.toml", LOCKFILE)

MARKER_OURS = b"<<<<<<< "
MARKER_BASE = b"||||||| "
MARKER_SEP = b"======="
MARKER_THEIRS = b">>>>>>> "

SUMMARY: list[str] = []


def run(args: list[str], check: bool = True, capture: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(args, capture_output=capture)
    if check and result.returncode != 0:
        if capture:
            sys.stderr.buffer.write(result.stderr or b"")
        raise SystemExit(f"command failed: {' '.join(args)}")
    return result


def git(*args: str) -> subprocess.CompletedProcess[bytes]:
    return run(["git", *args])


def note(message: str) -> None:
    SUMMARY.append(message)
    print(message, flush=True)


def set_output(changed: bool) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if not output:
        return
    with open(output, "a", encoding="utf-8") as handle:
        handle.write(f"changed={'true' if changed else 'false'}\n")


def unmerged_paths() -> dict[str, dict[int, str]]:
    """Return {path: {stage: blob sha}} for every unmerged index entry."""
    entries: dict[str, dict[int, str]] = {}
    raw = git("ls-files", "--unmerged", "-z").stdout
    for record in raw.split(b"\0"):
        if not record:
            continue
        meta, _, path = record.partition(b"\t")
        _mode, sha, stage = meta.split()
        entries.setdefault(path.decode("utf-8"), {})[int(stage)] = sha.decode("ascii")
    return entries


def blob(sha: str) -> bytes:
    return git("cat-file", "blob", sha).stdout


def merge_insertions(ours: bytes, base: bytes, theirs: bytes) -> bytes | None:
    """Return the union of both sides, or None when the edits really overlap."""
    with tempfile.TemporaryDirectory() as directory:
        paths = []
        for name, content in (("ours", ours), ("base", base), ("theirs", theirs)):
            path = Path(directory) / name
            path.write_bytes(content)
            paths.append(str(path))
        merged = run(["git", "merge-file", "--diff3", "-p", *paths], check=False)

    if merged.returncode < 0:
        return None
    if merged.returncode == 0:
        return merged.stdout

    lines = merged.stdout.splitlines(keepends=True)
    result: list[bytes] = []
    resolved_any = False
    index = 0
    while index < len(lines):
        line = lines[index]
        if not line.startswith(MARKER_OURS):
            result.append(line)
            index += 1
            continue

        def find(start: int, predicate) -> int:
            for position in range(start, len(lines)):
                if predicate(lines[position]):
                    return position
            return -1

        base_start = find(index + 1, lambda value: value.startswith(MARKER_BASE))
        separator = find(index + 1, lambda value: value.rstrip(b"\r\n") == MARKER_SEP)
        end = find(index + 1, lambda value: value.startswith(MARKER_THEIRS))
        if min(base_start, separator, end) < 0 or not base_start < separator < end:
            return None

        # A non-empty base section means one side changed or deleted existing
        # lines; concatenating both sides would silently corrupt the file.
        if b"".join(lines[base_start + 1 : separator]).strip():
            return None

        result.extend(lines[index + 1 : base_start])
        result.extend(lines[separator + 1 : end])
        resolved_any = True
        index = end + 1

    # A failed merge-file run without markers must not pass for a clean merge.
    return b"".join(result) if resolved_any else None


def resolve(path: str, stages: dict[int, str]) -> bool:
    """Resolve one conflict. Return False when it needs a human."""
    if 2 not in stages:
        # Deleted by the fork, modified upstream: the fork's deletion wins.
        git("rm", "-f", "--quiet", "--", path)
        return True

    if path == README:
        Path(path).write_bytes(blob(stages[2]))
        git("add", "--", path)
        if 3 in stages:
            Path(README_UPSTREAM).write_bytes(blob(stages[3]))
            git("add", "--", README_UPSTREAM)
        return True

    if path == LOCKFILE:
        if 3 not in stages:
            return False
        Path(path).write_bytes(blob(stages[3]))
        git("add", "--", path)
        return True

    if 1 not in stages or 3 not in stages:
        return False

    merged = merge_insertions(blob(stages[2]), blob(stages[1]), blob(stages[3]))
    if merged is None:
        return False
    Path(path).write_bytes(merged)
    git("add", "--", path)
    return True


def regenerate_lockfile() -> None:
    changed = git("diff", "--name-only", "HEAD", "--").stdout.decode("utf-8").splitlines()
    if not any(name.endswith(LOCK_TRIGGERS) for name in changed):
        return
    if shutil.which("cargo") is None:
        print("warning: cargo not found, keeping the upstream Cargo.lock", file=sys.stderr)
        return
    print("regenerating Cargo.lock with cargo metadata ...", flush=True)
    metadata = run(["cargo", "metadata", "--format-version", "1"], check=False)
    if metadata.returncode != 0:
        print("warning: cargo metadata failed, keeping the upstream Cargo.lock", file=sys.stderr)
        return
    if git("status", "--porcelain", "--", LOCKFILE).stdout.strip():
        git("add", "--", LOCKFILE)
        print("  Cargo.lock: regenerated with cargo metadata", flush=True)


def commit_message() -> str:
    lines = ["Merge upstream main", "", "Auto-resolved by tools/fork-sync.py:"]
    lines.extend(SUMMARY)
    return "\n".join(lines) + "\n"


def self_test() -> None:
    base = b"a\nb\n"
    merged = merge_insertions(b"a\nours\nb\n", base, b"a\ntheirs\nb\n")
    assert merged == b"a\nours\ntheirs\nb\n", merged
    # Overlapping edits of an existing line are never guessed.
    assert merge_insertions(b"a\nours\nb\n", base, b"a\ntheirs\nb2\n") is None
    print("fork-sync self-test passed")


def main() -> int:
    before = git("rev-parse", "HEAD").stdout.strip()
    merge = run(["git", "merge", "--no-edit", UPSTREAM_REF], check=False, capture=False)
    if merge.returncode == 0:
        changed = git("rev-parse", "HEAD").stdout.strip() != before
        if changed:
            print("merged upstream cleanly; no conflicts to resolve", flush=True)
        else:
            print("upstream is already merged; nothing to do", flush=True)
        set_output(changed)
        return 0

    conflicts = unmerged_paths()
    if not conflicts:
        print("merge failed without unmerged paths; resolve it by hand", file=sys.stderr)
        return 1

    deleted: list[str] = []
    additions: list[str] = []
    unresolved: list[str] = []
    for path, stages in sorted(conflicts.items()):
        if resolve(path, stages):
            if path == README:
                note("  README.md: kept the fork version, refreshed README.upstream.md")
            elif path == LOCKFILE:
                note("  Cargo.lock: took the upstream version")
            elif 2 not in stages:
                deleted.append(path)
            else:
                additions.append(path)
        else:
            unresolved.append(path)

    if unresolved:
        run(["git", "merge", "--abort"], check=False)
        print("", file=sys.stderr)
        print("Unresolved conflicts (edits overlap, resolve by hand):", file=sys.stderr)
        for path in unresolved:
            print(f"  {path}", file=sys.stderr)
        print(
            "\nRun `git merge upstream/main`, resolve these paths, then push and "
            "re-run the build. Nothing was pushed.",
            file=sys.stderr,
        )
        return 1

    if deleted:
        note(f"  kept {len(deleted)} fork deletions")
    if additions:
        note("  merged additions: " + ", ".join(additions))

    regenerate_lockfile()
    # `git commit -F -` needs stdin, which run() cannot feed; write a temp file.
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
        handle.write(commit_message())
        message_path = handle.name
    try:
        run(["git", "commit", "--quiet", "-F", message_path], capture=False)
    finally:
        os.unlink(message_path)

    print("\nmerge commit ready:", flush=True)
    print(git("log", "-1", "--oneline").stdout.decode("utf-8").strip(), flush=True)
    set_output(True)
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        self_test()
        sys.exit(0)
    sys.exit(main())
