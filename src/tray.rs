//! Tray mode (`--tray`): battery level as a StatusNotifierItem icon, click opens
//! the GUI, low-battery sound below `LOW_BATTERY` %.

use crate::{hid, protocol};
use ksni::blocking::TrayMethods;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

/// Short enough to notice the mouse waking up (cheap: answered by the receiver).
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const REPLY_TIMEOUT: Duration = Duration::from_millis(2500);
/// Alert when the level drops below this (and not charging).
const LOW_BATTERY: u8 = 10;
/// Re-arm the alert once the level is back at this (or charging).
const REARM_BATTERY: u8 = 15;
/// Candidates, tried in order until one plays.
const ALERT_SOUND: &[(&str, &[&str])] = &[
    ("pw-play", &["/usr/share/sounds/ocean/stereo/battery-caution.oga"]),
    ("paplay", &["/usr/share/sounds/ocean/stereo/battery-caution.oga"]),
    ("canberra-gtk-play", &["-i", "battery-caution"]),
];

#[derive(Clone, Default, PartialEq)]
struct Battery {
    /// Last known level; kept while the mouse sleeps.
    level: Option<u8>,
    charging: bool,
    asleep: bool,
    error: Option<String>,
}

struct AjazzTray {
    bat: Battery,
    gui: Option<Child>,
}

impl AjazzTray {
    fn reap(&mut self) {
        if let Some(c) = &mut self.gui {
            if !matches!(c.try_wait(), Ok(None)) {
                self.gui = None;
            }
        }
    }

    fn open_gui(&mut self) {
        self.reap();
        if self.gui.is_some() {
            return; // already open
        }
        match Command::new(self_exe()).stdin(Stdio::null()).spawn() {
            Ok(c) => self.gui = Some(c),
            Err(e) => eprintln!("GUI starten fehlgeschlagen: {e}"),
        }
    }
}

/// Path of the running binary, also valid after it was replaced on disk.
fn self_exe() -> PathBuf {
    let p = std::env::current_exe().unwrap_or_else(|_| "ajazz-dpi".into());
    let s = p.to_string_lossy();
    match s.strip_suffix(" (deleted)") {
        Some(orig) => orig.into(),
        None => p,
    }
}

impl ksni::Tray for AjazzTray {
    fn id(&self) -> String {
        "ajazz-dpi".into()
    }
    fn title(&self) -> String {
        "AJ159P".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [16, 22, 32, 48, 64].into_iter().map(|s| render_icon(&self.bat, s)).collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        let b = &self.bat;
        let description = match (&b.error, b.level) {
            (Some(e), _) => e.clone(),
            (None, None) => "Maus schläft – kurz bewegen".into(),
            (None, Some(l)) => {
                let mut s = format!("Akku {l} %");
                if b.charging {
                    s.push_str(" (lädt)");
                }
                if b.asleep {
                    s.push_str(" · Maus schläft");
                }
                s
            }
        };
        ksni::ToolTip { title: "Ajazz AJ159P".into(), description, ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.open_gui();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Öffnen".into(),
                icon_name: "input-mouse".into(),
                activate: Box::new(|t: &mut Self| t.open_gui()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Beenden".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

// --------------------------------------------------------------- polling

fn read_battery(dev: &mut Option<hid::Device>, prev: &Battery) -> Battery {
    if dev.is_none() {
        match hid::Device::open() {
            Ok(d) => *dev = Some(d),
            Err(e) => return Battery { error: Some(e.to_string()), ..Default::default() },
        }
    }
    let d = dev.as_mut().unwrap();
    match d.transact(&protocol::read_request(protocol::CMD_DEVICE_INFO), REPLY_TIMEOUT) {
        // Without the mouse online the receiver's battery byte is meaningless.
        Ok(r) => {
            let info = protocol::parse_info(&r, d.wired);
            if info.mouse_online {
                Battery { level: Some(info.battery), charging: info.charging, ..Default::default() }
            } else {
                Battery { asleep: true, error: None, ..prev.clone() }
            }
        }
        Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::NotConnected) => {
            Battery { asleep: true, error: None, ..prev.clone() }
        }
        Err(e) => {
            *dev = None; // unplugged: reopen next time
            Battery { error: Some(e.to_string()), ..Default::default() }
        }
    }
}

fn play_alert() {
    thread::spawn(|| {
        for (cmd, args) in ALERT_SOUND {
            let ok = Command::new(cmd)
                .args(*args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                return;
            }
        }
        eprintln!("Warnton konnte nicht abgespielt werden");
    });
}

/// Decide whether to play the low-battery sound for this reading.
///
/// Fires once when the level drops below `LOW_BATTERY`, and again every time the
/// mouse wakes up (or reconnects) while still low. Re-armed by charging or by
/// reaching `REARM_BATTERY`.
fn low_battery_alert(prev: &Battery, new: &Battery, alerted: &mut bool) -> bool {
    let Some(l) = new.level else { return false };
    if new.asleep || new.error.is_some() {
        return false;
    }
    let woke = prev.asleep || prev.error.is_some() || prev.level.is_none();
    if new.charging || l >= REARM_BATTERY || woke {
        *alerted = false;
    }
    if !new.charging && l < LOW_BATTERY && !*alerted {
        *alerted = true;
        return true;
    }
    false
}

pub fn run() {
    let tray = AjazzTray { bat: Battery::default(), gui: None };
    // The tray host may come up after us at login.
    let handle = match tray.assume_sni_available(true).spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Tray nicht verfügbar: {e}");
            std::process::exit(1);
        }
    };
    let mut dev = None;
    let mut bat = Battery::default();
    let mut alerted = false;
    loop {
        let new = read_battery(&mut dev, &bat);
        if low_battery_alert(&bat, &new, &mut alerted) {
            play_alert();
        }
        let changed = new != bat;
        bat = new;
        let b = bat.clone();
        handle.update(move |t| {
            t.reap();
            if changed {
                t.bat = b;
            }
        });
        if handle.is_closed() {
            return;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

// ----------------------------------------------------------------- icon

/// Bold system font for the tray text; `None` → built-in pixel font.
fn font() -> Option<&'static ab_glyph::FontVec> {
    static FONT: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();
    FONT.get_or_init(|| {
        let mut paths: Vec<PathBuf> = [
            "/usr/share/fonts/noto/NotoSans-CondensedBold.ttf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSansCondensed-Bold.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        if let Ok(o) = Command::new("fc-match").args(["-f", "%{file}", "sans:bold"]).output() {
            paths.push(String::from_utf8_lossy(&o.stdout).into_owned().into());
        }
        paths
            .iter()
            .filter_map(|p| std::fs::read(p).ok())
            .find_map(|data| ab_glyph::FontVec::try_from_vec(data).ok())
    })
    .as_ref()
}

/// Square icon: coloured rounded background with the level as big white digits.
fn render_icon(b: &Battery, size: i32) -> ksni::Icon {
    let (text, bg): (String, [u8; 3]) = match b.level {
        None => ("–".into(), [0x7f, 0x8c, 0x8d]),
        Some(l) => {
            let bg = if b.charging {
                [0x1d, 0x99, 0xf3] // blue
            } else if l < LOW_BATTERY {
                [0xda, 0x44, 0x53] // red
            } else if l <= 30 {
                [0xe0, 0x6a, 0x00] // orange
            } else {
                [0x1e, 0x9e, 0x52] // green
            };
            (l.to_string(), bg)
        }
    };
    let s = size as usize;
    // Coverage of the background shape and of the text, 0.0..=1.0.
    let mut shape = vec![0f32; s * s];
    let mut ink = vec![0f32; s * s];

    // Background with rounded corners (anti-aliased).
    let r = (s as f32 / 5.0).max(2.0);
    let n = s as f32;
    for y in 0..s {
        for x in 0..s {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = (r - px).max(px - (n - r)).max(0.0);
            let cy = (r - py).max(py - (n - r)).max(0.0);
            let d = (cx * cx + cy * cy).sqrt();
            shape[y * s + x] = (r - d + 0.5).clamp(0.0, 1.0);
        }
    }

    match font() {
        Some(f) => draw_text_font(f, &text, s, &mut ink),
        None => draw_text_pixel(&text, s, &mut ink),
    }

    let data = shape
        .iter()
        .zip(&ink)
        .flat_map(|(&a, &t)| {
            let mix = |c: u8| (c as f32 + (255.0 - c as f32) * t).round() as u8;
            [(a * 255.0).round() as u8, mix(bg[0]), mix(bg[1]), mix(bg[2])]
        })
        .collect();
    ksni::Icon { width: size, height: size, data }
}

/// Lay out `text` at `px` with the baseline origin at `origin`. Returns the
/// outlined glyphs and the exact (unrounded) ink bounds.
fn layout(
    f: &ab_glyph::FontVec,
    text: &str,
    px: f32,
    origin: ab_glyph::Point,
) -> (Vec<ab_glyph::OutlinedGlyph>, ab_glyph::Rect) {
    use ab_glyph::{Font, ScaleFont, point};
    let sf = f.as_scaled(px);
    let (hs, vs) = (sf.h_scale_factor(), sf.v_scale_factor());
    let mut x = origin.x;
    let mut prev = None;
    let mut out = Vec::new();
    let mut bb = ab_glyph::Rect { min: point(f32::MAX, f32::MAX), max: point(f32::MIN, f32::MIN) };
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        // Font units are y-up (ab_glyph stores the top in min.y); pixels are
        // y-down from the baseline.
        if let Some(o) = f.outline(id) {
            let b = o.bounds;
            let (top, bottom) = (b.min.y.max(b.max.y), b.min.y.min(b.max.y));
            bb.min.x = bb.min.x.min(x + b.min.x.min(b.max.x) * hs);
            bb.max.x = bb.max.x.max(x + b.min.x.max(b.max.x) * hs);
            bb.min.y = bb.min.y.min(origin.y - top * vs);
            bb.max.y = bb.max.y.max(origin.y - bottom * vs);
        }
        if let Some(o) = f.outline_glyph(id.with_scale_and_position(px, point(x, origin.y))) {
            out.push(o);
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
    (out, bb)
}

fn draw_text_font(f: &ab_glyph::FontVec, text: &str, s: usize, ink: &mut [f32]) {
    use ab_glyph::point;
    let n = s as f32;
    // Fit the ink box: as tall as possible, width-limited for "100".
    let (_, bb) = layout(f, text, n, point(0.0, 0.0));
    if bb.width() <= 0.0 {
        return;
    }
    let max_h = if text.chars().all(|c| c.is_ascii_digit()) { n * 0.72 } else { n * 0.12 };
    let px = n * (n * 0.88 / bb.width()).min(max_h / bb.height());
    // Measure at the final size, then shift so the ink box sits exactly centred.
    let (_, bb) = layout(f, text, px, point(0.0, 0.0));
    let origin = point((n - bb.width()) / 2.0 - bb.min.x, (n - bb.height()) / 2.0 - bb.min.y);
    let (glyphs, _) = layout(f, text, px, origin);
    for g in glyphs {
        let r = g.px_bounds();
        g.draw(|gx, gy, c| {
            let x = r.min.x as i64 + gx as i64;
            let y = r.min.y as i64 + gy as i64;
            if (0..s as i64).contains(&x) && (0..s as i64).contains(&y) {
                let v = &mut ink[y as usize * s + x as usize];
                *v = (*v + c).min(1.0);
            }
        });
    }
}

/// Fallback without a system font: 3×5 pixel glyphs, MSB = left column.
fn draw_text_pixel(text: &str, s: usize, ink: &mut [f32]) {
    fn glyph(c: char) -> [u8; 5] {
        match c {
            '0' => [7, 5, 5, 5, 7],
            '1' => [2, 6, 2, 2, 7],
            '2' => [7, 1, 7, 4, 7],
            '3' => [7, 1, 7, 1, 7],
            '4' => [5, 5, 7, 1, 1],
            '5' => [7, 4, 7, 1, 7],
            '6' => [7, 4, 7, 5, 7],
            '7' => [7, 1, 1, 1, 1],
            '8' => [7, 5, 7, 5, 7],
            '9' => [7, 5, 7, 1, 7],
            _ => [0, 0, 7, 0, 0],
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let cols = chars.len() * 4 - 1;
    let k = (s.saturating_sub(2) / cols).clamp(1, s / 6 + 1);
    let (w, h) = (cols * k, 5 * k);
    let (x0, y0) = (s.saturating_sub(w) / 2, s.saturating_sub(h) / 2);
    for (i, c) in chars.iter().enumerate() {
        for (row, bits) in glyph(*c).iter().enumerate() {
            for col in 0..3 {
                if bits & (4 >> col) == 0 {
                    continue;
                }
                for dy in 0..k {
                    for dx in 0..k {
                        let (x, y) = (x0 + (i * 4 + col) * k + dx, y0 + row * k + dy);
                        if x < s && y < s {
                            ink[y * s + x] = 1.0;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn awake(l: u8) -> Battery {
        Battery { level: Some(l), ..Default::default() }
    }
    fn asleep(l: u8) -> Battery {
        Battery { level: Some(l), asleep: true, ..Default::default() }
    }

    #[test]
    fn alert_sequence() {
        let mut alerted = false;
        let mut prev = awake(12);
        let mut step = |new: Battery| {
            let fired = low_battery_alert(&prev, &new, &mut alerted);
            prev = new;
            fired
        };
        assert!(!step(awake(11)));
        assert!(step(awake(9)), "drop below 10 %");
        assert!(!step(awake(9)), "only once while awake");
        assert!(!step(awake(8)));
        assert!(!step(asleep(8)), "no sound while asleep");
        assert!(step(awake(8)), "again after waking up");
        assert!(!step(awake(8)));
        assert!(!step(Battery { level: Some(8), charging: true, ..Default::default() }));
        assert!(step(awake(9)), "unplugged while still low");
        assert!(!step(Battery { error: Some("weg".into()), ..Default::default() }));
        assert!(step(awake(7)), "receiver reconnected");
        assert!(!step(awake(15)));
        assert!(!step(awake(10)));
        assert!(step(awake(9)));
    }

    #[test]
    fn startup_low() {
        let mut alerted = false;
        assert!(low_battery_alert(&Battery::default(), &awake(5), &mut alerted));
        let mut alerted = false;
        assert!(!low_battery_alert(&Battery::default(), &asleep(5), &mut alerted));
    }

    /// `cargo test dump_icons -- --ignored` writes PPM previews to $ICON_DIR.
    #[test]
    #[ignore]
    fn dump_icons() {
        let dir = std::env::var("ICON_DIR").unwrap_or_else(|_| ".".into());
        for (name, level, charging) in [("100", Some(100), false), ("87", Some(87), false), ("25", Some(25), false), ("7", Some(7), false), ("55c", Some(55), true), ("none", None, false)] {
            for size in [22, 32, 64] {
                let b = Battery { level, charging, ..Default::default() };
                let icon = render_icon(&b, size);
                // Composite onto a dark panel colour.
                let mut ppm = format!("P6 {size} {size} 255\n").into_bytes();
                for p in icon.data.chunks(4) {
                    let a = p[0] as f32 / 255.0;
                    for c in &p[1..] {
                        ppm.push((*c as f32 * a + 0x2a as f32 * (1.0 - a)) as u8);
                    }
                }
                std::fs::write(format!("{dir}/icon_{name}_{size}.ppm"), ppm).unwrap();
            }
        }
    }
}
