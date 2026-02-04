#!/bin/sh
set -e

REPO="euceph/ovid"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin) OS="darwin" ;;
        Linux) OS="linux" ;;
        *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        arm64|aarch64) ARCH="arm64" ;;
        *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac

    echo "ovid-${OS}-${ARCH}"
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
}

main() {
    PLATFORM="$(detect_platform)"
    VERSION="${VERSION:-$(get_latest_version)}"

    if [ -z "$VERSION" ]; then
        echo "Failed to determine latest version" >&2
        exit 1
    fi

    URL="https://github.com/${REPO}/releases/download/${VERSION}/${PLATFORM}.tar.gz"

    echo "Downloading ovid ${VERSION} for ${PLATFORM}..."

    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    curl -fsSL "$URL" | tar -xz -C "$TMPDIR"

    if [ -w "$INSTALL_DIR" ]; then
        mv "$TMPDIR/ovid" "$INSTALL_DIR/ovid"
    else
        echo "Installing to ${INSTALL_DIR} (requires sudo)..."
        sudo mv "$TMPDIR/ovid" "$INSTALL_DIR/ovid"
    fi

    chmod +x "$INSTALL_DIR/ovid"

    echo "Installed ovid to ${INSTALL_DIR}/ovid"
}

main
