mod hid;
mod protocol;

use eframe::egui;
use protocol::{DeviceInfo, DpiTable, Report, Sensor, RATES, STAGES};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const REPLY_TIMEOUT: Duration = Duration::from_millis(2500);

// ------------------------------------------------------------------- worker

enum Cmd {
    SetDpi(DpiTable),
    SetRate(u16),
    Refresh,
}

#[derive(Default, Clone)]
struct Snapshot {
    path: String,
    wired: bool,
    info: Option<DeviceInfo>,
    dpi: Option<DpiTable>,
    rate: Option<u16>,
    /// Mouse is connected but resting: relayed reads fail, values may be stale.
    idle: bool,
}

enum Event {
    State(Snapshot),
    Disconnected(String),
    Error(String),
    Ok(String),
}

struct Worker {
    dev: Option<hid::Device>,
    snap: Snapshot,
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }

    fn sensor(&self) -> Sensor {
        self.snap.info.as_ref().map(|i| i.sensor).unwrap_or(Sensor(0x95))
    }

    fn transact(&mut self, req: &Report) -> std::io::Result<Report> {
        let dev = self.dev.as_mut().ok_or_else(|| std::io::Error::other("kein Gerät"))?;
        let r = dev.transact(req, REPLY_TIMEOUT);
        if let Err(e) = &r {
            use std::io::ErrorKind::*;
            if !matches!(e.kind(), TimedOut | NotConnected) {
                // Real I/O error: unplugged etc. Reopen on next poll.
                self.dev = None;
                self.snap = Snapshot::default();
                self.send(Event::Disconnected(e.to_string()));
            }
        }
        r
    }

    fn ensure_open(&mut self) -> bool {
        if self.dev.is_some() {
            return true;
        }
        match hid::Device::open() {
            Ok(d) => {
                self.snap = Snapshot {
                    path: d.path.display().to_string(),
                    wired: d.wired,
                    ..Default::default()
                };
                self.dev = Some(d);
                true
            }
            Err(e) => {
                self.send(Event::Disconnected(e.to_string()));
                false
            }
        }
    }

    fn refresh(&mut self) {
        if !self.ensure_open() {
            return;
        }
        match self.transact(&protocol::read_request(protocol::CMD_DEVICE_INFO)) {
            Ok(r) => self.snap.info = Some(protocol::parse_info(&r, self.snap.wired)),
            Err(_) => return,
        }
        let online = self.snap.info.as_ref().is_some_and(|i| i.mouse_online);
        self.snap.idle = true;
        // The receiver relays these to the mouse; a resting mouse doesn't answer
        // (~1.6 s each), so skip the second read when the first one fails.
        if online {
            if let Ok(r) = self.transact(&protocol::read_request(protocol::CMD_GET_DPI)) {
                self.snap.dpi = Some(protocol::parse_dpi(self.sensor(), &r));
                if let Ok(r) = self.transact(&protocol::read_request(protocol::CMD_GET_RATE)) {
                    self.snap.rate = protocol::parse_rate(&r);
                    self.snap.idle = false;
                }
            }
        }
        self.send(Event::State(self.snap.clone()));
    }

    fn write(&mut self, req: Report, what: &str) {
        if !self.ensure_open() {
            return;
        }
        // Retry a few times: a resting mouse answers once it moves again.
        let mut result = self.transact(&req);
        for _ in 0..3 {
            match &result {
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => result = self.transact(&req),
                _ => break,
            }
        }
        match result {
            Ok(_) => self.send(Event::Ok(format!("{what} gespeichert"))),
            Err(e) => self.send(Event::Error(format!("{what} nicht gespeichert: {e}"))),
        }
        self.refresh();
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        self.refresh();
        loop {
            match rx.recv_timeout(POLL_INTERVAL) {
                Ok(Cmd::SetDpi(t)) => {
                    let req = protocol::set_dpi_request(self.sensor(), &t);
                    self.write(req, "DPI");
                }
                Ok(Cmd::SetRate(hz)) => {
                    if let Some(req) = protocol::set_rate_request(hz) {
                        self.write(req, "Polling-Rate");
                    }
                }
                Ok(Cmd::Refresh) | Err(RecvTimeoutError::Timeout) => self.refresh(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

// ---------------------------------------------------------------------- GUI

struct App {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    snap: Snapshot,
    conn_error: Option<String>,
    /// Editable copy of the DPI table; only re-synced from the mouse when not dirty.
    edit: Option<DpiTable>,
    dirty: bool,
    message: Option<(String, bool)>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let worker = Worker { dev: None, snap: Snapshot::default(), tx: ev_tx, ctx: cc.egui_ctx.clone() };
        thread::spawn(move || worker.run(cmd_rx));
        App {
            tx: cmd_tx,
            rx: ev_rx,
            snap: Snapshot::default(),
            conn_error: None,
            edit: None,
            dirty: false,
            message: None,
        }
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::State(s) => {
                    self.conn_error = None;
                    if !self.dirty {
                        self.edit = s.dpi.clone();
                    }
                    self.snap = s;
                }
                Event::Disconnected(e) => {
                    self.conn_error = Some(e);
                    self.snap = Snapshot::default();
                }
                Event::Error(e) => self.message = Some((e, true)),
                Event::Ok(m) => self.message = Some((m, false)),
            }
        }
    }

    fn apply_dpi(&mut self) {
        if let Some(t) = &self.edit {
            let _ = self.tx.send(Cmd::SetDpi(t.clone()));
            self.dirty = false;
        }
    }

    fn status_ui(&self, ui: &mut egui::Ui) {
        if let Some(e) = &self.conn_error {
            ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {e}"));
            return;
        }
        let Some(info) = &self.snap.info else {
            ui.label("Verbinde …");
            return;
        };
        ui.horizontal_wrapped(|ui| {
            let conn = if self.snap.wired { "Kabel" } else { "2.4 GHz" };
            ui.label(format!("{} · {} · {}", info.id, conn, self.snap.path));
            ui.separator();
            let bat = if info.charging {
                format!("🔋 {} % (lädt)", info.battery)
            } else {
                format!("🔋 {} %", info.battery)
            };
            ui.label(bat);
            if !info.mouse_online {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, "Maus schläft – kurz bewegen");
            } else if self.snap.idle {
                ui.separator();
                ui.label(egui::RichText::new("Maus ruht – Werte evtl. nicht aktuell").weak());
            }
        });
    }

    fn current_dpi_ui(&self, ui: &mut egui::Ui) {
        let (big, small) = match &self.snap.dpi {
            Some(t) => (
                format!("{} DPI", t.dpi[t.active]),
                format!("Stufe {} von {}", t.active + 1, t.count),
            ),
            None => ("– DPI".into(), String::new()),
        };
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(big).size(40.0).strong());
            ui.label(small);
        });
    }

    fn stages_ui(&mut self, ui: &mut egui::Ui) {
        let sensor = self.snap.info.as_ref().map(|i| i.sensor).unwrap_or(Sensor(0x95));
        let (min, max, step) = sensor.range();
        let Some(t) = self.edit.as_mut() else {
            ui.label("Keine Daten – Maus bewegen, damit sie aufwacht.");
            return;
        };
        let mut changed = false;
        let mut select_now = None;

        ui.horizontal(|ui| {
            ui.label("Anzahl Stufen:");
            for n in 1..=STAGES {
                if ui.selectable_label(t.count == n, n.to_string()).clicked() {
                    t.count = n;
                    t.active = t.active.min(n - 1);
                    changed = true;
                }
            }
        });
        ui.add_space(4.0);

        egui::Grid::new("stages").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for i in 0..STAGES {
                let enabled = i < t.count;
                ui.add_enabled_ui(enabled, |ui| {
                    if ui
                        .radio(t.active == i, format!("Stufe {}", i + 1))
                        .on_hover_text("Als aktive DPI setzen (sofort)")
                        .clicked()
                    {
                        select_now = Some(i);
                    }
                });
                let r = ui.add_enabled(
                    enabled,
                    egui::Slider::new(&mut t.dpi[i], min..=max)
                        .step_by(step as f64)
                        .logarithmic(true)
                        .suffix(" DPI"),
                );
                changed |= r.changed();
                ui.end_row();
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "Bereich {min}–{max} DPI in {step}er-Schritten · Doppelklick auf die Zahl zum Eintippen"
            ))
            .small()
            .weak(),
        );

        if changed {
            self.dirty = true;
        }
        if let Some(i) = select_now {
            t.active = i;
            self.apply_dpi();
        }
    }

    fn rate_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (hz, _) in RATES {
                let sel = self.snap.rate == Some(hz);
                if ui.selectable_label(sel, format!("{hz} Hz")).clicked() && !sel {
                    let _ = self.tx.send(Cmd::SetRate(hz));
                    self.snap.rate = Some(hz); // optimistic, corrected on refresh
                }
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_events();
        let online = self.snap.info.as_ref().is_some_and(|i| i.mouse_online) && self.edit.is_some();

        egui::CentralPanel::default().show(ui, |ui| {
            self.status_ui(ui);
            ui.separator();
            self.current_dpi_ui(ui);
            ui.separator();

            ui.add_enabled_ui(online, |ui| {
                ui.heading("DPI-Stufen");
                self.stages_ui(ui);
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.dirty, egui::Button::new("✔ Übernehmen")).clicked() {
                        self.apply_dpi();
                    }
                    if ui.add_enabled(self.dirty, egui::Button::new("↺ Verwerfen")).clicked() {
                        self.dirty = false;
                        self.edit = self.snap.dpi.clone();
                    }
                });
                ui.add_space(8.0);
                ui.heading("Polling-Rate");
                self.rate_ui(ui);
            });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("⟳ Neu laden").clicked() {
                    let _ = self.tx.send(Cmd::Refresh);
                }
                if let Some((m, err)) = &self.message {
                    let c = if *err { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
                    ui.colored_label(c, m);
                }
            });
        });
    }
}

/// Read-only diagnostic: print raw replies of the read commands.
fn probe() {
    let mut dev = match hid::Device::open() {
        Ok(d) => d,
        Err(e) => return eprintln!("{e}"),
    };
    println!("{} (wired: {})", dev.path.display(), dev.wired);
    let mut sensor = Sensor(0x95);
    for cmd in [protocol::CMD_DEVICE_INFO, protocol::CMD_GET_DPI, protocol::CMD_GET_RATE] {
        match dev.transact(&protocol::read_request(cmd), REPLY_TIMEOUT) {
            Ok(r) => {
                let hex: Vec<String> = r.iter().map(|b| format!("{b:02x}")).collect();
                println!("0x{cmd:02x}: {}", hex.join(" "));
                match cmd {
                    protocol::CMD_DEVICE_INFO => {
                        let i = protocol::parse_info(&r, dev.wired);
                        sensor = i.sensor;
                        println!("      {i:?}");
                    }
                    protocol::CMD_GET_DPI => println!("      {:?}", protocol::parse_dpi(sensor, &r)),
                    _ => println!("      {:?} Hz", protocol::parse_rate(&r)),
                }
            }
            Err(e) => println!("0x{cmd:02x}: {e}"),
        }
    }
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--probe") {
        probe();
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AJ159P DPI")
            .with_app_id("ajazz-dpi")
            .with_inner_size([520.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native("AJ159P DPI", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
