#!/bin/sh
# Compile native appearance variants; retain the pack's ICNS for older macOS.
set -eu
resources="$1"
mkdir -p "$resources"
# actool caches relative output arguments across worktrees; give it a unique path.
resources="$(cd "$resources" && pwd -P)"
if xcrun actool --version >/dev/null 2>&1; then
  xcrun actool packaging/macos/Serein.icon \
    --compile "$resources" --platform macosx --target-device mac \
    --minimum-deployment-target 13.0 --app-icon Serein \
    --output-partial-info-plist "$resources/icon-info.plist"
  rm -f "$resources/icon-info.plist"
fi
cp packaging/macos/Serein.icns "$resources/Serein.icns"
