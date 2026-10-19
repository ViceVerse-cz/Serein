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
BOLD=$(printf '\033[1m')
DIM=$(printf '\033[2m')
BLUE=$(printf '\033[1;34m')
CYAN=$(printf '\033[1;36m')
GREEN=$(printf '\033[1;32m')
PURPLE=$(printf '\033[1;35m')
YELLOW=$(printf '\033[1;33m')
RED=$(printf '\033[1;31m')
NC=$(printf '\033[0m')

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

case "$CHANNEL" in
    nightly|production) ;;
    *) error "SEREIN_CHANNEL must be nightly or production." ;;
esac
# These values enter apt/RPM/pacman configuration, not just download arguments.
# Accept an ASCII HTTPS root without whitespace, credentials, query or fragment.
case "$BASE_URL" in
    https://?*) ;;
    *) error "SEREIN_REPO_BASE_URL must be an HTTPS repository root." ;;
esac
case "$BASE_URL" in
    *[!A-Za-z0-9:/._~%+-]*) error "SEREIN_REPO_BASE_URL contains unsupported URL characters." ;;
esac
authority=${BASE_URL#https://}
authority=${authority%%/*}
[ -n "$authority" ] || error "SEREIN_REPO_BASE_URL must include a host."
BASE_URL=${BASE_URL%/}

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
    # The whole file is installed as a keyring. Checking only its first key would
    # also trust any additional primary key appended to the download. Check GPG's
    # status separately: POSIX sh pipelines otherwise hide a failed decoder.
    if ! key_records=$(gpg --batch --show-keys --with-colons "$keyfile" 2>/dev/null); then
        error "Could not read downloaded signing key."
    fi
    fingerprint=$(printf '%s\n' "$key_records" | awk -F: '
        $1 == "pub" { primary_count++; primary_fingerprint = 1; next }
        $1 == "sub" { primary_fingerprint = 0; next }
        $1 == "fpr" && primary_fingerprint { fingerprint = $10; primary_fingerprint = 0 }
        END {
            if (primary_count != 1 || fingerprint == "") exit 1
            print fingerprint
        }
    ') || error "Downloaded signing key must contain exactly one primary key with a fingerprint."
    # Normalize uppercase
    fingerprint=$(echo "$fingerprint" | tr '[:lower:]' '[:upper:]')
    expected=$(echo "$EXPECTED_FINGERPRINT" | tr '[:lower:]' '[:upper:]')
    if [ "$fingerprint" != "$expected" ]; then
        error "GPG fingerprint mismatch! Expected: $expected, Got: $fingerprint. Refusing to install untrusted key."
    fi
    success "Cryptographic key verified (${DIM}$fingerprint${NC})"
}

configure_rpm() {
    # Generate signature policy locally rather than importing arbitrary settings
    # from an unsigned .repo response. Point only at the verified installed key.
    $SUDO install -Dm644 "$KEY_FILE" /etc/pki/rpm-gpg/serein.asc
    $SUDO rpm --import "$KEY_FILE"
    REPO_FILE="$TEMP_DIR/serein.repo"
    printf '[serein-%s]\nname=Serein %s\nbaseurl=%s\nenabled=1\ngpgcheck=1\nrepo_gpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/serein.asc\n' \
        "$CHANNEL" "$CHANNEL" "$REPO_URL" > "$REPO_FILE"
    $SUDO install -m644 "$REPO_FILE" "$1"
}

banner

if [ ! -f /etc/os-release ]; then
    error "/etc/os-release not found. Unsupported Linux distribution."
fi

. /etc/os-release

DISTRO_ID="$ID"
case " ${ID_LIKE:-} " in
    *" arch "*) DISTRO_ID="arch" ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
    x86_64) DEB_ARCH=amd64 ;;
    aarch64|arm64)
        [ "$DISTRO_ID:${VERSION_ID:-}" = ubuntu:26.04 ] || \
            error "Architecture $ARCH is supported only on Ubuntu 26.04."
        DEB_ARCH=arm64
        ;;
    *)
        error "Architecture $ARCH is not currently supported by Serein package repositories."
        ;;
esac

# Native packages must match the distribution that built their shared libraries.
case "$DISTRO_ID:${VERSION_ID:-}" in
    ubuntu:26.04) REPO_PATH="ubuntu-26.04/$DEB_ARCH/apt" ;;
    fedora:43|fedora:44) REPO_PATH="fedora-$VERSION_ID/$ARCH/rpm" ;;
    opensuse-tumbleweed:*) REPO_PATH="opensuse-tumbleweed/$ARCH/rpm" ;;
    arch:*) REPO_PATH="arch/$ARCH/arch" ;;
    *) error "No matching native repository for $ID ${VERSION_ID:-rolling}. Use the Flatpak bundle." ;;
esac
REPO_URL="$BASE_URL/$CHANNEL/$REPO_PATH"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM
KEY_URL="$REPO_URL/serein.asc"
KEY_FILE="$TEMP_DIR/serein.asc"

log "Configuring Serein ${BOLD}${CHANNEL}${NC} repository for ${BOLD}${PRETTY_NAME:-$ID}${NC} (${ARCH})..."
log "Fetching official signing key..."
download "$KEY_URL" "$KEY_FILE" || error "Signing key unavailable for this distribution; its signed repository must be published before setup."
verify_key "$KEY_FILE"

INSTALL_CMD=""

case "$DISTRO_ID" in
    ubuntu)
        log "Installing APT keyring and source list..."
        $SUDO install -Dm644 "$KEY_FILE" /etc/apt/keyrings/serein.asc

        printf 'deb [arch=%s signed-by=/etc/apt/keyrings/serein.asc] %s ./\n' "$DEB_ARCH" "$REPO_URL" | \
            $SUDO tee /etc/apt/sources.list.d/serein.list >/dev/null

        log "Updating APT package lists..."
        $SUDO apt-get update -o Dir::Etc::sourcelist="sources.list.d/serein.list" -o Dir::Etc::sourceparts="-" >/dev/null 2>&1 || $SUDO apt-get update >/dev/null 2>&1

        INSTALL_CMD="$SUDO apt install serein"
        ;;

    fedora)
        log "Importing RPM key and configuring DNF repository..."

        configure_rpm /etc/yum.repos.d/serein.repo

        INSTALL_CMD="$SUDO dnf install serein"
        ;;

    opensuse-tumbleweed)
        log "Importing RPM key and configuring Zypper repository..."
        configure_rpm /etc/zypp/repos.d/serein.repo
        $SUDO zypper --non-interactive refresh serein-$CHANNEL >/dev/null 2>&1 || true

        INSTALL_CMD="$SUDO zypper install serein"
        ;;

    arch)
        log "Importing key into Pacman keyring..."
        $SUDO pacman-key --add "$KEY_FILE" >/dev/null 2>&1
        $SUDO pacman-key --lsign-key "$EXPECTED_FINGERPRINT" >/dev/null 2>&1

        PACMAN_CONF="/etc/pacman.conf"

        # Replace only Serein's Server directives. Preserve unrelated sections,
        # comments and settings, and support an existing stanza without a Server.
        awk -v server="$REPO_URL" '
            /^[[:space:]]*\[[^]]+\]/ {
                if (serein && !has_server) print "Server = " server
                serein = ($0 ~ /^[[:space:]]*\[serein\][[:space:]]*(#.*)?$/)
                if (serein) { found = 1; has_server = 0 }
            }
            serein && /^[[:space:]]*Server[[:space:]]*=/ {
                print "Server = " server
                has_server = 1
                next
            }
            { print }
            END {
                if (serein && !has_server) print "Server = " server
                if (!found) print "\n[serein]\nSigLevel = Required\nServer = " server
            }
        ' "$PACMAN_CONF" > "$TEMP_DIR/pacman.conf"
        $SUDO install -m644 "$TEMP_DIR/pacman.conf" "$PACMAN_CONF"

        INSTALL_CMD="$SUDO pacman -Syu serein"
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
elif ( : </dev/tty ) 2>/dev/null; then
    TTY_INPUT=1
else
    TTY_INPUT=0
fi

if [ "$TTY_INPUT" -eq 1 ]; then
    printf "%b?%b Would you like to install %bSerein%b now? [Y/n]: " "${PURPLE}" "${NC}" "${BOLD}" "${NC}"
    if [ -t 0 ]; then
        read -r answer || answer=n
    else
        read -r answer </dev/tty || answer=n
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
    # Package managers also prompt for keys/transactions when the script is piped.
    if [ -t 0 ]; then
        $INSTALL_CMD
    else
        $INSTALL_CMD </dev/tty
    fi
    printf '\n'
    success "${BOLD}Serein installed successfully!${NC}"
    log "Launch it from your desktop application launcher or run ${BOLD}serein${NC}."
else
    log "To install Serein later, run:"
    printf '\n    %b%s%b\n\n' "${CYAN}" "$INSTALL_CMD" "${NC}"
fi
