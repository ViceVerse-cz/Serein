#!/bin/sh
# Serein Package Repository Setup Script
# Detects distribution (Ubuntu/Debian, Fedora, openSUSE, Arch Linux),
# imports and verifies the GPG signing key, and configures the package repository.
set -eu

BASE_URL="${SEREIN_REPO_BASE_URL:-https://viceverse-cz.github.io/Serein}"
CHANNEL="${SEREIN_CHANNEL:-nightly}"
EXPECTED_FINGERPRINT="${SEREIN_FINGERPRINT:-CA19DA939E9BCAB500751CE480FE95CAD86141A5}"

if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
else
    SUDO=""
fi

log() {
    printf '\033[1;34m::\033[0m %s\n' "$1"
}

error() {
    printf '\033[1;31merror:\033[0m %s\n' "$1" >&2
    exit 1
}

# Ensure required download tools exist
if command -v curl >/dev/null 2>&1; then
    download() { curl --fail --silent --show-error --location "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    download() { wget --quiet -O "$2" "$1"; }
else
    error "Either curl or wget is required to download repository configuration."
fi

# Verify GPG key fingerprint matches expected signing key
verify_key() {
    keyfile="$1"
    if ! command -v gpg >/dev/null 2>&1; then
        error "gpg is required to verify the signing key."
    fi
    fingerprint=$(gpg --batch --show-keys --with-colons "$keyfile" 2>/dev/null | awk -F: '$1 == "fpr" {print $10; exit}')
    if [ -z "$fingerprint" ]; then
        error "Could not extract fingerprint from downloaded signing key."
    fi
    # Normalize uppercase
    fingerprint=$(echo "$fingerprint" | tr '[:lower:]' '[:upper:]')
    expected=$(echo "$EXPECTED_FINGERPRINT" | tr '[:lower:]' '[:upper:]')
    if [ "$fingerprint" != "$expected" ]; then
        error "GPG fingerprint mismatch! Expected: $expected, Got: $fingerprint. Refusing to install untrusted key."
    fi
    log "GPG key verified ($fingerprint)"
}

if [ ! -f /etc/os-release ]; then
    error "/etc/os-release not found. Unsupported Linux distribution."
fi

. /etc/os-release

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)
        DEB_ARCH="amd64"
        RPM_ARCH="x86_64"
        ARCH_ARCH="x86_64"
        ;;
    *)
        error "Architecture $ARCH is not currently supported by Serein package repositories."
        ;;
esac

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

KEY_URL="$BASE_URL/$CHANNEL/ubuntu-26.04/$DEB_ARCH/apt/serein.asc"
KEY_FILE="$TEMP_DIR/serein.asc"

log "Configuring Serein $CHANNEL repository for $ID..."
log "Downloading signing key..."
download "$KEY_URL" "$KEY_FILE"
verify_key "$KEY_FILE"

case "$ID" in
    ubuntu|debian|pop|linuxmint|elementary|neon)
        log "Configuring APT repository..."
        $SUDO install -Dm644 "$KEY_FILE" /etc/apt/keyrings/serein.asc

        # Determine distribution directory; default to ubuntu-26.04 if on derivative
        DIST="ubuntu-26.04"
        REPO_URL="$BASE_URL/$CHANNEL/$DIST/$DEB_ARCH/apt"

        printf 'deb [arch=%s signed-by=/etc/apt/keyrings/serein.asc] %s ./\n' "$DEB_ARCH" "$REPO_URL" | \
            $SUDO tee /etc/apt/sources.list.d/serein.list >/dev/null

        log "Updating package lists..."
        $SUDO apt-get update -o Dir::Etc::sourcelist="sources.list.d/serein.list" -o Dir::Etc::sourceparts="-" || $SUDO apt-get update

        log "Success! Install Serein by running:"
        printf '\n    %s apt install serein\n\n' "$SUDO"
        ;;

    fedora|rhel|centos|rocky|alma)
        log "Configuring DNF repository..."
        $SUDO rpm --import "$KEY_FILE"

        REPO_FILE="$TEMP_DIR/serein.repo"
        REPO_URL="$BASE_URL/$CHANNEL/fedora-44/$RPM_ARCH/rpm"
        download "$REPO_URL/serein.repo" "$REPO_FILE"
        $SUDO install -m644 "$REPO_FILE" /etc/yum.repos.d/serein.repo

        log "Success! Install Serein by running:"
        printf '\n    %s dnf install serein\n\n' "$SUDO"
        ;;

    opensuse*|suse|sles)
        log "Configuring Zypper repository..."
        $SUDO rpm --import "$KEY_FILE"

        REPO_FILE="$TEMP_DIR/serein.repo"
        REPO_URL="$BASE_URL/$CHANNEL/opensuse-tumbleweed/$RPM_ARCH/rpm"
        download "$REPO_URL/serein.repo" "$REPO_FILE"
        $SUDO install -m644 "$REPO_FILE" /etc/zypp/repos.d/serein.repo
        $SUDO zypper --non-interactive refresh || true

        log "Success! Install Serein by running:"
        printf '\n    %s zypper install serein\n\n' "$SUDO"
        ;;

    arch|manjaro|endeavouros|garuda|cachyos)
        log "Configuring Pacman repository..."
        $SUDO pacman-key --add "$KEY_FILE"
        $SUDO pacman-key --lsign-key "$EXPECTED_FINGERPRINT"

        PACMAN_CONF="/etc/pacman.conf"
        REPO_URL="$BASE_URL/$CHANNEL/arch/$ARCH_ARCH/arch"

        if grep -q "\[serein\]" "$PACMAN_CONF"; then
            log "Repository [serein] already present in $PACMAN_CONF."
        else
            printf '\n[serein]\nSigLevel = Required DatabaseOptional\nServer = %s\n' "$REPO_URL" | \
                $SUDO tee -a "$PACMAN_CONF" >/dev/null
        fi

        $SUDO pacman -Sy

        log "Success! Install Serein by running:"
        printf '\n    %s pacman -S serein\n\n' "$SUDO"
        ;;

    *)
        error "Distribution '$ID' is not automatically supported by this script. See packaging/repositories/README.md for manual instructions."
        ;;
esac
