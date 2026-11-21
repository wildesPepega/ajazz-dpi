# ajazz-dpi

A small, dependency-light Linux tool for the **Ajazz AJ159P** gaming mouse, written in Rust.

- **GUI** (egui): view and change DPI stages and the polling rate.
- **Tray icon**: shows the battery level in the system tray (KDE Plasma and any
  other StatusNotifierItem host). Click it to open the GUI.
- **Low-battery alert**: plays the KDE `battery-caution` sound when the battery
  drops below 10 %.
- **Autostart**: ships a systemd user service for the tray.

It talks to the mouse directly via `hidraw` – no vendor software, no kernel
driver, no root at runtime (after installing a udev rule).

> The user interface is currently in German.

## Quick install

```sh
curl -fsSL https://raw.githubusercontent.com/wildesPepega/ajazz-dpi/main/install.sh | bash
```

This builds from source (needs [Rust](https://rustup.rs) and `git`), installs the
binary to `~/.local/bin`, the udev rule (asks for your sudo password) and
enables the tray icon as a systemd user service that starts with your session.
Running it again updates an existing installation.

Prefer to read the script first? [`install.sh`](install.sh) – or clone the repo
and run `./install.sh` from the checkout.

---

## Contents

- [Features](#features)
- [Supported hardware](#supported-hardware)
- [Requirements](#requirements)
- [Quick install](#quick-install)
- [Installation](#installation)
- [Usage](#usage)
- [Tray icon & low-battery alert](#tray-icon--low-battery-alert)
- [Running as a systemd service (autostart)](#running-as-a-systemd-service-autostart)
- [Updating](#updating)
- [Uninstalling](#uninstalling)
- [Troubleshooting](#troubleshooting)
- [How it works](#how-it-works)
- [Development](#development)
- [Credits](#credits)
- [License](#license)

---

## Features

### GUI (`ajazz-dpi`)

- Shows the current DPI and the active stage (refreshed every 1.5 s).
- Shows device ID, connection type (2.4 GHz / cable), battery level and charging state.
- Switch the active DPI stage with one click (applied immediately).
- Edit the DPI stages: 1–6 stages, each 50–26 000 DPI in steps of 50
  (sensor-dependent), then press **Übernehmen** (apply) or **Verwerfen** (discard).
  Double-click a slider value to type an exact number.
- Set the polling rate: 125 / 250 / 500 / 1000 Hz (applied immediately).
- Write commands are retried automatically if the mouse is resting.

### Tray (`ajazz-dpi --tray`)

- Battery level as large, centred digits on a coloured rounded square:

  | Colour | Meaning |
  |--------|---------|
  | green  | > 30 % |
  | orange | 10–30 % |
  | red    | < 10 % |
  | blue   | charging |
  | grey `–` | no data yet / mouse not found |

- Tooltip with the exact value, charging state and "mouse asleep" hint.
- Left click (or **Öffnen** in the context menu) opens the GUI; if the GUI is
  already open, nothing happens. **Beenden** quits the tray.
- Low-battery sound – see [below](#tray-icon--low-battery-alert).
- Polls every 5 s. The query is answered by the receiver itself, so it is cheap.

## Supported hardware

| Connection | USB ID |
|-----------|--------|
| 2.4 GHz receiver | `249a:5c2f` |
| USB cable | `248a:5c2e` (also `248a:5d2e`, `248a:5e2e`) |

Developed and tested with an AJ159P (PAW3395 sensor). Other mice using the same
XCTECH receiver protocol may work; the DPI encoding for several other sensors is
included but untested.

Bluetooth is not supported.

## Requirements

- Linux with `hidraw` (any modern kernel)
- Rust toolchain (edition 2024, i.e. Rust ≥ 1.85) – install via [rustup](https://rustup.rs)
- Wayland or X11
- For the tray: a StatusNotifierItem host (KDE Plasma has one built in; GNOME
  needs the *AppIndicator* extension)
- For the alert sound: `pw-play` (PipeWire), `paplay` (PulseAudio) or
  `canberra-gtk-play`, plus the KDE *ocean* sound theme
  (`/usr/share/sounds/ocean/stereo/battery-caution.oga`)
- Optional: Noto Sans (Condensed) Bold for the tray digits. Any bold sans font
  found via `fc-match` works too; without one, a built-in pixel font is used.

## Installation

The [quick install](#quick-install) above does all of the following. Options:

```sh
./install.sh                # build + install everything
./install.sh --no-service   # without the tray autostart service
./install.sh --uninstall    # remove binary, service and udev rule
```

To do it manually instead:

### 1. udev rule (one-time, needs root)

Grants the logged-in user access to the mouse's configuration interface:

```sh
sudo install -m644 70-ajazz-aj159p.rules /etc/udev/rules.d/
sudo udevadm control --reload
sudo udevadm trigger
```

Re-plug the receiver/cable if the tool still reports a permission error.

### 2. Build

```sh
git clone https://github.com/wildesPepega/ajazz-dpi.git
cd ajazz-dpi
cargo build --release
```

The binary is `target/release/ajazz-dpi`.

### 3. Install the binary

```sh
install -Dm755 target/release/ajazz-dpi ~/.local/bin/ajazz-dpi
```

Make sure `~/.local/bin` is in your `PATH` if you want to start it from a shell.

## Usage

```sh
ajazz-dpi            # open the GUI
ajazz-dpi --tray     # run the tray icon (battery level + low-battery alert)
ajazz-dpi --probe    # read-only diagnostics: print raw replies and parsed values
```

### Wireless notes

Over the 2.4 GHz receiver the mouse only answers **while it is being moved**.
When it rests, the GUI keeps showing the last values and says so
("Maus ruht"). When it is in deep sleep the GUI shows "Maus schläft – kurz
bewegen" (move it briefly). Clicking inside the window usually wakes it, and
write commands are retried a few times.

## Tray icon & low-battery alert

The tray plays the KDE `battery-caution` sound when:

1. the battery drops **below 10 %** while not charging (once), and
2. **every time the mouse wakes up** (or the receiver is plugged back in) while
   the battery is still below 10 %, and
3. at tray start-up if the battery is already below 10 %.

No sound is played while the mouse sleeps or charges. Charging or reaching
**15 %** re-arms the alert. The sound player is tried in this order: `pw-play`,
`paplay`, `canberra-gtk-play -i battery-caution`.

Thresholds and the poll interval are constants at the top of
[`src/tray.rs`](src/tray.rs) (`LOW_BATTERY`, `REARM_BATTERY`, `POLL_INTERVAL`,
`ALERT_SOUND`).

## Running as a systemd service (autostart)

[`ajazz-dpi-tray.service`](ajazz-dpi-tray.service) is a systemd **user** service
that starts with your graphical session:

```sh
install -Dm644 ajazz-dpi-tray.service ~/.config/systemd/user/ajazz-dpi-tray.service
systemctl --user daemon-reload
systemctl --user enable --now ajazz-dpi-tray.service
```

It expects the binary at `~/.local/bin/ajazz-dpi` (edit `ExecStart` otherwise),
is bound to `graphical-session.target` and restarts on failure. It also works if
the panel comes up after the service.

Useful commands:

```sh
systemctl --user status ajazz-dpi-tray     # state
journalctl --user -u ajazz-dpi-tray -f     # logs
systemctl --user restart ajazz-dpi-tray    # restart (also closes a GUI opened from the tray)
systemctl --user disable --now ajazz-dpi-tray
```

A GUI window opened from the tray runs inside the service's cgroup, so stopping
or restarting the service closes it as well.

## Updating

Re-run the [quick install](#quick-install) command, or manually:

```sh
git pull
cargo build --release
install -Dm755 target/release/ajazz-dpi ~/.local/bin/ajazz-dpi
systemctl --user restart ajazz-dpi-tray
```

## Uninstalling

```sh
./install.sh --uninstall
# or: curl -fsSL https://raw.githubusercontent.com/wildesPepega/ajazz-dpi/main/install.sh | bash -s -- --uninstall
```

Manually:

```sh
systemctl --user disable --now ajazz-dpi-tray
rm ~/.config/systemd/user/ajazz-dpi-tray.service ~/.local/bin/ajazz-dpi
systemctl --user daemon-reload
sudo rm /etc/udev/rules.d/70-ajazz-aj159p.rules
sudo udevadm control --reload
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| "Keine AJ159P gefunden" (not found) | Receiver/cable plugged in? Check `lsusb` for the IDs above. |
| "Keine Berechtigung für /dev/hidrawN" (permission denied) | Install the udev rule (see above) and re-plug the device. |
| Values don't update, "Maus ruht" / "Maus schläft" | Move the mouse – over 2.4 GHz it only answers while active. |
| Tray icon missing | Check `journalctl --user -u ajazz-dpi-tray`. On GNOME install the AppIndicator extension. |
| No alert sound | Run `pw-play /usr/share/sounds/ocean/stereo/battery-caution.oga` manually; install the KDE sound theme or one of the players listed above. |
| Raw protocol debugging | `ajazz-dpi --probe` prints raw 32-byte replies and parsed values. |

## How it works

```
src/
├── main.rs      GUI (egui/eframe) + background worker thread, CLI flags
├── tray.rs      tray icon (ksni), battery polling, alert logic, icon rendering
├── hid.rs       hidraw transport: device discovery, request/reply with timeout
└── protocol.rs  AJ159P vendor protocol: packet building and parsing
```

### Transport

The tool scans `/sys/class/hidraw`, matches the vendor/product IDs and picks the
interface whose `HID_PHYS` ends in `/input2` – the vendor configuration
interface with 32-byte reports. Requests are written to `/dev/hidrawN` and the
reply with the matching command byte is awaited (with a timeout).

Since every process that opens a hidraw node receives *every* input report, the
GUI and the tray serialise their request/reply pairs with an exclusive `flock`
on the device node, so they never consume each other's replies.

### Protocol

Every request and reply is a 32-byte report on interface 2 (no report IDs):

```
[0]      command
[1]      status (0 = OK in replies; 1 = mouse resting; 2 = mouse asleep/offline)
[2]      (chunk_index << 4) | chunk_count – 0x01 for writes, 0x00 for reads
[3]      payload length
[4..31]  payload, zero-padded
[31]     checksum = sum(bytes[4..31]) & 0xFF
```

| Command | Meaning |
|---------|---------|
| `0x10` | device info: ID, sensor ID (byte 10), charging (12), battery % (13), mouse online (14) |
| `0x12` / `0x02` | get / set polling rate (bit flag: 1000 Hz = `0x01`, 500 = `0x02`, 250 = `0x04`, 125 = `0x08`) |
| `0x13` / `0x03` | get / set DPI table: `(active << 4) \| count`, then 6 × (X, Y) little-endian 16-bit values |

DPI values are sensor-specific; for the PAW3395 (sensor ID `0x95`) the raw value
is `dpi / 50 − 1`. See [`src/protocol.rs`](src/protocol.rs) for all sensors.

### Tray icon

The icon is rendered at runtime (16–64 px) as ARGB pixmaps: an anti-aliased
rounded square plus the digits, rasterised with `ab_glyph` from a system font
and centred on their exact outline bounds. It is published via the
StatusNotifierItem D-Bus protocol using `ksni`.

## Development

```sh
cargo build              # debug build
cargo test               # unit tests (protocol encoding, alert logic)
cargo run -- --tray      # run the tray from the source tree

# Render tray icon previews (PPM) into a directory:
ICON_DIR=/tmp/icons cargo test dump_icons -- --ignored
```

The release profile is size-optimised (`opt-level = "s"`, LTO, stripped).

## Credits

The protocol is based on reverse-engineering work by:

- [Rockeyxx/aj179-linux](https://github.com/Rockeyxx/aj179-linux) (`docs/PROTOCOL.md`)
- [Khabarovsk6000/Ajazz-AJ159P-Linux](https://github.com/Khabarovsk6000/Ajazz-AJ159P-Linux)

## License

[MIT](LICENSE)

Not affiliated with Ajazz.
