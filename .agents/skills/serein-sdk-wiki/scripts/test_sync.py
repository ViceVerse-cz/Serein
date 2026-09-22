"""Offline checks for extracted resources, navigation and immutable source links."""
from sync import (
    GITHUB, RESOURCE_SECTIONS, SOURCES, build_pages, headings,
    repository_links, section, validate_links, with_contents,
)


def rejected(function, *args):
    try:
        function(*args)
    except ValueError:
        return
    raise AssertionError("invalid document or link was accepted")


def main():
    revision = "a" * 40
    tracked = {"docs/index.md", "docs/guide.md", "docs/image.png", "examples/demo/Cargo.toml"}
    text = (
        "[guide](guide.md#limits) ![preview](image.png) [example](../examples/demo)\n"
        "[external](https://example.org/guide) [section](#local)\n"
        "```markdown\n[code](missing.md)\n```"
    )
    rewritten = repository_links(text, "docs/index.md", revision, tracked)
    assert f"{GITHUB}/blob/{revision}/docs/guide.md#limits" in rewritten
    assert f"{GITHUB}/blob/{revision}/docs/image.png" in rewritten
    assert f"{GITHUB}/tree/{revision}/examples/demo" in rewritten
    assert "[external](https://example.org/guide)" in rewritten
    assert f"[section]({GITHUB}/blob/{revision}/docs/index.md#local)" in rewritten
    assert "```markdown\n[code](missing.md)\n```" in rewritten
    for invalid in ("[missing](missing.md)", "[outside](../../outside.md)"):
        rejected(repository_links, invalid, "docs/index.md", revision, tracked)
    document = "# Guide\n\n## First\nfirst\n```md\n## Fake\n```\n\n## Second\nsecond\n"
    assert section(document, "First") == "## First\nfirst\n```md\n## Fake\n```"
    assert section(document, "Second") == "## Second\nsecond"
    rejected(section, document, "Fake")
    rejected(section, document, "Missing")
    nested = "## Parent\n### Fields\nfields\n#### Example\nexample\n### Next\nnext\n## End\n"
    assert section(nested, "Fields", level=3) == "### Fields\nfields\n#### Example\nexample"
    duplicate = "# Title\n## Repeat\n## Repeat\n## Repeat-1\n~~~md\n## Hidden\n~~~\n" + "body\n" * 100
    assert [entry[3] for entry in headings(duplicate)] == ["title", "repeat", "repeat-1", "repeat-1-1"]
    contents = with_contents(duplicate)
    assert "[Repeat](#repeat-1)" in contents and "[Repeat-1](#repeat-1-1)" in contents
    assert "[Hidden]" not in contents
    validate_links({"Page.md": contents})
    rejected(validate_links, {"Page.md": "# Page\n[broken](Missing)"})
    rejected(validate_links, {"Page.md": "# Page\n[broken](#missing)"})
    validate_links({"Page.md": '# Page\n<a id="old-anchor"></a>\n[old](#old-anchor)'})

    sources = {
        SOURCES[0]: "# Build your first Serein plugin\n\n## Build and package\nBuild.\n## Test and develop locally\nTest.\n## ABI version 1\nABI.\n",
        SOURCES[1]: "# Extensions\n## Creator workflow\nShare.\n## Shop previews\nPreview.\n## Install and remove\nInstall.\n## Host contract\n### Capability reference\nConsent.\n## Resource and privacy limits\nLimits.\n",
        SOURCES[2]: "# Theme API\nThemes.\n",
        SOURCES[3]: "# SDK inputs\n## Invocation and events\n### Events\nEvents.\n## App data\nRead [messages](#timelinesnapshot-and-messagesnapshot-fixture).\n### IDs, absence and partial data\nIDs.\n### AppSnapshot: choose the group you need\nGroups.\n",
        SOURCES[4]: "# Actions\n## Outputs and host actions\n### Apply\nApprove.\n## Panels and storage\n### Fields\nControls.\n",
        SOURCES[5]: "# SDK overview\n[users](extension-sdk-reference.md#appcontextsnapshot-fixture)\n",
        SOURCES[6]: "# SDK troubleshooting\n[unknown](extension-sdk-reference.md#future)\n",
    }
    for _, (_, prefixes, _) in RESOURCE_SECTIONS.items():
        for prefix in prefixes:
            sources[SOURCES[3]] += f"### {prefix}: fixture\n{prefix} body.\n"
            if prefix == "AppContextSnapshot":
                sources[SOURCES[3]] += "#### Example\n[settings](#notificationsettingssnapshot-fixture)\n"
            if prefix == "NotificationSettingsSnapshot":
                sources[SOURCES[3]] += "#### Example\n[user example](#example)\n"
    sources[SOURCES[3]] += "### Example: read a snapshot without confusing unknown with zero\nRead example.\n"
    generated = build_pages(sources, revision, "Preview SDK", set(sources))
    assert '<a id="localsettingssnapshot-five-reading-preferences"></a>' in generated["SDK-App-Data.md"]
    assert len(generated) == 19
    assert "AppContextSnapshot body." in generated["SDK-Users-and-Relationships.md"]
    assert "AppContextSnapshot body." not in generated["SDK-App-Data.md"]
    assert "Read example." in generated["SDK-App-Data.md"]
    assert '[settings](SDK-Settings#notificationsettingssnapshot-fixture)' in generated["SDK-Users-and-Relationships.md"]
    assert '[user example](SDK-Users-and-Relationships#example)' in generated["SDK-Settings.md"]
    assert '<a id="example-1"></a>' in generated["SDK-App-Data.md"]
    assert '[Example](SDK-Settings#example)' in generated["SDK-App-Data.md"]
    assert '[messages](SDK-Messages#timelinesnapshot-and-messagesnapshot-fixture)' in generated["SDK-App-Data.md"]
    assert '[users](SDK-Users-and-Relationships#appcontextsnapshot-fixture)' in generated["SDK-Overview.md"]
    assert f'{GITHUB}/blob/{revision}/{SOURCES[3]}#future' in generated["SDK-Troubleshooting.md"]
    assert "## Resources" in generated["_Sidebar.md"] and "## Help" in generated["Home.md"]
    assert all(revision in text for text in generated.values())
    validate_links(generated)
    print("Wiki resource, anchor, contents and source-link checks passed.")


if __name__ == "__main__":
    main()
