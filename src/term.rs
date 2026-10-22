//! `/dev/tty` access with libc only: window size, raw-mode queries, foreground color, graphics support.
//! Everything goes through `/dev/tty`, never stdin, because stdin may carry the document.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;

pub struct Tty(File);

#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    /// Cell size in pixels; zero when the terminal reports no pixel size (no graphics).
    pub cell_w: u32,
    pub cell_h: u32,
}

/// Primary device attributes request. Every terminal answers it, so appending it to a query that
/// a terminal may ignore turns "wait for the timeout" into "wait for the DA1 reply".
const DA1: &[u8] = b"\x1b[c";

impl Tty {
    pub fn open() -> Option<Tty> {
        File::options()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .ok()
            .map(Tty)
    }

    pub fn size(&self) -> Size {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(self.0.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } != 0
            || ws.ws_col == 0
            || ws.ws_row == 0
        {
            return Size::default();
        }
        Size {
            cols: ws.ws_col,
            rows: ws.ws_row,
            cell_w: u32::from(ws.ws_xpixel) / u32::from(ws.ws_col),
            cell_h: u32::from(ws.ws_ypixel) / u32::from(ws.ws_row),
        }
    }

    pub fn fg_color(&self) -> Option<[u8; 3]> {
        self.osc_color(10)
    }

    pub fn bg_color(&self) -> Option<[u8; 3]> {
        self.osc_color(11)
    }

    /// A terminal color from OSC `n` (10: foreground, 11: background), as 8-bit channels.
    fn osc_color(&self, n: u8) -> Option<[u8; 3]> {
        let reply = self.query(format!("\x1b]{n};?\x1b\\").as_bytes())?;
        let reply = String::from_utf8_lossy(&reply);
        let rgb = reply.split_once("rgb:")?.1;
        let end = rgb.find(['\x1b', '\x07']).unwrap_or(rgb.len());
        let mut channels = rgb[..end].split('/').map(|c| {
            let bits = 4 * c.len() as u32;
            let v = u32::from_str_radix(c, 16).ok()?;
            Some((v >> bits.checked_sub(8)?) as u8)
        });
        Some([channels.next()??, channels.next()??, channels.next()??])
    }

    /// Whether the terminal answers a kitty graphics query.
    pub fn supports_graphics(&self) -> bool {
        self.query(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\")
            .is_some_and(|r| r.windows(8).any(|w| w == b"i=31;OK\x1b"))
    }

    /// Raw-mode round trip: write `request` followed by DA1, read until the DA1 reply arrives or
    /// the terminal stays silent for a second
    /// (a busy kitty or a slow SSH link can take longer than a few hundred ms to answer). Returns whatever arrived, if anything.
    fn query(&self, request: &[u8]) -> Option<Vec<u8>> {
        let _raw = Raw::enter(self.0.as_raw_fd())?;
        (&self.0).write_all(request).ok()?;
        (&self.0).write_all(DA1).ok()?;
        let mut reply = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match (&self.0).read(&mut buf) {
                Ok(0) => return (!reply.is_empty()).then_some(reply),
                Ok(n) => reply.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
            // DA1 reply: ESC [ ? <digits and ;> c. Once "ESC[?" is in, the next 'c' ends it.
            if reply.ends_with(b"c") && reply.windows(3).any(|w| w == b"\x1b[?") {
                return Some(reply);
            }
        }
    }
}

/// Raw mode guard: no echo, no line buffering, 1 s read timeout; restored on drop.
struct Raw {
    fd: i32,
    saved: libc::termios,
}

impl Raw {
    fn enter(fd: i32) -> Option<Raw> {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(libc::ECHO | libc::ICANON);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 10;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) } != 0 {
            return None;
        }
        Some(Raw { fd, saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved) };
    }
}
