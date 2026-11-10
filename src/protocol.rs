//! Vendor protocol of the Ajazz AJ159P (XCTECH receiver 249A:5C2F / cable 248A:5C2E).
//!
//! Every request and reply is a 32-byte report on interface 2 (no report IDs):
//!
//! ```text
//! [0]      command
//! [1]      status (0 = OK in replies, 0 in requests)
//! [2]      (chunk_index << 4) | chunk_count — 0x01 for writes, 0x00 for reads
//! [3]      payload length
//! [4..31]  payload, zero padded
//! [31]     checksum = sum([4..31]) & 0xFF
//! ```
//!
//! Sources: reverse engineering in github.com/Rockeyxx/aj179-linux (docs/PROTOCOL.md)
//! and github.com/Khabarovsk6000/Ajazz-AJ159P-Linux.

pub const REPORT_LEN: usize = 32;
pub type Report = [u8; REPORT_LEN];

pub const CMD_SET_RATE: u8 = 0x02;
pub const CMD_SET_DPI: u8 = 0x03;
pub const CMD_DEVICE_INFO: u8 = 0x10;
pub const CMD_GET_RATE: u8 = 0x12;
pub const CMD_GET_DPI: u8 = 0x13;

pub const STAGES: usize = 6;

fn checksum(r: &Report) -> u8 {
    r[4..31].iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

pub fn read_request(cmd: u8) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0] = cmd;
    r
}

fn write_request(cmd: u8, payload: &[u8]) -> Report {
    assert!(payload.len() <= 27);
    let mut r = [0u8; REPORT_LEN];
    r[0] = cmd;
    r[2] = 0x01;
    r[3] = payload.len() as u8;
    r[4..4 + payload.len()].copy_from_slice(payload);
    r[31] = checksum(&r);
    r
}

// ---------------------------------------------------------------- polling rate

/// Supported report rates in Hz, with their wire value (a bit flag).
pub const RATES: [(u16, u8); 4] = [(125, 0x08), (250, 0x04), (500, 0x02), (1000, 0x01)];

pub fn rate_to_wire(hz: u16) -> Option<u8> {
    RATES.iter().find(|(h, _)| *h == hz).map(|(_, w)| *w)
}

pub fn rate_from_wire(v: u8) -> Option<u16> {
    RATES.iter().find(|(_, w)| *w == v).map(|(h, _)| *h)
}

pub fn set_rate_request(hz: u16) -> Option<Report> {
    rate_to_wire(hz).map(|w| write_request(CMD_SET_RATE, &[w]))
}

pub fn parse_rate(r: &Report) -> Option<u16> {
    rate_from_wire(r[4])
}

// ------------------------------------------------------------------------ DPI

/// DPI encoding depends on the sensor id reported in device info byte 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sensor(pub u8);

impl Sensor {
    /// (min, max, step) in DPI as the vendor app uses it.
    pub fn range(self) -> (u32, u32, u32) {
        match self.0 {
            0x95 => (50, 26000, 50),
            0x96 => (50, 26000, 50), // > 26000 needs a coarser encoding, not offered
            0x11 => (50, 10000, 50),
            0x05 => (50, 6400, 50),
            0x35 => (100, 16000, 100),
            0x25 => (100, 5000, 100),
            0x26 => (200, 5000, 100),
            0x04 => (100, 8000, 100),
            0x10 | 0x12 => (100, 4800, 100),
            _ => (50, 26000, 50),
        }
    }

    pub fn encode(self, dpi: u32) -> u16 {
        let (min, max, step) = self.range();
        let dpi = (dpi.clamp(min, max) / step) * step;
        let raw = match self.0 {
            0x11 | 0x05 => dpi / 50,
            0x35 | 0x25 | 0x04 | 0x10 | 0x12 => dpi / 100,
            0x26 => dpi / 100 - 1,
            _ => dpi / 50 - 1, // 0x95 (PAW3395), 0x96, unknown
        };
        raw as u16
    }

    pub fn decode(self, raw: u16) -> u32 {
        let raw = raw as u32;
        match self.0 {
            0x11 | 0x05 => raw * 50,
            0x35 | 0x25 | 0x04 | 0x10 | 0x12 => raw * 100,
            0x26 => (raw + 1) * 100,
            _ => (raw + 1) * 50,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiTable {
    /// 0-based active stage.
    pub active: usize,
    /// Number of enabled stages (1..=6).
    pub count: usize,
    pub dpi: [u32; STAGES],
}

pub fn set_dpi_request(sensor: Sensor, t: &DpiTable) -> Report {
    let mut p = [0u8; 1 + STAGES * 4];
    p[0] = ((t.active as u8) << 4) | (t.count as u8 & 0x0F);
    for (i, d) in t.dpi.iter().enumerate() {
        let raw = sensor.encode(*d).to_le_bytes();
        p[1 + i * 4..1 + i * 4 + 2].copy_from_slice(&raw); // X
        p[3 + i * 4..3 + i * 4 + 2].copy_from_slice(&raw); // Y
    }
    let mut r = write_request(CMD_SET_DPI, &p);
    // The vendor app (and both verified reimplementations) send 0x25 here even
    // though the payload is 25 (0x19) bytes long. Replicate exactly.
    r[3] = 0x25;
    r
}

pub fn parse_dpi(sensor: Sensor, r: &Report) -> DpiTable {
    let mut dpi = [0u32; STAGES];
    for (i, d) in dpi.iter_mut().enumerate() {
        let o = 5 + i * 4;
        *d = sensor.decode(u16::from_le_bytes([r[o], r[o + 1]]));
    }
    let count = ((r[4] & 0x0F) as usize).clamp(1, STAGES);
    DpiTable { active: ((r[4] >> 4) as usize).min(count - 1), count, dpi }
}

// ---------------------------------------------------------------- device info

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub sensor: Sensor,
    pub charging: bool,
    pub battery: u8,
    /// false = mouse asleep / out of range (receiver answers, mouse doesn't).
    pub mouse_online: bool,
}

pub fn parse_info(r: &Report, wired: bool) -> DeviceInfo {
    DeviceInfo {
        id: r[4..8].iter().filter(|c| c.is_ascii_graphic()).map(|c| *c as char).collect(),
        sensor: Sensor(r[10]),
        charging: r[12] != 0,
        battery: r[13],
        // Over the cable byte 14 is 0 but the mouse answers everything.
        mouse_online: wired || r[14] != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_packet() {
        let r = set_rate_request(500).unwrap();
        assert_eq!(&r[..5], &[0x02, 0x00, 0x01, 0x01, 0x02]);
        assert_eq!(r[31], 0x02);
        assert!(set_rate_request(333).is_none());
    }

    #[test]
    fn paw3395_dpi_roundtrip() {
        let s = Sensor(0x95);
        for d in [50, 400, 800, 1600, 3200, 26000] {
            assert_eq!(s.decode(s.encode(d)), d);
        }
        assert_eq!(s.encode(400), 7);
        assert_eq!(s.encode(99999), 519);
    }

    #[test]
    fn dpi_packet_layout() {
        let s = Sensor(0x95);
        let t = DpiTable { active: 2, count: 6, dpi: [400, 800, 1200, 1600, 2400, 3200] };
        let r = set_dpi_request(s, &t);
        assert_eq!(&r[..5], &[0x03, 0x00, 0x01, 0x25, 0x26]);
        assert_eq!(&r[5..9], &[7, 0, 7, 0]);
        assert_eq!(r[31], checksum(&r));
        // A reply has the same payload layout as the write.
        assert_eq!(parse_dpi(s, &r), t);
    }

    #[test]
    fn info_parse() {
        let mut r = [0u8; 32];
        r[..15].copy_from_slice(&[
            0x10, 0, 1, 0x0b, b'M', b'1', b'7', b'9', 0, 0, 0x95, 1, 0, 0x51, 1,
        ]);
        let i = parse_info(&r, false);
        assert_eq!(i.id, "M179");
        assert_eq!(i.sensor, Sensor(0x95));
        assert_eq!(i.battery, 81);
        assert!(i.mouse_online);
    }
}
