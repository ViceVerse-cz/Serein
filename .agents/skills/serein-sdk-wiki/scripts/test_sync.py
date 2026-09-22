"""Offline checks for source-link rewriting and section extraction."""
from sync import GITHUB, repository_links, section


def main():
    revision = "a" * 40
    tracked = {
        "docs/guide.md", "docs/image.png", "examples/demo/Cargo.toml",
        "docs/extension-sdk-reference.md", "docs/extension-sdk-actions.md",
        "examples/extensions/README.md",
    }
    source = "docs/index.md"
    text = (
        "[guide](guide.md#limits) ![preview](image.png) [example](../examples/demo)\n"
        "[external](https://example.org/guide) [section](#local)\n"
        "```markdown\n[code](missing.md)\n```"
    )
    rewritten = repository_links(text, source, revision, tracked)
    assert f"{GITHUB}/blob/{revision}/docs/guide.md#limits" in rewritten
    assert f"{GITHUB}/blob/{revision}/docs/image.png" in rewritten
    assert f"{GITHUB}/tree/{revision}/examples/demo" in rewritten
    assert "[external](https://example.org/guide) [section](#local)" in rewritten
    assert "```markdown\n[code](missing.md)\n```" in rewritten
    references = repository_links(
        "[events](extension-sdk-reference.md#message-event-fields) "
        "[data](extension-sdk-reference.md#app-data) "
        "[forms](extension-sdk-actions.md#panels-and-storage) "
        "[tutorial](../examples/extensions/README.md#write-the-handler) "
        "[source](extension-sdk-reference.md)",
        source, revision, tracked,
    )
    assert "[events](SDK-Inputs-and-Events#message-event-fields)" in references
    assert "[data](SDK-App-Data#app-data)" in references
    assert "[forms](SDK-Panels-and-Storage#panels-and-storage)" in references
    assert "[tutorial](Creating-a-Plugin#write-the-handler)" in references
    assert f"[source]({GITHUB}/blob/{revision}/docs/extension-sdk-reference.md)" in references
    for invalid in ("[missing](missing.md)", "[outside](../../outside.md)"):
        try:
            repository_links(invalid, source, revision, tracked)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid source link was accepted")
    document = "# Guide\n\n## First\nfirst\n\n## Second\nsecond\n"
    assert section(document, "First") == "## First\nfirst"
    assert section(document, "Second") == "## Second\nsecond"
    nested = "## Parent\n### Fields\nfields\n#### Example\nexample\n### Next\nnext\n## End\n"
    assert section(nested, "Fields", level=3) == "### Fields\nfields\n#### Example\nexample"
    assert section(nested, "Next", level=3) == "### Next\nnext"
    try:
        section(document, "Missing")
    except ValueError:
        pass
    else:
        raise AssertionError("missing canonical section was accepted")
    print("Wiki source-link and section checks passed.")


if __name__ == "__main__":
    main()
