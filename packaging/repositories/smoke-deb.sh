#!/bin/sh
# Synthetic, isolated signing/apt verification. Does not install or run Serein.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
export GNUPGHOME="$work/gnupg"
mkdir -m700 "$GNUPGHOME"
gpg --batch --passphrase '' --quick-generate-key \
  'Synthetic repository test <test@example.invalid>' rsa2048 sign 1d
key=$(gpg --with-colons --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
arch=$(dpkg --print-architecture)
mkdir -p "$work/pkg/DEBIAN" "$work/pkg/usr/bin" "$work/input"
cp /bin/true "$work/pkg/usr/bin/serein"
printf 'Package: serein\nVersion: 0.1.0-1\nArchitecture: %s\nMaintainer: Test <test@example.invalid>\nDescription: synthetic test\n' \
  "$arch" > "$work/pkg/DEBIAN/control"
dpkg-deb --build "$work/pkg" "$work/input/serein.deb"
python3 "$script_dir/build.py" --input "$work/input" --output "$work/site" \
  --channel nightly --distribution synthetic --format deb --architecture "$arch" \
  --key "$key" --base-url https://example.invalid
repo="$work/site/nightly/synthetic/$arch/apt"
mkdir -p "$work/lists/partial"
printf 'deb [signed-by=%s/serein.asc] file:%s ./\n' "$repo" "$repo" > "$work/sources.list"
apt-get -o "Dir::Etc::sourcelist=$work/sources.list" -o Dir::Etc::sourceparts=- \
  -o "Dir::State::lists=$work/lists" -o APT::Sandbox::User="$(id -un)" update
# Tampering with authenticated metadata must fail verification.
printf '\nTampered: yes\n' >> "$repo/Release"
if gpg --batch --verify "$repo/Release.gpg" "$repo/Release"; then
  echo 'tampered metadata was accepted' >&2
  exit 1
fi
