#!/usr/bin/env bash
# Installer for ajazz-dpi (Ajazz AJ159P DPI tool + battery tray).
#
#   curl -fsSL https://raw.githubusercontent.com/wildesPepega/ajazz-dpi/main/install.sh | bash
#   ./install.sh [--version vX.Y.Z] [--from-source] [--no-service] [--uninstall]
#
# Downloads the latest GitHub release (prebuilt x86_64 binary), verifies its
# SHA-256 and installs the binary to ~/.local/bin, the udev rule to
# /etc/udev/rules.d (sudo) and enables the tray as a systemd user service.

set -euo pipefail

REPO="wildesPepega/ajazz-dpi"
ASSET="ajazz-dpi-x86_64-linux"
BIN_DIR="$HOME/.local/bin" # the service file expects the binary here
BIN="$BIN_DIR/ajazz-dpi"
SERVICE_NAME="ajazz-dpi-tray.service"
SERVICE_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UDEV_RULE="/etc/udev/rules.d/70-ajazz-aj159p.rules"

if [ -t 1 ]; then
    B=$'\e[1m' G=$'\e[32m' Y=$'\e[33m' R=$'\e[31m' N=$'\e[0m'
else
    B="" G="" Y="" R="" N=""
fi
step() { printf '%s==>%s %s\n' "$B$G" "$N" "$*"; }
warn() { printf '%swarning:%s %s\n' "$B$Y" "$N" "$*" >&2; }
die() { printf '%serror:%s %s\n' "$B$R" "$N" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<EOF
Usage: install.sh [options]

  --version TAG   install a specific release, e.g. v0.2.0 (default: latest)
  --from-source   build from source with cargo instead of downloading a release
  --no-service    don't install/enable the tray systemd user service
  --uninstall     remove binary, service and udev rule
  -h, --help      show this help
EOF
}

TMP="" SRC="" BIN_SRC=""
cleanup() { [ -z "$TMP" ] || rm -rf "$TMP"; }
trap cleanup EXIT
tmpdir() { [ -n "$TMP" ] || TMP="$(mktemp -d)"; }

download() { # url dest
    if have curl; then
        curl -fsSL --retry 3 -o "$2" "$1"
    elif have wget; then
        wget -q -O "$2" "$1"
    else
        die "curl or wget is required."
    fi
}

user_systemd() {
    have systemctl && systemctl --user show-environment >/dev/null 2>&1
}

uninstall() {
    if user_systemd; then
        step "Stopping and disabling $SERVICE_NAME"
        systemctl --user disable --now "$SERVICE_NAME" 2>/dev/null || true
    fi
    if [ -f "$SERVICE_DIR/$SERVICE_NAME" ]; then
        rm -f "$SERVICE_DIR/$SERVICE_NAME"
        if user_systemd; then systemctl --user daemon-reload; fi
    fi
    step "Removing $BIN"
    rm -f "$BIN"
    if [ -f "$UDEV_RULE" ]; then
        step "Removing udev rule (needs sudo)"
        sudo rm -f "$UDEV_RULE"
        sudo udevadm control --reload
    fi
    step "ajazz-dpi uninstalled."
}

# Sets SRC (udev rule + service file) and BIN_SRC (binary) from a release.
fetch_release() { # tag|latest
    [ "$(uname -m)" = x86_64 ] ||
        die "Prebuilt releases are x86_64 only (this is $(uname -m)). Use --from-source."
    have tar || die "tar is required."
    have sha256sum || die "sha256sum is required."

    local base
    if [ "$1" = latest ]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$1"
    fi
    tmpdir
    step "Downloading release ($1)"
    download "$base/$ASSET.tar.gz" "$TMP/$ASSET.tar.gz" ||
        die "Download failed: $base/$ASSET.tar.gz"
    download "$base/$ASSET.tar.gz.sha256" "$TMP/$ASSET.tar.gz.sha256" ||
        die "Download failed: $base/$ASSET.tar.gz.sha256"
    (cd "$TMP" && sha256sum --check --quiet "$ASSET.tar.gz.sha256") ||
        die "Checksum mismatch – download corrupted?"
    tar -xzf "$TMP/$ASSET.tar.gz" -C "$TMP"
    SRC="$TMP/$ASSET"
    BIN_SRC="$SRC/ajazz-dpi"

    # Catch an incompatible glibc before installing anything.
    "$BIN_SRC" --version >/dev/null 2>&1 ||
        die "The prebuilt binary does not run here (needs glibc >= 2.35). Use --from-source."
}

# Sets SRC/BIN_SRC from a source checkout (this script's directory or a fresh clone).
build_from_source() {
    have cargo || die "cargo not found. Install Rust first: https://rustup.rs"
    local self="${BASH_SOURCE[0]:-}" dir=""
    if [ -n "$self" ] && [ -f "$self" ]; then
        dir="$(cd "$(dirname "$self")" && pwd)"
    fi
    if [ -n "$dir" ] && grep -qs '^name = "ajazz-dpi"' "$dir/Cargo.toml"; then
        SRC="$dir"
    else
        have git || die "git is required to download the sources."
        tmpdir
        step "Downloading sources"
        git clone --quiet --depth 1 "https://github.com/$REPO.git" "$TMP/src"
        SRC="$TMP/src"
    fi
    step "Building (cargo build --release) – this can take a few minutes"
    (cd "$SRC" && cargo build --release --locked)
    BIN_SRC="$SRC/target/release/ajazz-dpi"
}

install_udev() {
    if cmp -s "$SRC/70-ajazz-aj159p.rules" "$UDEV_RULE"; then
        step "udev rule already up to date"
        return
    fi
    have sudo || die "sudo is required to install the udev rule."
    step "Installing udev rule to $UDEV_RULE (needs sudo)"
    sudo install -m644 "$SRC/70-ajazz-aj159p.rules" "$UDEV_RULE"
    sudo udevadm control --reload
    sudo udevadm trigger --subsystem-match=hidraw
}

install_service() {
    if ! user_systemd; then
        warn "No systemd user session found – skipping the tray service."
        warn "Start the tray manually with: $BIN --tray"
        return
    fi
    step "Installing systemd user service $SERVICE_NAME"
    install -Dm644 "$SRC/$SERVICE_NAME" "$SERVICE_DIR/$SERVICE_NAME"
    systemctl --user daemon-reload
    systemctl --user enable "$SERVICE_NAME" >/dev/null 2>&1
    # restart picks up a new binary if it was already running
    if systemctl --user restart "$SERVICE_NAME"; then
        step "Tray service enabled and running (starts automatically with your session)"
    else
        warn "Service enabled but failed to start – see: journalctl --user -u $SERVICE_NAME"
    fi
}

main() {
    local service=1 source=0 version=latest
    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                [ $# -ge 2 ] || die "--version needs a tag, e.g. v0.2.0"
                version="$2"
                shift
                ;;
            --from-source) source=1 ;;
            --no-service) service=0 ;;
            --uninstall) uninstall; return ;;
            -h | --help) usage; return ;;
            *) usage >&2; die "unknown option: $1" ;;
        esac
        shift
    done

    [ "$(id -u)" -ne 0 ] || die "Run this as your normal user, not as root (sudo is used where needed)."
    [ "$(uname -s)" = Linux ] || die "ajazz-dpi only supports Linux."

    if [ "$source" -eq 1 ]; then
        build_from_source
    else
        fetch_release "$version"
    fi

    step "Installing $("$BIN_SRC" --version) to $BIN"
    install -Dm755 "$BIN_SRC" "$BIN"

    install_udev
    if [ "$service" -eq 1 ]; then install_service; fi

    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) warn "$BIN_DIR is not in your PATH – add it to run 'ajazz-dpi' from a shell." ;;
    esac

    echo
    step "Done. Open the GUI with 'ajazz-dpi' or click the tray icon."
    echo "    Re-plug the receiver/cable if the mouse is not found right away."
}

# Wrapped in main so a truncated download (curl | bash) never runs half a script.
main "$@"
