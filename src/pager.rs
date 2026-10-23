//! viso's pager: the rendered lines on the alternate screen, keys from `/dev/tty`, and on a
//! resize the document laid out again at the new width, the place kept by block. Every key draws
//! a whole screen with cursor moves and erase-to-end-of-line, never a terminal scroll, so kitty's
//! placeholder images stay where their cells are.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::term::Tty;

/// One rendering of the document at one width.
pub struct Layout {
    pub lines: Vec<String>,
    /// First line of each top-level block, for keeping the place across a re-layout.
    pub starts: Vec<usize>,
    /// kitty image ids the layout transmitted, deleted before the next layout.
    pub ids: Vec<u32>,
}

/// Lays the document out at `cols` columns, image transfers into the writer.
pub type Render<'a> = dyn FnMut(u16, &mut dyn Write) -> io::Result<Layout> + 'a;

static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_: libc::c_int) {
    RESIZED.store(true, Ordering::SeqCst);
}

#[derive(Debug, PartialEq)]
enum Key {
    Char(char),
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

struct Pager<'a> {
    name: &'a str,
    layout: Layout,
    top: usize,
    rows: usize,
    query: String,
    /// The search prompt's text while it is open.
    edit: Option<String>,
    message: Option<String>,
}

/// Page the document on the alternate screen until `q`; the terminal is restored afterwards.
pub fn run(tty: &Tty, name: &str, render: &mut Render) -> io::Result<()> {
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            on_winch as extern "C" fn(libc::c_int) as usize,
        )
    };
    let _raw = tty
        .raw_keys()
        .ok_or_else(|| io::Error::other("cannot put /dev/tty in raw mode"))?;
    let out = &tty.0;
    // alternate screen, cursor hidden, no auto-wrap: a line that is somehow too long is clipped
    (&*out).write_all(b"\x1b[?1049h\x1b[?25l\x1b[?7l")?;
    let result = Pager::new(tty, name, render).and_then(|mut p| p.main_loop(tty, render));
    (&*out).write_all(b"\x1b[?7h\x1b[?25h\x1b[?1049l")?;
    result
}

impl<'a> Pager<'a> {
    fn new(tty: &Tty, name: &'a str, render: &mut Render) -> io::Result<Self> {
        let size = tty.size();
        let mut sink = &tty.0;
        Ok(Pager {
            name,
            layout: render(size.cols, &mut sink)?,
            top: 0,
            rows: rows(size.rows),
            query: String::new(),
            edit: None,
            message: None,
        })
    }

    fn main_loop(&mut self, tty: &Tty, render: &mut Render) -> io::Result<()> {
        let mut keys = Vec::new();
        loop {
            if RESIZED.swap(false, Ordering::SeqCst) {
                self.relayout(tty, render)?;
            }
            self.draw(tty)?;
            keys.clear();
            read_keys(tty, &mut keys)?;
            for key in keys.drain(..) {
                if self.edit.is_some() {
                    self.edit_key(key);
                } else if !self.key(key) {
                    return Ok(());
                }
            }
        }
    }

    /// A key outside the search prompt; false means quit.
    fn key(&mut self, key: Key) -> bool {
        let page = self.rows.saturating_sub(1).max(1) as isize;
        self.message = None;
        match key {
            Key::Char('q' | '\x03') => return false,
            Key::Char('j' | '\r' | '\n' | 'e' | '\x05') | Key::Down => self.scroll(1),
            Key::Char('k' | 'y' | '\x19') | Key::Up => self.scroll(-1),
            Key::Char(' ' | 'f' | '\x06') | Key::PageDown => self.scroll(page),
            Key::Char('b' | '\x02') | Key::PageUp => self.scroll(-page),
            Key::Char('d' | '\x04') => self.scroll(page / 2),
            Key::Char('u' | '\x15') => self.scroll(-page / 2),
            Key::Char('g') | Key::Home => self.top = 0,
            Key::Char('G') | Key::End => self.top = self.max_top(),
            Key::Char('/') => self.edit = Some(String::new()),
            Key::Char('n') => self.search(self.top + 1, true),
            Key::Char('N') => self.search(self.top, false),
            _ => {}
        }
        true
    }

    fn edit_key(&mut self, key: Key) {
        let Some(edit) = &mut self.edit else { return };
        match key {
            Key::Char('\r' | '\n') => {
                self.query = self.edit.take().unwrap_or_default();
                self.search(self.top + 1, true);
            }
            Key::Char('\x1b' | '\x03') => self.edit = None,
            Key::Char('\x7f' | '\x08') => {
                edit.pop();
            }
            Key::Char(c) if !c.is_control() => edit.push(c),
            _ => {}
        }
    }

    fn max_top(&self) -> usize {
        self.layout
            .lines
            .len()
            .saturating_sub(self.rows.saturating_sub(1).max(1))
    }

    fn scroll(&mut self, by: isize) {
        self.top = self.top.saturating_add_signed(by).min(self.max_top());
    }

    /// Case-insensitive substring search from `from`, forward or back, around the end; the hit
    /// becomes the top line.
    fn search(&mut self, from: usize, forward: bool) {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            return;
        }
        let n = self.layout.lines.len();
        let f = from.min(n);
        let hit = |i: usize| plain(&self.layout.lines[i]).to_lowercase().contains(&q);
        let found = if forward {
            (f..n).chain(0..f).find(|&i| hit(i))
        } else {
            (0..f).rev().chain((f..n).rev()).find(|&i| hit(i))
        };
        match found {
            Some(i) => self.top = i.min(self.max_top()),
            None => self.message = Some(format!("{} not found", self.query)),
        }
    }

    /// After the window changed: wait for the size to settle, drop the old images, lay the
    /// document out again and return to the same block.
    fn relayout(&mut self, tty: &Tty, render: &mut Render) -> io::Result<()> {
        let mut size = tty.size();
        loop {
            std::thread::sleep(Duration::from_millis(80));
            let now = tty.size();
            if now == size {
                break;
            }
            size = now;
        }
        RESIZED.store(false, Ordering::SeqCst);
        let mut sink = &tty.0;
        for id in &self.layout.ids {
            write!(sink, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")?;
        }
        let (block, offset) = locate(&self.layout.starts, self.top);
        self.layout = render(size.cols, &mut sink)?;
        self.rows = rows(size.rows);
        self.top =
            remap(&self.layout.starts, self.layout.lines.len(), block, offset).min(self.max_top());
        Ok(())
    }

    fn draw(&self, tty: &Tty) -> io::Result<()> {
        let content = self.rows.saturating_sub(1).max(1);
        let mut s = String::from("\x1b[H");
        for r in 0..content {
            s.push_str("\x1b[K");
            if let Some(l) = self.layout.lines.get(self.top + r) {
                s.push_str(l);
            }
            s.push_str("\r\n");
        }
        s.push_str("\x1b[K");
        match &self.edit {
            Some(q) => {
                s.push('/');
                s.push_str(q);
                s.push_str("\x1b[?25h");
            }
            None => {
                let n = self.layout.lines.len().max(1);
                let pct = (self.top + content).min(n) * 100 / n;
                s.push_str(&format!("\x1b[?25l\x1b[7m {} {pct:>3}% \x1b[0m", self.name));
                if let Some(m) = &self.message {
                    s.push(' ');
                    s.push_str(m);
                }
            }
        }
        (&tty.0).write_all(s.as_bytes())
    }
}

fn rows(reported: u16) -> usize {
    if reported == 0 {
        24
    } else {
        usize::from(reported)
    }
}

/// Keys from one read; raw mode returns every half second with nothing, so a resize is noticed.
fn read_keys(tty: &Tty, keys: &mut Vec<Key>) -> io::Result<()> {
    let mut buf = [0u8; 256];
    match (&tty.0).read(&mut buf) {
        Ok(n) => parse_keys(&buf[..n], keys),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Plain characters, and the cursor, page and home/end keys in their CSI and SS3 forms.
fn parse_keys(bytes: &[u8], keys: &mut Vec<Key>) {
    let text = String::from_utf8_lossy(bytes);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' || !matches!(it.peek(), Some('[' | 'O')) {
            keys.push(Key::Char(c));
            continue;
        }
        let csi = it.next() == Some('[');
        let mut params = String::new();
        let fin = loop {
            match it.next() {
                Some(ch) if !csi || ('\x40'..='\x7e').contains(&ch) => break Some(ch),
                Some(ch) => params.push(ch),
                None => break None,
            }
        };
        let key = match (fin, params.as_str()) {
            (Some('A'), _) => Key::Up,
            (Some('B'), _) => Key::Down,
            (Some('H'), _) | (Some('~'), "1" | "7") => Key::Home,
            (Some('F'), _) | (Some('~'), "4" | "8") => Key::End,
            (Some('~'), "5") => Key::PageUp,
            (Some('~'), "6") => Key::PageDown,
            _ => continue,
        };
        keys.push(key);
    }
}

/// The text of a rendered line: SGR and OSC sequences and placeholder cells removed.
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']' | '_') => {
                    let mut prev = ' ';
                    for ch in chars.by_ref() {
                        if ch == '\x07' || (prev == '\x1b' && ch == '\\') {
                            break;
                        }
                        prev = ch;
                    }
                }
                _ => {}
            },
            // a placeholder cell is the character and its two diacritics
            '\u{10EEEE}' => {
                chars.next();
                chars.next();
            }
            c => out.push(c),
        }
    }
    out
}

/// The block holding line `top`, and how far into it.
fn locate(starts: &[usize], top: usize) -> (usize, usize) {
    let block = starts.partition_point(|&s| s <= top).saturating_sub(1);
    (
        block,
        top.saturating_sub(starts.get(block).copied().unwrap_or(0)),
    )
}

/// The line `offset` into `block` in a new layout, at most the block's last line.
fn remap(starts: &[usize], lines: usize, block: usize, offset: usize) -> usize {
    let start = starts.get(block).copied().unwrap_or(0);
    let end = starts.get(block + 1).copied().unwrap_or(lines);
    start + offset.min(end.saturating_sub(start).saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_plain_csi_and_ss3() {
        let mut keys = Vec::new();
        parse_keys(b"j\x1b[B\x1b[5~\x1b[F\x1bOA\x1b[1;5H/q", &mut keys);
        assert_eq!(
            keys,
            [
                Key::Char('j'),
                Key::Down,
                Key::PageUp,
                Key::End,
                Key::Up,
                Key::Home,
                Key::Char('/'),
                Key::Char('q')
            ]
        );
    }

    #[test]
    fn plain_text_drops_escapes_and_placeholders() {
        let line = "\x1b[4mab\x1b[0m \x1b]8;id=1;http://u\x1b\\x\x1b]8;;\x1b\\ \x1b[38;2;1;2;3m\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{305}\u{30D}\x1b[39m z";
        assert_eq!(plain(line), "ab x  z");
    }

    #[test]
    fn search_ignores_case_and_escapes_and_wraps_around() {
        let mut p = Pager {
            name: "x",
            layout: Layout {
                lines: vec!["one".into(), "two".into(), "\x1b[1mThree\x1b[0m".into()],
                starts: vec![0, 1, 2],
                ids: Vec::new(),
            },
            top: 2,
            rows: 3,
            query: "one".into(),
            edit: None,
            message: None,
        };
        p.search(p.top + 1, true); // nothing below: around to the first line
        assert_eq!(p.top, 0);
        p.query = "three".into();
        p.search(p.top + 1, true); // found on line 2, shown as far down as the screen allows
        assert_eq!(p.top, 1);
        p.search(p.top, false); // backward from line 1: nothing above, around to the end
        assert_eq!((p.top, p.message.is_none()), (1, true));
        p.query = "zzz".into();
        p.search(0, true);
        assert_eq!(p.message.as_deref(), Some("zzz not found"));
    }

    #[test]
    fn the_place_survives_a_relayout() {
        // three blocks starting at 0, 3 and 7; line 5 is the third line of block 1
        assert_eq!(locate(&[0, 3, 7], 5), (1, 2));
        assert_eq!(locate(&[0, 3, 7], 0), (0, 0));
        assert_eq!(locate(&[0, 3, 7], 9), (2, 2));
        // narrower: block 1 now spans lines 2..4, so the offset is capped at its last line
        assert_eq!(remap(&[0, 2, 4], 9, 1, 2), 3);
        assert_eq!(remap(&[0, 2, 4], 9, 2, 1), 5);
        assert_eq!(remap(&[0, 2, 4], 9, 2, 100), 8);
        assert_eq!(remap(&[], 0, 0, 0), 0);
    }
}
