#!/bin/sh
# Build tools only; downloads are pinned by commit/release tag and SHA-256.
set -eu
test "$(uname -s)-$(uname -m)" = Linux-x86_64
mkdir -p target/appimage-tools
cd target/appimage-tools

fetch() {
  url=$1
  sha=$2
  dest=$3
  mode=$4
  mkdir -p "$(dirname "$dest")"
  if [ -f "$dest" ] && printf '%s  %s\n' "$sha" "$dest" | sha256sum --check >/dev/null 2>&1; then
    chmod "$mode" "$dest"
    return 0
  fi
  curl --proto '=https' --tlsv1.2 --fail --location --retry 3 "$url" --output "$dest.download"
  printf '%s  %s\n' "$sha" "$dest.download" | sha256sum --check
  mv "$dest.download" "$dest"
  chmod "$mode" "$dest"
}

fetch \
  https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/5c1f0f9ebb88a2aa2229a4ddea26027edae0f238/useful-tools/quick-sharun.sh \
  0aea2f589658e5db1659a6ccb8d29546c4e03028f1e29b37c6450cece917249a \
  quick-sharun.sh \
  755

fetch \
  https://github.com/pkgforge-dev/Anylinux-sharun/releases/download/3.9.0/sharun+helper-libs-x86_64.tar \
  cc820113162cd8a56ccc66a85b638d9c4fc3802819327719323911adc9311b2b \
  sharun+helper-libs-x86_64.tar \
  644

fetch \
  https://github.com/pkgforge-dev/cross-libc-dlopen/releases/download/v0.2.11/cross-libc-dlopen-x86_64.tar \
  35bd45e4d0ce94e8ac86b2dded676f4a371747d0f47004861d4c25f374351ec8 \
  cross-libc-dlopen-x86_64.tar \
  644

fetch \
  https://github.com/pkgforge-dev/appimagetool/releases/download/0.7.0/appimagetool-x86_64-linux \
  b8bc7a63782978a8aebc0b7012ee8962eba69e05d0dfc8289b508d463c761887 \
  appimagetool \
  755

fetch \
  https://github.com/VHSgunzo/uruntime/releases/download/v0.8.3/uruntime-appimage-dwarfs-lite-x86_64 \
  649948f2bf4c012fa909f0b746949f22e97fd4a9b86ae1aea0512b70813a9248 \
  runtime-x86_64 \
  755

fetch \
  https://github.com/mhx/dwarfs/releases/download/v0.15.6/dwarfs-universal-0.15.6-Linux-x86_64 \
  50891c38ba359db8271819a6cbf6aaa8068681523f0c4f2b8242007a45edaa28 \
  bin/mkdwarfs \
  755
