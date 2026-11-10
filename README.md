# ajazz-dpi

Kleines GUI-Tool (Rust + egui) für die **Ajazz AJ159P** unter Linux:

- aktuelle DPI + aktive Stufe anzeigen (aktualisiert alle 1,5 s), Akkustand
- aktive DPI-Stufe umschalten (Radio-Button → sofort)
- DPI-Stufen bearbeiten (Anzahl 1–6, je 50–26000 DPI in 50er-Schritten) → „Übernehmen“
- Polling-Rate 125 / 250 / 500 / 1000 Hz (sofort)

Funktioniert über den 2.4-GHz-Receiver (249a:5c2f) und per Kabel (248a:5c2e).

## Installation

```sh
sudo install -m644 70-ajazz-aj159p.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
cargo build --release
./target/release/ajazz-dpi           # GUI
./target/release/ajazz-dpi --probe   # nur lesen, Rohdaten ausgeben
```

## Hinweise

- Über den Receiver antwortet die Maus nur, **wenn sie bewegt wird**. Liegt sie
  still, zeigt das Tool die zuletzt gelesenen Werte („Maus ruht“). Schreibbefehle
  werden mehrfach versucht – beim Klicken im Fenster ist die Maus ohnehin aktiv.
- Protokoll: 32-Byte-Reports auf Interface 2, siehe `src/protocol.rs`.
  Grundlage: [aj179-linux](https://github.com/Rockeyxx/aj179-linux) (docs/PROTOCOL.md)
  und [Ajazz-AJ159P-Linux](https://github.com/Khabarovsk6000/Ajazz-AJ159P-Linux).
  DPI-Kodierung für den PAW3395 (Sensor-ID 0x95): `raw = dpi/50 − 1`, an deiner Maus verifiziert.
