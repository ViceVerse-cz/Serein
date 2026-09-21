"""Offline checks for source-link rewriting and section extraction."""
from sync import GITHUB, repository_links, section


def main():
    revision = "a" * 40
    tracked = {"docs/guide.md", "docs/image.png", "examples/demo/Cargo.toml"}
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
    try:
        section(document, "Missing")
    except ValueError:
        pass
    else:
        raise AssertionError("missing canonical section was accepted")
    print("Wiki source-link and section checks passed.")


if __name__ == "__main__":
    main()
