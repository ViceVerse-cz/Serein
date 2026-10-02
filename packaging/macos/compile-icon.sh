#!/bin/sh
# Compile native appearance variants; retain the pack's ICNS for older macOS.
set -eu
resources="$1"
mkdir -p "$resources"
# actool caches relative output arguments across worktrees; give it a unique path.
resources="$(cd "$resources" && pwd -P)"
xcrun actool packaging/macos/Serein.icon \
  --compile "$resources" --platform macosx --target-device mac \
  --minimum-deployment-target 13.0 --app-icon Serein \
  --output-partial-info-plist "$resources/icon-info.plist"
cp packaging/macos/Serein.icns "$resources/Serein.icns"
rm "$resources/icon-info.plist"
