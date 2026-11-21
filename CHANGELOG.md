# Changelog

## v0.2.0

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/wildesPepega/ajazz-dpi/main/install.sh | bash
```

Installs this release (prebuilt x86_64 binary, no Rust needed), the udev rule and
the tray autostart service. Requires glibc ≥ 2.35 (Ubuntu 22.04, Debian 12,
Fedora 36 or newer).

### New

- **Tray icon** (`ajazz-dpi --tray`): battery level as large digits on a coloured
  badge (green / orange ≤ 30 % / red < 10 % / blue while charging). Tooltip
  with details; click opens the GUI.
- **Low-battery alert**: plays the KDE `battery-caution` sound below 10 %, and
  again every time the mouse wakes up while still low. Charging or ≥ 15 %
  re-arms it.
- **systemd user service** `ajazz-dpi-tray.service` for autostart with the
  graphical session.
- **Installer** `install.sh`: downloads the latest release, verifies its SHA-256,
  installs binary, udev rule and service. Options `--version`, `--from-source`,
  `--no-service`, `--uninstall`.
- `--version` flag.
- English README, MIT license.

### Fixed

- GUI and tray can run at the same time: request/reply pairs on the hidraw
  device are serialised with `flock`, so neither consumes the other's replies.

## v0.1.0

First release: GUI for DPI stages (1–6 stages, 50–26 000 DPI), active stage,
polling rate (125–1000 Hz) and battery level over the 2.4 GHz receiver or cable.
