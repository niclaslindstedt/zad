#!/bin/sh
# Install the latest zad release for the current OS/architecture.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/niclaslindstedt/zad/main/scripts/install.sh | sh
#
# Environment overrides:
#   ZAD_VERSION=v0.1.2      pin to a specific release tag (default: latest)
#   ZAD_INSTALL_DIR=/path   force a specific install directory
#   ZAD_REPO=owner/repo     download from a fork (default: niclaslindstedt/zad)
#
# Without ZAD_INSTALL_DIR the script picks the first directory from this
# priority list that is already on $PATH and writable:
#
#   1. $HOME/.local/bin
#   2. $HOME/bin
#   3. /usr/local/bin
#
# If none of those are on $PATH, the script falls back to $HOME/.local/bin
# (creating it if needed) and prints the line to add to your shell profile.

set -eu

REPO="${ZAD_REPO:-niclaslindstedt/zad}"
VERSION="${ZAD_VERSION:-latest}"

err() { echo "install: $*" >&2; exit 1; }

os=$(uname -s 2>/dev/null || echo unknown)
arch=$(uname -m 2>/dev/null || echo unknown)

case "$os" in
    Linux)  target_os="unknown-linux-gnu" ;;
    Darwin) target_os="apple-darwin" ;;
    *)      err "unsupported OS: $os (Linux and macOS only; download the Windows build from https://github.com/$REPO/releases)" ;;
esac

case "$arch" in
    x86_64|amd64)   target_arch="x86_64" ;;
    arm64|aarch64)  target_arch="aarch64" ;;
    *)              err "unsupported architecture: $arch" ;;
esac

asset="zad-${target_arch}-${target_os}"

in_path() {
    case ":$PATH:" in
        *":$1:"*) return 0 ;;
        *)        return 1 ;;
    esac
}

pick_dir() {
    if [ -n "${ZAD_INSTALL_DIR:-}" ]; then
        mkdir -p "$ZAD_INSTALL_DIR" || err "could not create $ZAD_INSTALL_DIR"
        [ -w "$ZAD_INSTALL_DIR" ] || err "$ZAD_INSTALL_DIR is not writable"
        printf '%s\n' "$ZAD_INSTALL_DIR"
        return
    fi
    for d in "$HOME/.local/bin" "$HOME/bin" "/usr/local/bin"; do
        if in_path "$d"; then
            if [ -d "$d" ] && [ -w "$d" ]; then
                printf '%s\n' "$d"
                return
            fi
            if [ ! -d "$d" ] && mkdir -p "$d" 2>/dev/null; then
                printf '%s\n' "$d"
                return
            fi
        fi
    done
    fallback="$HOME/.local/bin"
    mkdir -p "$fallback" || err "no writable directory found on \$PATH and could not create $fallback"
    printf '%s\n' "$fallback"
}

install_dir=$(pick_dir)

if [ "$VERSION" = "latest" ]; then
    url="https://github.com/$REPO/releases/latest/download/$asset"
else
    url="https://github.com/$REPO/releases/download/$VERSION/$asset"
fi

tmp=$(mktemp 2>/dev/null || mktemp -t zad) || err "mktemp failed"
trap 'rm -f "$tmp"' EXIT INT TERM

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp" || err "download failed: $url"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$tmp" "$url" || err "download failed: $url"
else
    err "neither curl nor wget is installed"
fi

[ -s "$tmp" ] || err "downloaded file is empty: $url"

chmod +x "$tmp"
target="$install_dir/zad"
mv "$tmp" "$target" || err "could not write $target"
trap - EXIT INT TERM

echo "installed zad to $target"
if ! in_path "$install_dir"; then
    echo "note: $install_dir is not on \$PATH — add this to your shell profile:"
    echo "      export PATH=\"$install_dir:\$PATH\""
fi
