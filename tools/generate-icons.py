#!/usr/bin/env python3
"""Rebuild the bundled Phosphor icon atlas (development only).

Downloads the version-pinned SVG sources, verifies every file hash, composes one SVG grid and
rasterizes it with the `resvg` command-line tool. Requires `resvg` 0.45.1 on PATH or `--resvg`:

    cargo install resvg --version 0.45.1 --root /tmp/resvg-tool
    python3 tools/generate-icons.py --resvg /tmp/resvg-tool/bin/resvg
"""

import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request

VERSION = "2.1.1"  # @phosphor-icons/core on npm, MIT
BASE = f"https://cdn.jsdelivr.net/npm/@phosphor-icons/core@{VERSION}"
COLUMNS, CELL, PAD = 8, 64, 4
GLYPH = CELL - 2 * PAD

# Repository-local artwork, rasterized from the repository instead of an upstream package.
# `assets/icons/thread.svg` is the thread glyph; it is listed inline below.
BRAND_MARK = "repo:assets/brand/serein-mark.svg"

# name, upstream asset, SHA-256 of the unmodified SVG file.
ICONS = [
    ("caret-down", "bold/caret-down-bold.svg", "76a97545e1b923bc13bcc15d7bcbb7f5530105e6eaa98a18c1e30d23e3622843"),
    ("caret-right", "bold/caret-right-bold.svg", "03cadd956d715541432ec8dc2eda1c53ca341af7d3ccecb8dd32c5b9747e290f"),
    ("gear", "fill/gear-six-fill.svg", "dc303bf88571d9aa8d56c30d2630303aa7aba4ca21c21010be3549d6c17666ce"),
    ("microphone", "fill/microphone-fill.svg", "cd8446012357f05a70679ecc792402aa6e49f1fdc820b04035a97b213a2f17a6"),
    ("microphone-slash", "fill/microphone-slash-fill.svg", "6970ccb81d74ffe2165904bc4ed09e0dc738c5d4930192426afd24a5457a2202"),
    ("headphones", "fill/headphones-fill.svg", "35db98ea0b59f4c8c81902d066a8f6db58680f99e48d1a36e6e167e3ef1a6751"),
    ("headphones-slash", "fill/headphones-fill.svg", "35db98ea0b59f4c8c81902d066a8f6db58680f99e48d1a36e6e167e3ef1a6751"),
    ("push-pin", "fill/push-pin-fill.svg", "b3a7a363401c6e09ccd0fc5a7f6813c3183dab69b07eaca3419acf2fafe942b3"),
    ("users", "fill/users-fill.svg", "6104c0634ed5b574e261acc05a3575db8abfb4ccbcb3bf12f5359a431303783b"),
    ("user-plus", "fill/user-plus-fill.svg", "07fe0181040b8dd7956f8067f13f83d81ec395d54b1760392a5daf5692232652"),
    ("user-circle", "fill/user-circle-fill.svg", "4e16cdba116195993e6398bb097e7c4b354fb129fafdf2b56fd4b7c069f440d3"),
    ("magnifying-glass", "bold/magnifying-glass-bold.svg", "b73e393b20bff0aaee96b9e325d0276fe1ab5fc81b5080633a95827bd14ebae6"),
    ("plus", "bold/plus-bold.svg", "3d20a4b2e00657baeb922bed94f13fbfa288968b738991d047dd252cb64005d8"),
    ("plus-circle", "fill/plus-circle-fill.svg", "7e6b31bae705c568cb7ce2b42fca9d65f85da725bac30f207ee2ce6368abb6c5"),
    ("x", "bold/x-bold.svg", "d540487912a267d83c495954b24ca07981002fda05ee2ea0b492d8fc188d1c3e"),
    ("smiley", "fill/smiley-fill.svg", "2f993043ef4566931f5ad81be6d1030734d2dbad5e5d7c5d380debfeaeeb9399"),
    ("bell", "fill/bell-fill.svg", "26437565edc7418a2c326a74078fb4519aa2316e84fcf1adefbdd1b2c89f4ac5"),
    ("phone", "fill/phone-fill.svg", "4546c9d7d0a26f08ec5a35f2ca3e12d8ef76767ba60c2effa6f214d7f070dcb6"),
    ("phone-call", "fill/phone-call-fill.svg", "6d67e611de99d8a33ba45f5d2a9e56d4123dcb9395020998a3b7250d3a08e36a"),
    ("phone-disconnect", "fill/phone-disconnect-fill.svg", "15996d65a74424921e84c8e40e94fb43da650183476abb00b3d1498d2400ffc4"),
    ("video-camera", "fill/video-camera-fill.svg", "daf72fb4e3f7978254c2ac1959db6d855f12bb0d9b7504309d2e6270b7192933"),
    ("video-camera-slash", "fill/video-camera-slash-fill.svg", "1cb4bf1be6de46f024be49c324641afe36cfa5807f4846691851d8c954a5d0e4"),
    ("monitor-arrow-up", "fill/monitor-arrow-up-fill.svg", "9519c0c4c734553167b598b03d493cdb6ef14f8df8c7283a1847699bcc33a0a8"),
    ("rocket-launch", "fill/rocket-launch-fill.svg", "fed02fcf57da975870e6898085b8e469a695dafac38ddae2d92b169678c661a9"),
    ("waveform", "fill/waveform-fill.svg", "fbbca82dbcc988184e314a671ac08517b4099540ab97605ae186998d9c0d3187"),
    ("arrow-bend-up-left", "bold/arrow-bend-up-left-bold.svg", "b59c6a1dea610f669a69920bece2138108a54d88eaa9a1d0a20f82e1657c9f25"),
    ("pencil-simple", "fill/pencil-simple-fill.svg", "b78e7b71cb19d43e235983a883d50ab78cf9227f9864106e1aa4f366ee317aa8"),
    ("dots-three", "bold/dots-three-bold.svg", "5bd06c9754d075b409e3259350c9abc9ec106ffb3b6f3cd451f6419271163bc1"),
    ("tray", "fill/tray-fill.svg", "8e0ed750696c01ef6e418436275501e4467c66b6f88688e1d0ac86e84a296b47"),
    ("question", "fill/question-fill.svg", "91390e2c8d076dedcf73ec4ffc5ac6531dbf93aadac7e0907ba1cfb9e95a145b"),
    ("arrow-clockwise", "bold/arrow-clockwise-bold.svg", "773d814cdf564a4d0db6e0408a60e5bd97c76c90293aaf383c72399ce2e034a1"),
    ("chats", "fill/chats-fill.svg", "c88539cf3c42ef3a42f90d10e9c4736cbad59e2639948dbe647227e56160447b"),
    ("speaker-high", "fill/speaker-high-fill.svg", "c266c407915382db3866570f816934b804c1b84bd47d971fad0e95ac5107dd6b"),
    ("hash", "bold/hash-bold.svg", "8d2cb5e39903004da3d0a85a919adc6c76adcd65b3c9c1e8e355d8cf6cdf7193"),
    ("chat-centered-text", "fill/chat-centered-text-fill.svg", "cc2c02e50a62aa98aae8634d518b2cc311413765ddfddae5908bf9168113d0a7"),
    ("paper-plane-right", "fill/paper-plane-right-fill.svg", "a8e6f3a92755f1bc79bcfd691e794709bec6b13d68b948adc1eb035a2ddd8fff"),
    ("arrow-square-out", "bold/arrow-square-out-bold.svg", "68348bacc7f539b8d73de92d7c120f22e424b4364374ccab01719ada8c8a12aa"),
    ("github-logo", "fill/github-logo-fill.svg", "35b2036490005bcb95a68d07d43d441ae585c6c6dde0e902bb3daf0ddbc615b9"),
    ("twitch-logo", "fill/twitch-logo-fill.svg", "df5adaa427994af42c961715d6e75f496b1832606a68cbd8090d09c25a4e8361"),
    ("steam-logo", "fill/steam-logo-fill.svg", "1715fdfe47e32c6cbff76fd80dcb47c1e5a704b2067412dd64dff20b8d802a11"),
    ("spotify-logo", "fill/spotify-logo-fill.svg", "50360bee2998f48367780b6f1637d208169acb0d3384e27e145f29b456b73bd4"),
    ("youtube-logo", "fill/youtube-logo-fill.svg", "fce097e65d0df0cb2bb84cd282bb6cc08ccb47ec18d41b7c22870830257065be"),
    ("x-logo", "fill/x-logo-fill.svg", "b760dcebe932bafc69784799231eb8ce988dc33fa5d68d20e706508b24264526"),
    ("reddit-logo", "fill/reddit-logo-fill.svg", "f6e1c29f724ad06cf8ab8c41e26a72a482e6187724ac7cc91fe880b9c499873d"),
    ("facebook-logo", "fill/facebook-logo-fill.svg", "4ef00877d0348c4db3eb93d01d36d9b82528ca2d635d0e3a5e51515299cf99f7"),
    ("instagram-logo", "fill/instagram-logo-fill.svg", "24444c671b0c50580b39138453762225d4d53b7d04c978244b8027a84c719746"),
    ("tiktok-logo", "fill/tiktok-logo-fill.svg", "aafb59c3c1902035736bc955e1b4485e46fd8b633bb2cd2e046937cc1b26c5cb"),
    ("paypal-logo", "fill/paypal-logo-fill.svg", "b49cb17698ca5c2d807577413f992b9d0050b2a94ca4c812c6e6441a32f9f820"),
    ("amazon-logo", "fill/amazon-logo-fill.svg", "96d475f1eaed1733e9c6dcae74b52900be7e399b128917a8f50d89d773041491"),
    ("mastodon-logo", "fill/mastodon-logo-fill.svg", "f9d1bf489b16a34763aa7f8e8de62f72812cb9888fd92bbc47eeac33d1426122"),
    ("skype-logo", "fill/skype-logo-fill.svg", "bd264055d7317fb0cfe3a39ae17028d2a5447dc3a204a999c5e5fbebed423e28"),
    ("game-controller", "fill/game-controller-fill.svg", "9dcb7af7b4854bb2da15ab1852292428c4d95314b06cd25aa5f45f21b3134884"),
    ("television", "fill/television-fill.svg", "47da225e96c028e8d8fb02db79798082f8f2dfaad22dd9c2ad576f982f4a0de4"),
    ("globe", "bold/globe-bold.svg", "b1dff2302e56cfc39cb704bb109486ee53d29a9a65f5a02f50e258259fbf743e"),
    ("link", "bold/link-bold.svg", "087d63d78d4a37d570853cb62e688323759cda2081b22455674544f87adc3444"),
    ("copy", "bold/copy-bold.svg", "204e84365593c418c71d9ce0674be3dd3e5ff13d0dc13e1f728249ef9571bf9e"),
    ("seal-check", "fill/seal-check-fill.svg", "27702a62622ac4156d2a18b10fc7c526b3b2b9f782741e2cb07aea6964d71132"),
    ("calendar-blank", "fill/calendar-blank-fill.svg", "405596825bf3705baa7d289ed1242106fded80e34967927549a0413f4303a0ac"),
    ("serein-mark", BRAND_MARK, "131359276c0f4abe7f97f99f0ad73cad88fec4d9845ad01bcdece940a089dc3e"),
    ("file", "fill/file-fill.svg", "d6fe00691e45b5e9b87ccc5a8fc9022485935408167ab1234817915df4fd1ca4"),
    ("file-image", "fill/file-image-fill.svg", "0266eb983ed5cec9152d76691152123b92b2cb84d8c139c37793d0870df3f601"),
    ("file-pdf", "fill/file-pdf-fill.svg", "12622b293b9a1efa1f56969ca74ab88085d388c4be926f81cfcb49406a8da05d"),
    ("file-zip", "fill/file-zip-fill.svg", "3b4355954bef19b2572639dcb619e0e81513118dd76c3c2fc8bab6d9a23c9032"),
    ("file-text", "fill/file-text-fill.svg", "8c6f36452a441d39886ad7929778c2a90ea66cca519490da4918e7da7bade242"),
    ("file-code", "fill/file-code-fill.svg", "127044e7395681d5306c7553cc2287e16c63f9d4578f4f6fe7710f277e5b7624"),
    ("file-audio", "fill/file-audio-fill.svg", "835c6bfde42e68d716db3dd4db0e02843f6a7bd704a4377287a4b022d80ace7a"),
    ("file-video", "fill/file-video-fill.svg", "2940d83bf3560f389d6a826ca639741a9098d5bfe8910ea371fdc4c83790b827"),
    ("trash", "fill/trash-fill.svg", "f78767cc15e1a7d6eea49c4efb515cf6fceaf07fbc421e8ce18373d07c14b673"),
    ("arrow-down", "bold/arrow-down-bold.svg", "193222b87a796f1c56336f51cb041cfd2a6c899ae0a027248a63fc183758a364"),
    ("arrow-up", "bold/arrow-up-bold.svg", "87dade5b87b48190dae13370375e2d84f6c22548eebac93ff2f46d091882fb78"),
    ("check", "bold/check-bold.svg", "d0ca4e324ff5bb3a1a3bacb9f7580359b8e03cc6862a614d5ed14458db64bedf"),
    ("gif", "bold/gif-bold.svg", "cbdd66cdfbe9f084d2ecc7e7a27686830b5230cc0a18f9765fb9c147ac8e0647"),
    ("star", "bold/star-bold.svg", "e456b195ce0f28235d63c0612b835fd120c747a8132d2be984863e2125674240"),
    ("star-fill", "fill/star-fill.svg", "42451b34121b695bdaab88fdcf3eacec9cda9ac36d222349609426dcb9f04b48"),
    ("fire", "fill/fire-fill.svg", "512355fb2a156f0c39485a28bc6f81cf1df3de35faed9c5ad62c30cec5d8d63c"),
    ("arrow-left", "bold/arrow-left-bold.svg", "7588792d7824e7c5337bca7b5de96ad0685ddea0a5732dfb62701d163a076563"),
    ("folder", "fill/folder-fill.svg", "2217bd2f730884d7a8aec3e3358eb7500448b0e3d86f33a63142f40f0622bfe9"),
    ("folder-open", "fill/folder-open-fill.svg", "f9e57c3d40ab536915f8302a4390405f5660ee7b7275a3f80b384d977c59373e"),
    ("caret-left", "bold/caret-left-bold.svg", "b78b6f532b53b9847340961848cb9e4f5be7e1da07e9dcf11c60cf64aa3986b7"),
    ("download-simple", "bold/download-simple-bold.svg", "3ca0b8bba15633eb29f9818fe3670d6438bcbc897649c767e491a411e696e2d3"),
    ("arrow-right", "bold/arrow-right-bold.svg", "aa0f883acecf63d9140ad7cd51910fa4b723671c400772a3c621c9cc16b1687b"),
    ("image", "fill/image-fill.svg", "4f4dbea3adfc054509fed7f8e0f7d50fbf716904e37fc55387d29018d15b52fd"),
    ("sparkle", "fill/sparkle-fill.svg", "9872a9255717f4ecc0f01b028b9436042e0f49391e4ce7a57f61aa466684fad1"),
    ("compass", "fill/compass-fill.svg", "9663ec833384ba767b617ba3c256a3e00a23bd666810e2f2214171896498b6dc"),
    ("megaphone-simple", "fill/megaphone-simple-fill.svg", "cead45ff02f94b5bd1aa2e59268b884482b43cc54f7b316d69e7030a2bd7196e"),
    ("shield-warning", "fill/shield-warning-fill.svg", "8c594562a5adfd9fcce1f76514b62e8c626c4c00d742a69aa51214bae0b4037f"),
    ("crown", "fill/crown-fill.svg", "a285272ce54401826e1458e5437aaa5dd78b5bb92b75092093f091eb024b0a45"),
    ("chart-bar", "fill/chart-bar-fill.svg", "a417cc075c304374043e7d47f7622606b15f14e4c65663fd7a8a9898d768362a"),
    ("shopping-cart-simple", "fill/shopping-cart-simple-fill.svg", "679456c02afec984a2ab635443724baca8b891a1bc4f245469f9bcbbf6a13fdb"),
    ("lock-simple", "fill/lock-simple-fill.svg", "54b137caf94b8082e63a2831e5b726bbc9cd32ad40938bf5656ff4d44b442d95"),
    ("eye-slash", "fill/eye-slash-fill.svg", "fc34ad807da63ae5f99a235618cddc1b2bef8d98d7c95004d38b61312540b90d"),
    ("sliders-horizontal", "bold/sliders-horizontal-bold.svg", "e409a5fb3c2c134e46d51e48ac395e392223e04535c5ec664253f3e7e78cd7a9"),
    ("arrows-down-up", "bold/arrows-down-up-bold.svg", "174464c54af7273e46a6fc204ebd0fc1da75906573880687836b314e3fbdb85e"),
    ("thread", "repo:assets/icons/thread.svg", "dfd7daf80375504a5af37b95bee1773af55a9eabe7c802e7f4152905f123c72c"),
    ("device-mobile", "fill/device-mobile-fill.svg", "cec802e8cbe698b5bdf604770c770e4431f24ad879d441937fc6120b8b29ff28"),
]
LICENSE_SHA256 = "ddbe6082ec3cf979db47e5af549d2849c5d6182b3e005ef91ce1dbb9eb122f11"

# Brand marks Phosphor does not ship come from Simple Icons (CC0 1.0). Their 24-unit glyphs
# fill the whole view box, so they are drawn at Phosphor's visual size inside the cell.
SIMPLE_VERSION = "16.30.0"  # simple-icons on npm, CC0-1.0
SIMPLE_BASE = f"https://cdn.jsdelivr.net/npm/simple-icons@{SIMPLE_VERSION}"
SIMPLE_ICONS = [
    ("playstation", "icons/playstation.svg", "b68b4d7b63443759b9c4d77a5501c5758eeae06a6d594d278d5eb6cb4d3dbbe4"),
    ("battle-net", "icons/battledotnet.svg", "78206c9c5e7fd24803cd50cd5c71c3bb1b02def42ca4f5d186e318eb9e247b7e"),
    ("epic-games", "icons/epicgames.svg", "a19b1eb5a46edc11a7dc7f1ce6fa1701ea4e4cf451feec88441c523f4b50cde3"),
    ("league-of-legends", "icons/leagueoflegends.svg", "b653d9c5733c71613fa0367c09c9d419df2f0e7ef91cbeab6b93a2e20d5f223b"),
    ("riot-games", "icons/riotgames.svg", "b80c5880b88b8e489b2da7753bed612b3ce6b2948a3f995ad5d31050f443b520"),
    ("bungie", "icons/bungie.svg", "92c3b473805eaffdc7092cb8e8c1500ccac1bea19c129d46b026c22af6000db7"),
    ("roblox", "icons/roblox.svg", "9245b2f23fde91a5ef34f36aea32c1bf08a1a1254c08b86e7f060a4f72986234"),
    ("crunchyroll", "icons/crunchyroll.svg", "3b9c3d87339e18ec09f25e0c3eaffdc5ad4630df3d108fcd907e1b64c4cd13ea"),
    ("ebay", "icons/ebay.svg", "846e8d8ac6cea49766e7c62e095e739d115ed355f9b63bf46067f14c9343c745"),
    ("bluesky", "icons/bluesky.svg", "49752973164fbbf4464fbb4776f011c1eff207e5d7ad9254e031af025814eb75"),
]
SIMPLE_LICENSE_SHA256 = "9046848b63a5c92bff14e4accca80bd987e0623b74adf9226ce5198d312b79d5"
SIMPLE_SCALE = 0.8  # Phosphor fill glyphs span roughly 205 of their 256 units.


def fetch(path, base=BASE):
    with urllib.request.urlopen(f"{base}/{path}", timeout=60) as response:
        return response.read(1024 * 1024)


def inner_svg(svg):
    start = svg.index(">", svg.index("<svg")) + 1
    end = svg.rindex("</svg>")
    return svg[start:end]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resvg", default="resvg", help="path to the resvg executable")
    parser.add_argument("--print-hashes", action="store_true", help="print SHA-256 values instead of checking")
    args = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / "assets" / "icons"
    destination.mkdir(parents=True, exist_ok=True)

    license_text = fetch("LICENSE")
    if args.print_hashes:
        print("LICENSE", hashlib.sha256(license_text).hexdigest())
    elif hashlib.sha256(license_text).hexdigest() != LICENSE_SHA256:
        raise ValueError("LICENSE SHA-256 mismatch")

    simple_license = fetch("LICENSE.md", SIMPLE_BASE)
    if args.print_hashes:
        print("SIMPLE LICENSE", hashlib.sha256(simple_license).hexdigest())
    elif hashlib.sha256(simple_license).hexdigest() != SIMPLE_LICENSE_SHA256:
        raise ValueError("Simple Icons LICENSE SHA-256 mismatch")

    sources = [(name, asset, sha256, False) for name, asset, sha256 in ICONS]
    sources += [(name, asset, sha256, True) for name, asset, sha256 in SIMPLE_ICONS]
    names = [name for name, _, _, _ in sources]
    assert len(set(names)) == len(names)
    rows = (len(sources) + COLUMNS - 1) // COLUMNS
    width, height = COLUMNS * CELL, rows * CELL
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">']
    parts.append(
        '<defs><mask id="slash" maskUnits="userSpaceOnUse" x="0" y="0" width="256" height="256">'
        '<rect width="256" height="256" fill="#fff"/>'
        '<path d="M40 24 L232 216" stroke="#000" stroke-width="44" stroke-linecap="round"/></mask></defs>'
    )
    index = []
    root = Path(__file__).resolve().parents[1]
    for cell, (name, asset, sha256, simple) in enumerate(sources):
        local = asset.startswith("repo:")
        if local:
            svg = (root / asset.removeprefix("repo:")).read_bytes()
        else:
            svg = fetch(asset, SIMPLE_BASE) if simple else fetch(f"assets/{asset}")
        digest = hashlib.sha256(svg).hexdigest()
        if args.print_hashes:
            print(name, asset, digest)
        elif digest != sha256:
            raise ValueError(f"{asset} SHA-256 mismatch: {digest}")
        text = svg.decode("utf-8")
        body = inner_svg(text)
        x = (cell % COLUMNS) * CELL + PAD
        y = (cell // COLUMNS) * CELL + PAD
        if local:
            # Our own mark is trimmed to its bounding box; honour its view box offset.
            box = text.split('viewBox="', 1)[1].split('"', 1)[0].split()
            left, top, size = float(box[0]), float(box[1]), float(box[2])
            scale = GLYPH / size
            x, y = x - left * scale, y - top * scale
        elif simple:
            assert 'viewBox="0 0 24 24"' in text, asset
            inset = GLYPH * (1 - SIMPLE_SCALE) / 2
            x, y, scale = x + inset, y + inset, GLYPH * SIMPLE_SCALE / 24
        else:
            assert 'viewBox="0 0 256 256"' in text, asset
            scale = GLYPH / 256
        if name.endswith("-slash") and "slash" not in asset:
            # Phosphor has no slashed headphones; compose the upstream glyph with a knocked-out
            # diagonal in the style of its own `*-slash` icons.
            body = (
                f'<g mask="url(#slash)">{body}</g>'
                '<path d="M40 24 L232 216" stroke="#fff" stroke-width="16" stroke-linecap="round"/>'
            )
        parts.append(f'<g transform="translate({x:.6f} {y:.6f}) scale({scale:.6f})" fill="#fff">{body}</g>')
        index.append((name, cell))
    parts.append("</svg>")
    if args.print_hashes:
        return
    atlas_svg = destination / "atlas.svg"
    atlas_svg.write_text("".join(parts), encoding="utf-8")
    subprocess.run([args.resvg, str(atlas_svg), str(destination / "atlas.png")], check=True)
    atlas_svg.unlink()
    # Lossless; resvg writes an unoptimized PNG about 4x larger. Install with `cargo install oxipng`.
    if shutil.which("oxipng"):
        subprocess.run(["oxipng", "-o", "max", "--strip", "all", "-q", destination / "atlas.png"], check=True)
    else:
        print("oxipng not found; atlas.png left at resvg compression", file=sys.stderr)
    (destination / "index.tsv").write_text("".join(f"{name}\t{cell}\n" for name, cell in index), encoding="utf-8")
    (destination / "LICENSE").write_bytes(license_text)
    (destination / "LICENSE-SIMPLE-ICONS").write_bytes(simple_license)
    print(f"{len(sources)} icons; {width}x{height}; atlas {(destination / 'atlas.png').stat().st_size} bytes")
    for file in ["atlas.png", "index.tsv", "LICENSE", "LICENSE-SIMPLE-ICONS"]:
        print(file, hashlib.sha256((destination / file).read_bytes()).hexdigest())


if __name__ == "__main__":
    main()
