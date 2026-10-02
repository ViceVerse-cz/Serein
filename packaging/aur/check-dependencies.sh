#!/usr/bin/env bash
# Compare exact dependency entries, including duplicate names and ABI floors.
set -euo pipefail
cd "$(dirname "$0")"
source ./PKGBUILD
archive=${1:-${source_x86_64[0]##*/}}
comparison_dir=$(mktemp -d)
trap 'rm -rf "$comparison_dir"' EXIT
printf '%s\n' "${depends[@]}" | LC_ALL=C sort > "$comparison_dir/recipe"
bsdtar -xOf "$archive" .PKGINFO > "$comparison_dir/pkginfo"
sed -n 's/^depend = //p' "$comparison_dir/pkginfo" | LC_ALL=C sort > "$comparison_dir/release"
test -s "$comparison_dir/release"
diff -u "$comparison_dir/release" "$comparison_dir/recipe"
