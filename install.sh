#!/usr/bin/env bash
# Installer for ajazz-dpi (Ajazz AJ159P DPI tool + battery tray).
#
#   curl -fsSL https://raw.githubusercontent.com/wildesPepega/ajazz-dpi/main/install.sh | bash
#   ./install.sh [--no-service] [--uninstall]
#
# Builds from source with cargo, installs the binary to ~/.local/bin, the udev
# rule to /etc/udev/rules.d (sudo) and enables the tray as a systemd user service.

set -euo pipefail

REPO_URL="https://github.com/wildesPepega/ajazz-dpi.git"
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

  --no-service   don't install/enable the tray systemd user service
  --uninstall    remove binary, service and udev rule
  -h, --help     show this help
EOF
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
        user_systemd && systemctl --user daemon-reload
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

# Directory with the sources: the script's own checkout, or a fresh clone.
find_source() {
    local self="${BASH_SOURCE[0]:-}"
    if [ -n "$self" ] && [ -f "$self" ]; then
        local dir
        dir="$(cd "$(dirname "$self")" && pwd)"
        if grep -qs '^name = "ajazz-dpi"' "$dir/Cargo.toml"; then
            SRC="$dir"
            return
        fi
    fi
    have git || die "git is required to download the sources."
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    step "Downloading sources"
    git clone --quiet --depth 1 "$REPO_URL" "$TMP/ajazz-dpi"
    SRC="$TMP/ajazz-dpi"
}

install_udev() {
    if cmp -s "$SRC/70-ajazz-aj159p.rules" "$UDEV_RULE"; then
        step "udev rule already up to date"
        return
    fi
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
    local service=1
    for arg in "$@"; do
        case "$arg" in
            --no-service) service=0 ;;
            --uninstall) uninstall; return ;;
            -h | --help) usage; return ;;
            *) usage >&2; die "unknown option: $arg" ;;
        esac
    done

    [ "$(id -u)" -ne 0 ] || die "Run this as your normal user, not as root (sudo is used where needed)."
    [ "$(uname -s)" = Linux ] || die "ajazz-dpi only supports Linux."
    have cargo || die "cargo not found. Install Rust first: https://rustup.rs (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh)"
    have sudo || die "sudo is required to install the udev rule."

    find_source

    step "Building (cargo build --release) – this can take a few minutes"
    (cd "$SRC" && cargo build --release --locked)

    step "Installing binary to $BIN"
    install -Dm755 "$SRC/target/release/ajazz-dpi" "$BIN"

    install_udev
    [ "$service" -eq 1 ] && install_service

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
