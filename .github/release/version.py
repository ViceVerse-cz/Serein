"""Update only workspace package versions; preserve dependency pins and formatting."""

import glob
import pathlib
import re
import sys
import tomllib


def update(version):
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-nightly\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))?', version):
        raise ValueError('Expected a stable or nightly semantic version')
    manifest = pathlib.Path('Cargo.toml')
    source = manifest.read_text()
    workspace = tomllib.loads(source)['workspace']
    previous = workspace['package']['version']
    names = set()
    for pattern in workspace['members']:
        for path in glob.glob(f'{pattern}/Cargo.toml'):
            package = tomllib.loads(pathlib.Path(path).read_text())['package']
            if package.get('version') == {'workspace': True}:
                names.add(package['name'])
    assert 'serein' in names, 'Desktop must inherit the workspace version'
    updated, count = re.subn(
        r'(\[workspace\.package\]\s*\nversion\s*=\s*")[^"]+("\s*\n)',
        lambda match: match[1] + version + match[2], source, count=1,
    )
    assert count == 1, 'Cannot locate workspace version'
    manifest.write_text(updated)
    for path in map(pathlib.Path, ['Cargo.lock', 'fuzz/Cargo.lock']):
        sections = path.read_text().split('[[package]]')
        for index in range(1, len(sections)):
            section = sections[index]
            package = tomllib.loads(section)
            if package['name'] in names and 'source' not in package:
                assert package['version'] == previous, 'Unexpected local lockfile version'
                section = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', section, count=1)
            # Cargo disambiguates duplicate names as "name version" in dependency lists.
            for name in names:
                section = section.replace(f'"{name} {previous}"', f'"{name} {version}"')
            sections[index] = section
        path.write_text('[[package]]'.join(sections))
    plist = pathlib.Path('packaging/macos/Info.plist')
    source, count = re.subn(
        r'(<key>CFBundleShortVersionString</key><string>)[^<]+(</string>)',
        lambda match: match[1] + version.split('-')[0] + match[2], plist.read_text(),
    )
    assert count == 1, 'Cannot locate Mac version'
    plist.write_text(source)


if __name__ == '__main__':
    update(sys.argv[1])
