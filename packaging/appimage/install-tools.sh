#!/bin/sh
# Build tools only; downloads are pinned by both version and SHA-256.
set -eu
test "$(uname -s)-$(uname -m)" = Linux-x86_64
mkdir -p target/appimage-tools
cd target/appimage-tools
curl --proto '=https' --tlsv1.2 --fail --location --retry 3 \
  https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
  --output appimagetool.download
printf '%s\n' 'ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0  appimagetool.download' | sha256sum --check
mv appimagetool.download appimagetool
chmod 755 appimagetool
curl --proto '=https' --tlsv1.2 --fail --location --retry 3 \
  https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
  --output runtime-x86_64.download
printf '%s\n' '2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d  runtime-x86_64.download' | sha256sum --check
mv runtime-x86_64.download runtime-x86_64
