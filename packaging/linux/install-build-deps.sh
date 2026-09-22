#!/bin/sh
# CI/source-build hosts only. Run deliberately as root in a native container.
set -eu
. /etc/os-release
case "$ID:${VERSION_ID:-rolling}" in
  ubuntu:24.04|ubuntu:26.04)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential cmake pkg-config curl ca-certificates git tar gzip xz-utils \
      python3 coreutils dpkg-dev desktop-file-utils file \
      libgtk-4-dev libwebkitgtk-6.0-dev libfontconfig1-dev libxkbcommon-dev \
      libwayland-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libvulkan-dev \
      libasound2-dev libpulse-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
    ;;
  fedora:43|fedora:44)
    dnf install -y --setopt=install_weak_deps=False gcc gcc-c++ make cmake pkgconf-pkg-config curl ca-certificates \
      git tar gzip xz python3 coreutils shadow-utils rpm-build cpio desktop-file-utils file \
      gtk4-devel webkitgtk6.0-devel fontconfig-devel libxkbcommon-devel \
      libxkbcommon-x11-devel wayland-devel libX11-devel libXi-devel libXrandr-devel \
      libXcursor-devel vulkan-loader-devel alsa-lib-devel pulseaudio-libs-devel gstreamer1-devel \
      gstreamer1-plugins-base-devel
    ;;
  opensuse-tumbleweed:*)
    zypper --non-interactive install --no-recommends gcc gcc-c++ make cmake \
      pkg-config curl ca-certificates git tar gzip xz python3 coreutils shadow file \
      rpm-build cpio desktop-file-utils gtk4-devel 'pkgconfig(webkitgtk-6.0)' \
      fontconfig-devel libxkbcommon-devel libxkbcommon-x11-devel wayland-devel \
      libX11-devel libXi-devel libXrandr-devel libXcursor-devel vulkan-devel \
      alsa-devel libpulse-devel gstreamer-devel gstreamer-plugins-base-devel
    ;;
  arch:*)
    pacman -Syu --noconfirm --needed base-devel cmake pkgconf curl ca-certificates \
      git tar gzip xz python coreutils desktop-file-utils file gtk4 webkitgtk-6.0 \
      fontconfig libxkbcommon libxkbcommon-x11 wayland libx11 libxi libxrandr \
      libxcursor vulkan-headers vulkan-icd-loader alsa-lib libpulse gstreamer gst-plugins-base-libs
    ;;
  *)
    printf '%s\n' "Unsupported build host: $ID ${VERSION_ID:-rolling}" >&2
    exit 1
    ;;
esac
