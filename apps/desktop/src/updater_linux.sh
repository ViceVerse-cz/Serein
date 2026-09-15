#!/bin/sh
set -eu
stage=$1
installed=$2
parent=$3
commit=$4
# The old image's mount disappears after shutdown. Never reuse its runtime paths.
unset APPIMAGE APPDIR ARGV0 OWD LD_LIBRARY_PATH LD_PRELOAD
printf '%s\n' "$$" > "$stage/helper-ready"
while [ ! -f "$commit" ]; do
  if ! /bin/kill -0 "$parent" 2>/dev/null; then
    [ -f "$commit" ] && break
    exit 0
  fi
  /bin/sleep 1
done
count=0
while /bin/kill -0 "$parent" 2>/dev/null; do
  count=$((count + 1)); [ "$count" -lt 120 ] || exit 0
  /bin/sleep 1
done
backup="$stage/previous.AppImage"
# Keep the old inode recoverable while replacing the public path atomically.
/bin/ln "$installed" "$backup" || exit 1
if ! /bin/mv -f "$stage/package.AppImage" "$installed"; then
  "$installed" &
  exit 1
fi
"$installed" &
launched=$!
/bin/sleep 2
if ! /bin/kill -0 "$launched" 2>/dev/null; then
  /bin/mv -f "$backup" "$installed"
  "$installed" &
  exit 1
fi
/bin/rm -rf "$stage"
