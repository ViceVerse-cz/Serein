"""Generate reviewed wiki pages from an immutable, pushed Serein source commit."""
import argparse
import posixpath
import re
import subprocess
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit

REPOSITORY = Path(__file__).resolve().parents[4]
GITHUB = "https://github.com/ViceVerse-cz/Serein"
WIKI_ORIGINS = {GITHUB + ".wiki.git", "git@github.com:ViceVerse-cz/Serein.wiki.git"}
SOURCES = (
    "examples/extensions/README.md", "docs/extensions.md", "docs/theme-api.md",
    "docs/extension-sdk-reference.md", "docs/extension-sdk-actions.md",
    "docs/extension-sdk-overview.md", "docs/extension-sdk-troubleshooting.md",
)
# Each section becomes its own wiki page. Source-code links stay commit-pinned.
GUIDES = {
    SOURCES[0]: {"": "Creating-a-Plugin"},
    SOURCES[1]: {
        "capability-reference": "API-and-Security-Reference",
        "resource-and-privacy-limits": "API-and-Security-Reference",
    },
    SOURCES[2]: {"": "Creating-a-Theme"},
    SOURCES[3]: {
        "invocation-and-events": "SDK-Inputs-and-Events",
        "message-event-fields": "SDK-Inputs-and-Events",
        "appeventkind-why-an-app-observer-ran": "SDK-Inputs-and-Events",
        "app-data": "SDK-App-Data",
    },
    SOURCES[5]: {"": "SDK-Overview"},
    SOURCES[6]: {"": "SDK-Troubleshooting"},
    SOURCES[4]: {
        "outputs-and-host-actions": "SDK-Outputs-and-Actions",
        "panels-and-storage": "SDK-Panels-and-Storage",
    },
}


def git(directory, *arguments):
    result = subprocess.run(
        ["git", "-C", str(directory), *arguments],
        capture_output=True, text=True, encoding="utf-8", check=True,
    )
    return result.stdout.strip()


def prose_lines(text):
    """Yield line offsets outside fenced code, retaining the original text positions."""
    fence = None
    offset = 0
    for line in text.splitlines(keepends=True):
        marker = re.match(r"^ {0,3}(`{3,}|~{3,})", line)
        if marker and fence is None:
            fence = marker[1]
        elif fence is not None:
            if re.match(r"^ {0,3}" + re.escape(fence[0]) + "{" + str(len(fence)) + r",}\s*$", line):
                fence = None
        else:
            yield offset, line
        offset += len(line)


def slug(title):
    title = re.sub(r"<[^>]+>", "", title)
    title = re.sub(r"!?\[([^]]+)\]\([^)]*\)", r"\1", title)
    return re.sub(r"[^\w -]", "", title.lower()).replace(" ", "-")


def headings(text):
    used = set()
    result = []
    for offset, line in prose_lines(text):
        match = re.match(r"^ {0,3}(#{1,6}) +(.+?)(?: +#+)?\s*$", line)
        if not match:
            continue
        title = match[2]
        base = slug(title)
        anchor = base
        number = 0
        while anchor in used:
            number += 1
            anchor = f"{base}-{number}"
        used.add(anchor)
        result.append((offset, len(match[1]), title, anchor))
    return result


def section_range(text, heading, level=2):
    entries = headings(text)
    for index, (start, depth, title, _) in enumerate(entries):
        if depth == level and title == heading:
            end = next((at for at, size, _, _ in entries[index + 1:] if size <= level), len(text))
            return start, end
    raise ValueError(f"missing canonical section: {heading}")


def section(text, heading, level=2):
    start, end = section_range(text, heading, level)
    return text[start:end].strip()


def repository_links(text, source, revision, tracked, guides=None):
    guides = GUIDES if guides is None else guides

    def rewrite(match):
        target = match[2]
        url = urlsplit(target)
        if url.scheme or url.netloc:
            return match[0]
        path = posixpath.normpath(posixpath.join(posixpath.dirname(source), url.path)) if url.path else source
        if path.startswith("../") or path.startswith("/"):
            raise ValueError(f"link leaves repository in {source}: {target}")
        if path in tracked:
            kind = "blob"
        elif any(name.startswith(path.rstrip("/") + "/") for name in tracked):
            kind = "tree"
        else:
            raise ValueError(f"missing link target at {revision}: {source} -> {target}")
        suffix = ("?" + url.query if url.query else "") + ("#" + url.fragment if url.fragment else "")
        guide = guides.get(path, {})
        page = guide.get(unquote(url.fragment))
        if page and not url.query:
            # Dynamic entries carry the final page-local anchor after extraction.
            destination = page if "#" in page or not url.fragment else page + "#" + url.fragment
            return match[1] + destination + match[3]
        return match[1] + f"{GITHUB}/{kind}/{revision}/{quote(path)}{suffix}" + match[3]

    output = []
    at = 0
    for offset, line in prose_lines(text):
        output.append(text[at:offset])
        output.append(re.sub(r"(!?\[[^\]\n]*\]\()([^\s)]+)(\))", rewrite, line))
        at = offset + len(line)
    output.append(text[at:])
    return "".join(output)


RESOURCE_SECTIONS = {
    "SDK-Users-and-Relationships": ("Users and relationships", ["AppContextSnapshot", "UserSnapshot and ChannelSnapshot", "AccountProfileSnapshot", "RelationshipsSnapshot"], "Current account, user identity and relationships"),
    "SDK-Channels-and-Guilds": ("Channels and guilds", ["GuildDirectorySnapshot", "ChannelDetailsSnapshot", "ChannelMetadataSnapshot", "ChannelDirectorySnapshot", "ForumDataSnapshot"], "Joined servers, channels, permissions and forum posts"),
    "SDK-Messages": ("Messages", ["TimelineSnapshot and MessageSnapshot", "MessageContentSnapshot", "ConversationActivitySnapshot", "MessageDetailsSnapshot"], "Loaded messages, embeds, attachments, pins and typing"),
    "SDK-Members-and-Roles": ("Members and roles", ["MemberDetailsSnapshot", "MembersSnapshot", "PresenceSnapshot and PresenceEntry"], "Loaded members, role labels and known presence"),
    "SDK-Voice-and-Read-State": ("Voice and read state", ["VoiceSnapshot", "ReadSnapshot"], "Current call, unread state and mention counts"),
    "SDK-Settings": ("Settings", ["LocalSettingsSnapshot", "NotificationSettingsSnapshot"], "Reading preferences, sounds and notification options"),
}


def with_contents(text):
    entries = headings(text)
    sections = [entry for entry in entries if entry[1] > 1]
    if len(text.splitlines()) < 100 or len(sections) < 3:
        return text
    depth = min(entry[1] for entry in sections)
    contents = "\n**On this page**\n\n" + "".join(
        "  " * (level - depth) + f"- [{title}](#{anchor})\n"
        for _, level, title, anchor in sections
    ) + "\n"
    first = entries[0][0]
    end = text.find("\n", first) + 1
    return text[:end] + contents + text[end:]


def validate_links(generated):
    anchors = {
        name[:-3]: {entry[3] for entry in headings(text)} | {
            match[1] for _, line in prose_lines(text)
            for match in re.finditer(r'<a id="([^"]+)"></a>', line)
        }
        for name, text in generated.items()
    }
    for name, text in generated.items():
        for _, line in prose_lines(text):
            for match in re.finditer(r"(?<!!)\[[^\]\n]*\]\(([^\s)]+)\)", line):
                url = urlsplit(match[1])
                if url.scheme or url.netloc:
                    continue
                page = unquote(url.path) or name[:-3]
                if page.endswith(".md"):
                    page = page[:-3]
                if page not in anchors or (url.fragment and unquote(url.fragment) not in anchors[page]):
                    raise ValueError(f"broken wiki link: {name} -> {match[1]}")


def build_pages(sources, revision, status, tracked):
    notice = f"> **{status}**\n> Source: [Serein `{revision[:12]}`]({GITHUB}/tree/{revision}).\n\n"
    generated = {}
    guides = {source: {} for source in sources}
    pieces = {}

    def add(page, title, selections):
        body = "# " + title + "\n\n"
        mapped = []
        for source, heading, level in selections:
            text = sources[source]
            start, end = section_range(text, heading, level) if heading else (0, len(text))
            chunk = text[start:end].strip()
            entries = [entry for entry in headings(text) if start <= entry[0] < end]
            if entries and entries[0][2] == title:
                chunk = chunk.split("\n", 1)[1].lstrip()
                mapped.append((source, entries[0][3], slug(title)))
                entries = entries[1:]
            previous = len(headings(body))
            combined = headings(body + chunk + "\n\n")
            for entry, rendered in zip(entries, combined[previous:]):
                mapped.append((source, entry[3], rendered[3]))
            pieces.setdefault(page, []).append((source, chunk))
            body += chunk + "\n\n"
        generated[page + ".md"] = body
        for source, old, new in mapped:
            guides[source].setdefault(old, page + "#" + new)
        return body

    add("Creating-a-Plugin", "Build your first Serein plugin", [(SOURCES[0], None, 1)])
    add("Creating-a-Theme", "Theme API", [(SOURCES[2], None, 1)])
    add("SDK-Overview", headings(sources[SOURCES[5]])[0][2], [(SOURCES[5], None, 1)])
    add("SDK-Troubleshooting", headings(sources[SOURCES[6]])[0][2], [(SOURCES[6], None, 1)])
    for source, page in [(SOURCES[0], "Creating-a-Plugin"), (SOURCES[2], "Creating-a-Theme"), (SOURCES[5], "SDK-Overview"), (SOURCES[6], "SDK-Troubleshooting")]:
        guides[source][""] = page
    for page, source, title in [
        ("SDK-Inputs-and-Events", SOURCES[3], "Invocation and events"),
        ("SDK-Outputs-and-Actions", SOURCES[4], "Outputs and host actions"),
        ("SDK-Panels-and-Storage", SOURCES[4], "Panels and storage"),
    ]:
        add(page, title, [(source, title, 2)])
    app = section(sources[SOURCES[3]], "App data")
    app_titles = [entry[2] for entry in headings(app) if entry[1] == 3]
    moved = {}
    for page, (title, prefixes, _) in RESOURCE_SECTIONS.items():
        selected = []
        for prefix in prefixes:
            matches = [heading for heading in app_titles if heading.startswith(prefix + ":")]
            if len(matches) != 1:
                raise ValueError(f"missing or ambiguous resource section: {prefix}")
            selected.append((SOURCES[3], matches[0], 3))
            moved[matches[0]] = page
        add(page, title, selected)
    remaining = [title for title in app_titles if title not in moved]
    add("SDK-App-Data", "App data", [(SOURCES[3], title, 3) for title in remaining])
    guides[SOURCES[3]]["app-data"] = "SDK-App-Data#app-data"
    intro = app.split("\n", 1)[1].split("### ", 1)[0].strip()
    resources = "| Resource | Fields |\n| --- | --- |\n" + "".join(
        f"| [{title}]({page}) | {description} |\n"
        for page, (title, _, description) in RESOURCE_SECTIONS.items()
    )
    generated["SDK-App-Data.md"] = generated["SDK-App-Data.md"].replace("# App data\n\n", "# App data\n\n" + intro + "\n\n" + resources + "\n", 1)
    pieces["SDK-App-Data"].append((SOURCES[3], intro))
    # Explicit anchors preserve deep links even when duplicate headings move to other pages.
    generated["SDK-App-Data.md"] += "## Legacy object links\n\n"
    for title, page in moved.items():
        start, end = section_range(sources[SOURCES[3]], title, 3)
        for at, _, heading, anchor in headings(sources[SOURCES[3]]):
            if start <= at < end:
                generated["SDK-App-Data.md"] += f'<a id="{anchor}"></a>\n- [{heading}]({guides[SOURCES[3]][anchor]})\n\n'
    # The published wiki used this name before scrolling preferences were added.
    reading = next(guides[SOURCES[3]][slug(title)] for title in moved if title.startswith("LocalSettingsSnapshot:"))
    generated["SDK-App-Data.md"] += f'<a id="localsettingssnapshot-five-reading-preferences"></a>\n\nSee [Reading preferences]({reading}).\n'
    add("API-and-Security-Reference", "API and security reference", [(SOURCES[1], "Capability reference", 3), (SOURCES[0], "ABI version 1", 2), (SOURCES[1], "Resource and privacy limits", 2)])
    add("Testing-and-Packaging", "Test and package an extension", [(SOURCES[0], "Build and package", 2), (SOURCES[0], "Test and develop locally", 2), (SOURCES[1], "Install and remove", 2)])
    add("Publishing-to-the-Community-Catalog", "Publish to the community catalog", [(SOURCES[1], "Creator workflow", 2), (SOURCES[1], "Shop previews", 2)])
    # Rewrite each extracted fragment with its original source context, including # links.
    for page, chunks in pieces.items():
        for source, chunk in chunks:
            generated[page + ".md"] = generated[page + ".md"].replace(chunk, repository_links(chunk, source, revision, tracked, guides), 1)
    start = "- [SDK overview](SDK-Overview)\n- [First plugin](Creating-a-Plugin)\n- [Create a theme](Creating-a-Theme)\n"
    resources = "".join(f"- [{title}]({page})\n" for page, (title, _, _) in RESOURCE_SECTIONS.items())
    reference = "- [App data index](SDK-App-Data)\n- [Inputs and events](SDK-Inputs-and-Events)\n- [Outputs and actions](SDK-Outputs-and-Actions)\n- [Panels and storage](SDK-Panels-and-Storage)\n- [Capabilities, ABI and limits](API-and-Security-Reference)\n"
    help_links = "- [Troubleshooting](SDK-Troubleshooting)\n- [Test and package](Testing-and-Packaging)\n- [Publish to the catalog](Publishing-to-the-Community-Catalog)\n"
    generated["Home.md"] = "# Serein extension SDK\n\nBuild local Wasm plugins and native themes. Start with a working example, then find the resource or action you need.\n\n## Start\n\n" + start + "\n## Choose a task\n\n| Task | Read |\n| --- | --- |\n| Understand how plugins run | [SDK overview](SDK-Overview) |\n| Identify users and relationships | [Users and relationships](SDK-Users-and-Relationships) |\n| Inspect servers, channels or forum threads | [Channels and guilds](SDK-Channels-and-Guilds) |\n| Read loaded messages, attachments, pins or typing | [Messages](SDK-Messages) |\n| Inspect members, roles or presence | [Members and roles](SDK-Members-and-Roles) |\n| Read current call or unread state | [Voice and read state](SDK-Voice-and-Read-State) |\n| Read or propose local preferences | [Settings](SDK-Settings) |\n| Navigate, copy text or propose an action | [Outputs and actions](SDK-Outputs-and-Actions) |\n| Build forms and save plugin state | [Panels and storage](SDK-Panels-and-Storage) |\n| Diagnose an error | [Troubleshooting](SDK-Troubleshooting) |\n\n## Reference\n\n" + reference + "\n## Help\n\n" + help_links + f"\n[SDK examples and source]({GITHUB}/tree/{revision}/examples/extensions)\n\nPlugins cannot call Discord, send messages, access credentials, open files or use the network. Each capability needs explicit user consent.\n"
    generated["_Sidebar.md"] = "- [Home](Home)\n\n## Start\n\n" + start + "\n## Resources\n\n" + resources + "\n## Reference\n\n" + reference + "\n## Help\n\n" + help_links + f"\n[Serein source]({GITHUB}/tree/{revision})\n"
    generated = {name: (notice + (text if name == "SDK-App-Data.md" else with_contents(text)) if name != "_Sidebar.md" else text) for name, text in generated.items()}
    generated = {name: text.rstrip() + "\n" for name, text in generated.items()}
    validate_links(generated)
    return generated


def pages(revision, status):
    tracked = set(git(REPOSITORY, "ls-tree", "-r", "--name-only", revision).splitlines())
    sources = {source: git(REPOSITORY, "show", f"{revision}:{source}") for source in SOURCES}
    return build_pages(sources, revision, status, tracked)


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
