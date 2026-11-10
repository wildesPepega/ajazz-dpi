//! Minimal hidraw transport: find the vendor interface and do request/reply.

use crate::protocol::{Report, REPORT_LEN};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// (vendor, product, wired)
const IDS: [(u32, u32, bool); 4] = [
    (0x249A, 0x5C2F, false), // 2.4 GHz receiver
    (0x248A, 0x5C2E, true),  // USB cable
    (0x248A, 0x5D2E, true),
    (0x248A, 0x5E2E, true),
];

pub struct Device {
    file: File,
    pub path: PathBuf,
    pub wired: bool,
}

#[derive(Debug)]
pub enum FindError {
    NotFound,
    Permission(PathBuf),
    Io(io::Error),
}

impl std::fmt::Display for FindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindError::NotFound => write!(f, "Keine AJ159P gefunden (Receiver/Kabel angesteckt?)"),
            FindError::Permission(p) => write!(
                f,
                "Keine Berechtigung für {} – udev-Regel installieren (siehe README)",
                p.display()
            ),
            FindError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Parse `HID_ID=0003:0000249A:00005C2F` from a hidraw uevent.
fn hid_id(uevent: &str) -> Option<(u32, u32)> {
    let v = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
    let mut it = v.split(':').skip(1);
    let vid = u32::from_str_radix(it.next()?, 16).ok()?;
    let pid = u32::from_str_radix(it.next()?, 16).ok()?;
    Some((vid, pid))
}

impl Device {
    pub fn open() -> Result<Device, FindError> {
        let mut perm_err = None;
        let entries = fs::read_dir("/sys/class/hidraw").map_err(FindError::Io)?;
        let mut names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        names.sort();
        for name in names {
            let sys = PathBuf::from("/sys/class/hidraw").join(&name).join("device");
            let Ok(uevent) = fs::read_to_string(sys.join("uevent")) else { continue };
            let Some((vid, pid)) = hid_id(&uevent) else { continue };
            let Some(&(_, _, wired)) = IDS.iter().find(|(v, p, _)| *v == vid && *p == pid) else {
                continue;
            };
            // The config interface is the one with 32-byte vendor in/out reports
            // (interface 2). Recognise it by "input2" in HID_PHYS.
            if !uevent.lines().any(|l| l.starts_with("HID_PHYS=") && l.ends_with("/input2")) {
                continue;
            }
            let path = PathBuf::from("/dev").join(&name);
            match OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(file) => return Ok(Device { file, path, wired }),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => perm_err = Some(path),
                Err(e) => return Err(FindError::Io(e)),
            }
        }
        Err(perm_err.map(FindError::Permission).unwrap_or(FindError::NotFound))
    }

    fn write(&mut self, r: &Report) -> io::Result<()> {
        // hidraw: first byte is the report id, 0 for devices without ids.
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(r);
        self.file.write_all(&buf)
    }

    /// Read one report, waiting at most `timeout`. Ok(None) on timeout.
    fn read(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: valid pollfd pointer, count 1.
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(None);
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Gerät getrennt"));
        }
        let mut buf = [0u8; 64];
        match self.file.read(&mut buf) {
            Ok(n) if n >= REPORT_LEN => Ok(Some(buf[..REPORT_LEN].try_into().unwrap())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Drop queued reports (stale replies, status pushes).
    fn drain(&mut self) -> io::Result<()> {
        while self.read(Duration::ZERO)?.is_some() {}
        Ok(())
    }

    /// Send a request and wait for the reply echoing the command byte.
    /// Returns Err(TimedOut) if the mouse doesn't answer (asleep / out of range).
    pub fn transact(&mut self, req: &Report, timeout: Duration) -> io::Result<Report> {
        self.drain()?;
        self.write(req)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Maus antwortet nicht"));
            }
            let Some(r) = self.read(left)? else { continue };
            if r[0] != req[0] {
                continue; // e.g. a C0 status push
            }
            match r[1] {
                0x00 => return Ok(r),
                0x01 => return Err(io::Error::new(io::ErrorKind::TimedOut, "Maus ruht – bitte kurz bewegen")),
                0x02 => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Maus schläft / nicht verbunden – kurz bewegen",
                    ));
                }
                s => return Err(io::Error::other(format!("Fehlerstatus 0x{s:02x}"))),
            }
        }
    }
}
