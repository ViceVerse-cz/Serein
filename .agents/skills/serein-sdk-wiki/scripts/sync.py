"""Generate reviewed wiki pages from an immutable, pushed Serein source commit."""
import argparse
import posixpath
import re
import subprocess
from pathlib import Path
from urllib.parse import quote, urlsplit

REPOSITORY = Path(__file__).resolve().parents[4]
GITHUB = "https://github.com/ViceVerse-cz/Serein"
WIKI_ORIGINS = {GITHUB + ".wiki.git", "git@github.com:ViceVerse-cz/Serein.wiki.git"}
SOURCES = ("examples/extensions/README.md", "docs/extensions.md", "docs/theme-api.md")


def git(directory, *arguments):
    result = subprocess.run(
        ["git", "-C", str(directory), *arguments],
        capture_output=True, text=True, encoding="utf-8", check=True,
    )
    return result.stdout.strip()


def section(text, heading):
    marker = "## " + heading + "\n"
    start = text.index(marker)
    end = text.find("\n## ", start + len(marker))
    return text[start:end if end >= 0 else len(text)].strip()


def repository_links(text, source, revision, tracked):
    def rewrite(match):
        target = match[2]
        url = urlsplit(target)
        if url.scheme or url.netloc or not url.path:
            return match[0]
        path = posixpath.normpath(posixpath.join(posixpath.dirname(source), url.path))
        if path.startswith("../") or path.startswith("/"):
            raise ValueError(f"link leaves repository in {source}: {target}")
        if path in tracked:
            kind = "blob"
        elif any(name.startswith(path.rstrip("/") + "/") for name in tracked):
            kind = "tree"
        else:
            raise ValueError(f"missing link target at {revision}: {source} -> {target}")
        suffix = ("?" + url.query if url.query else "") + ("#" + url.fragment if url.fragment else "")
        return match[1] + f"{GITHUB}/{kind}/{revision}/{quote(path)}{suffix}" + match[3]

    # Canonical guides use inline links; do not rewrite illustrative code blocks.
    output = []
    fence = None
    for line in text.splitlines():
        marker = re.match(r"^\s*(`{3,}|~{3,})", line)
        if marker:
            token = marker[1]
            if fence is None:
                fence = token
            elif token[0] == fence[0] and len(token) >= len(fence):
                fence = None
        elif fence is None:
            line = re.sub(r"(!?\[[^\]\n]*\]\()([^\s)]+)(\))", rewrite, line)
        output.append(line)
    return "\n".join(output)


def pages(revision, status):
    tracked = set(git(REPOSITORY, "ls-tree", "-r", "--name-only", revision).splitlines())
    sources = {
        source: repository_links(git(REPOSITORY, "show", f"{revision}:{source}"), source, revision, tracked)
        for source in SOURCES
    }
    sdk, extensions, theme = (sources[source] for source in SOURCES)
    notice = f"> **{status}**\n> Source: [Serein `{revision[:12]}`]({GITHUB}/tree/{revision}).\n\n"
    navigation = (
        "- [Create a plugin](Creating-a-Plugin)\n"
        "- [Create a theme](Creating-a-Theme)\n"
        "- [Test and package](Testing-and-Packaging)\n"
        "- [Publish to the community catalog](Publishing-to-the-Community-Catalog)\n"
        "- [API and security reference](API-and-Security-Reference)\n"
        "- [Capability reference](API-and-Security-Reference#capability-reference)\n"
    )
    return {
        "Home.md": notice + "# Serein extension SDK\n\n"
        "Build local, opt-in Wasm plugins and declarative themes for the native client. "
        "Start with an offline example and request only the capabilities your plugin needs.\n\n"
        "## Creator guides\n\n" + navigation + "\n## Examples and source\n\n"
        f"- [Rust SDK and complete example plugins]({GITHUB}/tree/{revision}/examples/extensions)\n"
        f"- [Versioned SDK authoring guide]({GITHUB}/blob/{revision}/examples/extensions/README.md)\n"
        f"- [Extension host contract]({GITHUB}/blob/{revision}/docs/extensions.md)\n"
        f"- [Theme schema]({GITHUB}/blob/{revision}/docs/theme-api.md)\n\n"
        "Plugins cannot call Discord, send messages, access credentials, open files or use the network. "
        "A supporting host and explicit user grants are required for each capability.\n",
        "_Sidebar.md": "## Creator guides\n\n- [Home](Home)\n" + navigation + f"\n[Serein source]({GITHUB}/tree/{revision})\n",
        "Creating-a-Plugin.md": notice + sdk + "\n",
        "Creating-a-Theme.md": notice + theme + "\n",
        "API-and-Security-Reference.md": notice + "# API and security reference\n\n"
        + section(extensions, "Host contract") + "\n\n" + section(extensions, "Resource and privacy limits") + "\n",
        "Testing-and-Packaging.md": notice + "# Test and package an extension\n\n"
        + sdk.split("\n## ", 1)[0].split("\n", 1)[1].strip() + "\n\n"
        + section(sdk, "Test and develop locally") + "\n\n" + section(extensions, "Install and remove") + "\n",
        "Publishing-to-the-Community-Catalog.md": notice + "# Publish to the community catalog\n\n"
        + section(extensions, "Creator workflow") + "\n\n" + section(extensions, "Shop previews") + "\n",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-ref", required=True, help="full immutable commit already pushed to origin")
    parser.add_argument("--wiki-dir", type=Path, required=True, help="existing Serein wiki clone")
    parser.add_argument("--status", required=True, help="explicit preview or verified release label")
    parser.add_argument("--check", action="store_true", help="compare generated pages without writing")
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", args.source_ref):
            raise ValueError("--source-ref must be a full immutable commit ID")
        revision = git(REPOSITORY, "rev-parse", "--verify", args.source_ref + "^{commit}")
        if revision.lower() != args.source_ref.lower():
            raise ValueError("--source-ref must identify a commit directly")
        if not git(REPOSITORY, "for-each-ref", "--contains", revision, "--format=%(refname)", "refs/remotes/origin/"):
            raise ValueError("source commit is not on a fetched origin branch; push and fetch first")
        directory = args.wiki_dir.resolve(strict=True)
        if Path(git(directory, "rev-parse", "--show-toplevel")).resolve() != directory:
            raise ValueError("--wiki-dir must name the wiki checkout root")
        if git(directory, "remote", "get-url", "origin") not in WIKI_ORIGINS:
            raise ValueError("wiki origin must be the existing ViceVerse-cz/Serein.wiki.git remote")
        if not args.status.strip() or len(args.status) > 256 or any(ord(c) < 32 for c in args.status):
            raise ValueError("--status must be one nonempty line, at most 256 characters")
        generated = pages(revision, args.status)
        changed = []
        for name, content in generated.items():
            path = directory / name
            if path.is_symlink():
                raise ValueError(f"refusing to overwrite a symlink: {name}")
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                changed.append(name)
        if args.check:
            if changed:
                raise ValueError("wiki pages differ: " + ", ".join(changed))
            print(f"All {len(generated)} wiki pages match {revision}.")
        else:
            for name in changed:
                (directory / name).write_text(generated[name], encoding="utf-8", newline="\n")
            print(f"Generated {len(changed)} changed pages from {revision}; nothing committed or pushed.")
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Wiki generation failed: {error}\n")


if __name__ == "__main__":
    main()
