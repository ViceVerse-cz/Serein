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

# Styling & Colors
BOLD='\033[1m'
DIM='\033[2m'
BLUE='\033[1;34m'
CYAN='\033[1;36m'
GREEN='\033[1;32m'
PURPLE='\033[1;35m'
YELLOW='\033[1;33m'
RED='\033[1;31m'
NC='\033[0m'

# Disable colors if not running in a terminal
if [ ! -t 1 ]; then
    BOLD=""
    DIM=""
    BLUE=""
    CYAN=""
    GREEN=""
    PURPLE=""
    YELLOW=""
    RED=""
    NC=""
fi

banner() {
    printf '%b' "${CYAN}"
    cat << 'EOF'
     _____ _____ ____  _____ ___ _   _ 
    /  ___|  ___|  _ \| ____|_ _| \ | |
    \ `--.| |__ | |_) | |__  | ||  \| |
     `--. \  __||  _ <|  __| | || |\  |
    /\__/ / |___| | \ \ |___ | || | \ |
    \____/\____/|_|  \_\____/|___|_| \_|
EOF
    printf '%b\n' "${DIM} Lightweight, native Discord client in Rust${NC}\n"
}

log() {
    printf " %b::%b %s\n" "${BLUE}" "${NC}" "$1"
}

success() {
    printf " %b✔%b %s\n" "${GREEN}" "${NC}" "$1"
}

warn() {
    printf " %b!%b %s\n" "${YELLOW}" "${NC}" "$1"
}

error() {
    printf " %b✖ error:%b %s\n" "${RED}" "${NC}" "$1" >&2
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
    success "Cryptographic key verified (${DIM}$fingerprint${NC})"
}

banner

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

log "Configuring Serein ${BOLD}${CHANNEL}${NC} repository for ${BOLD}${PRETTY_NAME:-$ID}${NC} (${ARCH})..."
log "Fetching official signing key..."
download "$KEY_URL" "$KEY_FILE"
verify_key "$KEY_FILE"

INSTALL_CMD=""

case "$ID" in
    ubuntu|debian|pop|linuxmint|elementary|neon)
        log "Installing APT keyring and source list..."
        $SUDO install -Dm644 "$KEY_FILE" /etc/apt/keyrings/serein.asc

        DIST="ubuntu-26.04"
        REPO_URL="$BASE_URL/$CHANNEL/$DIST/$DEB_ARCH/apt"

        printf 'deb [arch=%s signed-by=/etc/apt/keyrings/serein.asc] %s ./\n' "$DEB_ARCH" "$REPO_URL" | \
            $SUDO tee /etc/apt/sources.list.d/serein.list >/dev/null

        log "Updating APT package lists..."
        $SUDO apt-get update -o Dir::Etc::sourcelist="sources.list.d/serein.list" -o Dir::Etc::sourceparts="-" >/dev/null 2>&1 || $SUDO apt-get update >/dev/null 2>&1

        INSTALL_CMD="$SUDO apt install serein"
        ;;

    fedora|rhel|centos|rocky|alma)
        log "Importing RPM key and configuring DNF repository..."
        $SUDO rpm --import "$KEY_FILE"

        REPO_FILE="$TEMP_DIR/serein.repo"
        REPO_URL="$BASE_URL/$CHANNEL/fedora-44/$RPM_ARCH/rpm"
        download "$REPO_URL/serein.repo" "$REPO_FILE"
        $SUDO install -m644 "$REPO_FILE" /etc/yum.repos.d/serein.repo

        INSTALL_CMD="$SUDO dnf install serein"
        ;;

    opensuse*|suse|sles)
        log "Importing RPM key and configuring Zypper repository..."
        $SUDO rpm --import "$KEY_FILE"

        REPO_FILE="$TEMP_DIR/serein.repo"
        REPO_URL="$BASE_URL/$CHANNEL/opensuse-tumbleweed/$RPM_ARCH/rpm"
        download "$REPO_URL/serein.repo" "$REPO_FILE"
        $SUDO install -m644 "$REPO_FILE" /etc/zypp/repos.d/serein.repo
        $SUDO zypper --non-interactive refresh serein-$CHANNEL >/dev/null 2>&1 || true

        INSTALL_CMD="$SUDO zypper install serein"
        ;;

    arch|manjaro|endeavouros|garuda|cachyos)
        log "Importing key into Pacman keyring..."
        $SUDO pacman-key --add "$KEY_FILE" >/dev/null 2>&1
        $SUDO pacman-key --lsign-key "$EXPECTED_FINGERPRINT" >/dev/null 2>&1

        PACMAN_CONF="/etc/pacman.conf"
        REPO_URL="$BASE_URL/$CHANNEL/arch/$ARCH_ARCH/arch"

        if grep -q "\[serein\]" "$PACMAN_CONF"; then
            log "Repository [serein] already present in $PACMAN_CONF."
        else
            printf '\n[serein]\nSigLevel = Required DatabaseOptional\nServer = %s\n' "$REPO_URL" | \
                $SUDO tee -a "$PACMAN_CONF" >/dev/null
        fi

        log "Syncing Pacman database..."
        $SUDO pacman -Sy >/dev/null 2>&1

        INSTALL_CMD="$SUDO pacman -S serein"
        ;;

    *)
        error "Distribution '$ID' is not automatically supported by this script. See packaging/repositories/README.md for manual instructions."
        ;;
esac

success "Repository configuration complete!"
printf '\n'

# Prompt to install if interactive terminal is attached
DO_INSTALL="false"

# When run via `curl ... | sh`, stdin is the script itself. We can read from /dev/tty if available.
if [ -t 0 ]; then
    TTY_INPUT=1
elif [ -e /dev/tty ] && [ -r /dev/tty ]; then
    TTY_INPUT=1
else
    TTY_INPUT=0
fi

if [ "$TTY_INPUT" -eq 1 ]; then
    printf "%b?%b Would you like to install %bSerein%b now? [Y/n]: " "${PURPLE}" "${NC}" "${BOLD}" "${NC}"
    if [ -t 0 ]; then
        read -r answer
    else
        read -r answer </dev/tty
    fi
    case "$answer" in
        [nN][oO]|[nN])
            DO_INSTALL="false"
            ;;
        *)
            DO_INSTALL="true"
            ;;
    esac
fi

if [ "$DO_INSTALL" = "true" ]; then
    log "Installing Serein (${INSTALL_CMD})..."
    $INSTALL_CMD
    printf '\n'
    success "${BOLD}Serein installed successfully!${NC}"
    log "Launch it from your desktop application launcher or run ${BOLD}serein${NC}."
else
    log "To install Serein later, run:"
    printf '\n    %b%s%b\n\n' "${CYAN}" "$INSTALL_CMD" "${NC}"
fi
