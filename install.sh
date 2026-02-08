#!/bin/sh
set -e

REPO="euceph/ovid"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

if [ -t 1 ]; then
    BOLD='\033[1m'
    BLUE='\033[1;34m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    RED='\033[0;31m'
    DIM='\033[2m'
    RESET='\033[0m'
else
    BOLD=''
    BLUE=''
    GREEN=''
    YELLOW=''
    RED=''
    DIM=''
    RESET=''
fi

info() {
    printf "${BOLD}%s${RESET}\n" "$*"
}

success() {
    printf "${GREEN}%s${RESET}\n" "$*"
}

warn() {
    printf "${YELLOW}warning:${RESET} %s\n" "$*"
}

error() {
    printf "${RED}error:${RESET} %s\n" "$*" >&2
}

step() {
    printf "${BLUE}[%s/%s]${RESET} %s" "$1" "$2" "$3"
}

install_jpeg_turbo() {
    OS="$(uname -s)"

    case "$OS" in
        Darwin)
            if ! brew list jpeg-turbo >/dev/null 2>&1; then
                if command -v brew >/dev/null 2>&1; then
                    info "Installing libjpeg-turbo via Homebrew..."
                    brew install jpeg-turbo
                else
                    warn "Homebrew not found. Please install libjpeg-turbo manually:"
                    echo "  brew install jpeg-turbo"
                    echo ""
                fi
            fi
            ;;
        Linux)
            if ! ldconfig -p 2>/dev/null | grep -q libturbojpeg; then
                info "libjpeg-turbo not found. Attempting to install..."
                if command -v pacman >/dev/null 2>&1; then
                    sudo pacman -S --noconfirm libjpeg-turbo
                elif command -v dnf >/dev/null 2>&1; then
                    sudo dnf install -y libjpeg-turbo
                elif command -v apt-get >/dev/null 2>&1; then
                    info "Building libjpeg-turbo 3.x from source..."
                    sudo apt-get update && sudo apt-get install -y cmake make gcc curl
                    TMPBUILD="$(mktemp -d)"
                    curl -sL https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/3.1.0/libjpeg-turbo-3.1.0.tar.gz | tar xz -C "$TMPBUILD"
                    cd "$TMPBUILD/libjpeg-turbo-3.1.0"
                    cmake -G"Unix Makefiles" -DCMAKE_INSTALL_PREFIX=/usr .
                    make -j$(nproc)
                    sudo make install
                    sudo ldconfig
                    rm -rf "$TMPBUILD"
                else
                    warn "Could not detect package manager. Please install libjpeg-turbo >= 3.0"
                    echo ""
                fi
            fi
            ;;
    esac
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin) OS="darwin" ;;
        Linux) OS="linux" ;;
        *) error "Unsupported OS: $OS"; exit 1 ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        arm64|aarch64) ARCH="arm64" ;;
        *) error "Unsupported architecture: $ARCH"; exit 1 ;;
    esac

    echo "ovid-${OS}-${ARCH}"
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
}

main() {
    printf "${BLUE}"
    cat <<'BANNER'
              __     __
.-----.--.--.|__|.--|  |
|  _  |  |  ||  ||  _  |
|_____|\___/ |__||_____|
BANNER
    printf "${RESET}\n"

    STEPS=4

    step 1 "$STEPS" "Checking dependencies..."
    printf "\n"
    install_jpeg_turbo

    step 2 "$STEPS" "Detecting platform..."
    PLATFORM="$(detect_platform)"
    printf "  ${DIM}%s${RESET}\n" "$PLATFORM"

    VERSION="${VERSION:-$(get_latest_version)}"
    if [ -z "$VERSION" ]; then
        error "Failed to determine latest version"
        exit 1
    fi

    URL="https://github.com/${REPO}/releases/download/${VERSION}/${PLATFORM}.tar.gz"

    step 3 "$STEPS" "Downloading ovid ${VERSION}..."
    printf "\n"
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT
    if [ -t 1 ]; then
        curl -fL --progress-bar "$URL" -o "$TMPDIR/ovid.tar.gz"
    else
        curl -fsSL "$URL" -o "$TMPDIR/ovid.tar.gz"
    fi
    tar -xz -C "$TMPDIR" -f "$TMPDIR/ovid.tar.gz"

    step 4 "$STEPS" "Installing to ${INSTALL_DIR}..."
    printf "\n"
    if [ -w "$INSTALL_DIR" ]; then
        mv "$TMPDIR/ovid" "$INSTALL_DIR/ovid"
    else
        info "  requires sudo"
        sudo mv "$TMPDIR/ovid" "$INSTALL_DIR/ovid"
    fi
    chmod +x "$INSTALL_DIR/ovid"

    printf "\n"
    success "  ovid ${VERSION} installed successfully!"
    printf "  Run ${BOLD}ovid --help${RESET} to get started.\n"

    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            printf "\n"
            warn "${INSTALL_DIR} is not in your PATH."
            printf "  Add it by running:\n"
            printf "    ${BOLD}export PATH=\"%s:\$PATH\"${RESET}\n" "$INSTALL_DIR"
            ;;
    esac
}

main
