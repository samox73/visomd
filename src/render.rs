//! Block tree → styled, wrapped terminal text, with math as kitty images or raw LaTeX.

use std::io::{self, Write};
use std::path::PathBuf;

use syntect::highlighting::{Style, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::md::{Alignment, Block, Inline};
use crate::{image, kitty, math};

const DEFAULT_THEME: &str = "base16-ocean.dark";
const SGR_HEADING: &str = "1;34";
const SGR_CODE: &str = "38;5;180";
const SGR_CODE_LIGHT: &str = "38;5;130";

/// Terminal facts needed to place images. Absent → math prints as LaTeX.
#[derive(Clone, Copy)]
pub struct Images {
    pub cell_w: u32,
    pub cell_h: u32,
    pub fg: [u8; 3],
    /// Display math size relative to inline math.
    pub display_scale: f64,
}

pub struct Renderer<'a, W: Write> {
    out: W,
    /// Where image transfers go, flushed per image; `None`: into `out`, ahead of the placeholder line.
    pub transfers: Option<&'a mut dyn Write>,
    /// OSC 8 hyperlinks around link text instead of the URL in dim after it.
    pub hyperlinks: bool,
    /// Light terminal background: a darker inline-code color.
    pub light: bool,
    /// Directory that relative image paths resolve against.
    pub base: PathBuf,
    width: usize,
    images: Option<Images>,
    theme: String,
    ids: kitty::Ids,
    links: u32,
    highlighter: Option<(SyntaxSet, Theme)>,
    /// Line prefix per enclosing list item or quote: what the first line gets, then the rest.
    segs: Vec<Seg>,
}

struct Seg {
    first: String,
    rest: String,
    used: bool,
}

enum Tok {
    /// Styled text and its width in columns; never split.
    Word(String, usize),
    Space,
    Break,
    /// Placeholder cells of one row, plus the image transfer that must precede the line.
    Math {
        cols: usize,
        transfer: String,
        cells: String,
    },
}

#[derive(Default)]
struct Line {
    text: String,
    cols: usize,
    transfers: String,
}

impl<'a, W: Write> Renderer<'a, W> {
    pub fn new(out: W, width: usize, images: Option<Images>, theme: &str) -> Self {
        Renderer {
            out,
            transfers: None,
            hyperlinks: false,
            light: false,
            base: PathBuf::new(),
            width,
            images,
            theme: theme.to_string(),
            ids: kitty::Ids::new(),
            links: 0,
            highlighter: None,
            segs: Vec::new(),
        }
    }

    /// An image transfer: to the transfer sink if there is one, else ahead of the text.
    fn transfer(&mut self, data: &str) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        match &mut self.transfers {
            Some(sink) => {
                sink.write_all(data.as_bytes())?;
                sink.flush()
            }
            None => self.out.write_all(data.as_bytes()),
        }
    }

    pub fn render(&mut self, blocks: &[Block]) -> io::Result<()> {
        self.blocks(blocks, false)?;
        self.out.flush()
    }

    /// Blocks separated by blank lines; in a tight context a list hugs the paragraph before it.
    fn blocks(&mut self, bs: &[Block], tight: bool) -> io::Result<()> {
        for (i, b) in bs.iter().enumerate() {
            let hug = tight
                && i > 0
                && matches!(b, Block::List { .. })
                && matches!(bs[i - 1], Block::Paragraph(_));
            if i > 0 && !hug {
                self.line("")?;
            }
            self.block(b)?;
        }
        Ok(())
    }

    fn block(&mut self, b: &Block) -> io::Result<()> {
        match b {
            Block::Paragraph(inl) => self.paragraph(inl, ""),
            Block::Heading(level, inl) => {
                self.paragraph(inl, if *level <= 2 { SGR_HEADING } else { "1" })
            }
            Block::Rule => {
                let w = self.avail();
                self.line(&"─".repeat(w))
            }
            Block::Quote(bs) => {
                self.segs.push(Seg {
                    first: "│ ".into(),
                    rest: "│ ".into(),
                    used: false,
                });
                let r = self.blocks(bs, false);
                self.segs.pop();
                r
            }
            Block::List { ordered, items } => {
                for (i, item) in items.iter().enumerate() {
                    let bullet = match ordered {
                        Some(n) => format!("{}. ", n + i as u64),
                        None => "• ".into(),
                    };
                    let rest = " ".repeat(bullet.chars().count());
                    self.segs.push(Seg {
                        first: bullet,
                        rest,
                        used: false,
                    });
                    self.blocks(item, true)?;
                    if self.segs.last().is_some_and(|s| !s.used) {
                        self.line("")?; // empty item: still show its bullet
                    }
                    self.segs.pop();
                }
                Ok(())
            }
            Block::Code { lang, text } => self.code(lang, text),
            Block::DisplayMath(tex) => self.display(tex),
            Block::Image { alt, url } => self.image(alt, url),
            Block::Table { align, head, rows } => self.table(align, head, rows),
        }
    }

    /// Columns padded to the widest cell, header bold, `│` between columns, `┼` in the rule. A
    /// table wider than the available width has its widest columns narrowed, down to their
    /// longest word, and the cells wrap; rows are then separated by rules too.
    fn table(
        &mut self,
        align: &[Alignment],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
    ) -> io::Result<()> {
        let has_head = !head.is_empty();
        let mut table: Vec<Vec<Vec<Tok>>> = Vec::new();
        let all = std::iter::once((head, "1")).chain(rows.iter().map(|r| (r.as_slice(), "")));
        for (row, sgr) in all.filter(|(row, _)| !row.is_empty()) {
            table.push(
                row.iter()
                    .map(|cell| {
                        let mut toks = Vec::new();
                        self.tokens(cell, sgr, &mut toks);
                        toks
                    })
                    .collect(),
            );
        }
        let n = align
            .len()
            .max(table.iter().map(Vec::len).max().unwrap_or(0));
        if n == 0 {
            return Ok(());
        }
        // natural width of each column, and the floor it can be narrowed to
        let (mut widths, mut floors) = (vec![0; n], vec![0; n]);
        for row in &table {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(wrap(cell, usize::MAX)[0].cols);
                let longest = cell
                    .iter()
                    .map(|t| match t {
                        Tok::Word(_, n) | Tok::Math { cols: n, .. } => *n,
                        _ => 0,
                    })
                    .max();
                floors[i] = floors[i].max(longest.unwrap_or(0));
            }
        }
        let budget = self.avail().saturating_sub(3 * (n - 1));
        while widths.iter().sum::<usize>() > budget {
            let shrinkable = widths.iter().enumerate().filter(|&(i, &w)| w > floors[i]);
            let Some((i, _)) = shrinkable.max_by_key(|&(_, &w)| w) else {
                break;
            };
            widths[i] -= 1;
        }
        let rule = widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┼─");
        let table: Vec<Vec<Vec<Line>>> = table
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, c)| wrap(c, widths[i]))
                    .collect()
            })
            .collect();
        // once any row wraps, rows need a rule between them to be told apart
        let grid = table.iter().flatten().any(|cell| cell.len() > 1);
        for (r, cells) in table.iter().enumerate() {
            for line in cells.iter().flatten() {
                self.transfer(&line.transfers)?;
            }
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for k in 0..height {
                let mut text = String::new();
                for (i, &w) in widths.iter().enumerate() {
                    if i > 0 {
                        text.push_str(" │ ");
                    }
                    let line = cells.get(i).and_then(|c| c.get(k));
                    let (cell, cols) = line.map_or(("", 0), |l| (l.text.as_str(), l.cols));
                    let pad = w.saturating_sub(cols);
                    let (left, right) = match align.get(i) {
                        Some(Alignment::Right) => (pad, 0),
                        Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    text.extend(std::iter::repeat_n(' ', left));
                    text.push_str(cell);
                    text.extend(std::iter::repeat_n(' ', right));
                }
                self.line(&text)?;
            }
            if (r == 0 && has_head) || (grid && r + 1 < table.len()) {
                self.line(&rule)?;
            }
        }
        Ok(())
    }

    fn paragraph(&mut self, inl: &[Inline], sgr: &str) -> io::Result<()> {
        let mut toks = Vec::new();
        self.tokens(inl, sgr, &mut toks);
        self.flow(&toks)
    }

    /// Wrapped lines of `toks`, each preceded by the transfers its images need.
    fn flow(&mut self, toks: &[Tok]) -> io::Result<()> {
        for line in wrap(toks, self.avail()) {
            self.transfer(&line.transfers)?;
            self.line(&line.text)?;
        }
        Ok(())
    }

    fn tokens(&mut self, inl: &[Inline], sgr: &str, toks: &mut Vec<Tok>) {
        let code = if self.light { SGR_CODE_LIGHT } else { SGR_CODE };
        for i in inl {
            match i {
                Inline::Text(t) => words(t, sgr, toks),
                Inline::Code(t) => words(t, &join(sgr, code), toks),
                Inline::Emph(v) => self.tokens(v, &join(sgr, "3"), toks),
                Inline::Strong(v) => self.tokens(v, &join(sgr, "1"), toks),
                Inline::Link { text, url } | Inline::Image { alt: text, url } => {
                    self.link(text, url, sgr, toks)
                }
                Inline::Break => toks.push(Tok::Break),
                Inline::Math(tex) => toks.push(self.math(tex, sgr)),
            }
        }
    }

    /// Link text underlined. With hyperlinks on, every word of it is an OSC 8 target and all of
    /// them share one id, so a link wrapped over two lines is one hover target; otherwise the URL
    /// follows in dim.
    fn link(&mut self, text: &[Inline], url: &str, sgr: &str, toks: &mut Vec<Tok>) {
        let start = toks.len();
        self.tokens(text, &join(sgr, "4"), toks);
        if !self.hyperlinks {
            toks.push(Tok::Space);
            words(&format!("({url})"), &join(sgr, "2"), toks);
            return;
        }
        self.links += 1;
        for t in &mut toks[start..] {
            if let Tok::Word(s, _) = t {
                *s = format!("\x1b]8;id={};{url}\x1b\\{s}\x1b]8;;\x1b\\", self.links);
            }
        }
    }

    /// A block image as a kitty placement scaled to the width; without images, or when the file
    /// is not a readable PNG or JPEG, one line of the alt text with the path in dim.
    fn image(&mut self, alt: &[Inline], url: &str) -> io::Result<()> {
        if let Some(img) = self.images {
            if let Some(im) = image::load(&self.base.join(url)) {
                let (cols, rows) = image::fit(im.w, im.h, img.cell_w, img.cell_h, self.avail());
                let id = self.ids.next();
                self.transfer(&kitty::transmit(id, &im.png, cols, rows))?;
                for row in kitty::placeholder(id, cols, rows) {
                    self.line(&row)?;
                }
                return Ok(());
            }
            eprintln!("viso: cannot show {url}, printing its alt text and path");
        }
        let mut toks = Vec::new();
        self.tokens(alt, "4", &mut toks);
        toks.push(Tok::Space);
        words(&format!("({url})"), "2", &mut toks);
        self.flow(&toks)
    }

    fn math(&mut self, tex: &str, sgr: &str) -> Tok {
        if let Some(img) = self.images {
            if let Some(c) = math::inline(tex, img.cell_w, img.cell_h, img.fg) {
                let cols = (c.w / img.cell_w) as usize;
                if cols <= kitty::MAX_CELLS {
                    let id = self.ids.next();
                    return Tok::Math {
                        cols,
                        transfer: kitty::transmit(id, &c.png(), cols as u16, 1),
                        cells: kitty::placeholder(id, cols as u16, 1).remove(0),
                    };
                }
            }
            eprintln!("viso: cannot render ${tex}$, printing the LaTeX");
        }
        styled(
            &format!("${}$", tex.split_whitespace().collect::<Vec<_>>().join(" ")),
            sgr,
        )
    }

    fn display(&mut self, tex: &str) -> io::Result<()> {
        if let Some(img) = self.images {
            let avail = self.avail().min(kitty::MAX_CELLS) as u32;
            if let Some(c) = math::display(
                tex,
                img.cell_w,
                img.cell_h,
                avail * img.cell_w,
                img.display_scale,
                img.fg,
            ) {
                let (cols, rows) = ((c.w / img.cell_w) as u16, (c.h / img.cell_h) as u16);
                let id = self.ids.next();
                self.transfer(&kitty::transmit(id, &c.png(), cols, rows))?;
                for row in kitty::placeholder(id, cols, rows) {
                    self.line(&row)?;
                }
                return Ok(());
            }
            eprintln!("viso: cannot render $${tex}$$, printing the LaTeX");
        }
        self.paragraph(&[Inline::Text(format!("$$ {} $$", tex.trim()))], "")
    }

    fn code(&mut self, lang: &str, text: &str) -> io::Result<()> {
        use syntect::easy::HighlightLines;
        use syntect::util::{LinesWithEndings, as_24_bit_terminal_escaped};
        let lines: Vec<String> = {
            let (ss, theme) = self
                .highlighter
                .get_or_insert_with(|| load_highlighter(&self.theme));
            let syntax = ss
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| ss.find_syntax_plain_text());
            let mut h = HighlightLines::new(syntax, theme);
            LinesWithEndings::from(text)
                .map(|l| {
                    let regions = h
                        .highlight_line(l, ss)
                        .unwrap_or_else(|_| vec![(Style::default(), l)]);
                    format!(
                        "  {}\x1b[0m",
                        as_24_bit_terminal_escaped(&regions, false).trim_end()
                    )
                })
                .collect()
        };
        for l in lines {
            self.line(&l)?;
        }
        Ok(())
    }

    /// Columns left for content after the prefixes.
    fn avail(&self) -> usize {
        let prefix: usize = self.segs.iter().map(|s| s.rest.chars().count()).sum();
        self.width.saturating_sub(prefix).max(1)
    }

    fn prefix(&mut self) -> String {
        self.segs
            .iter_mut()
            .map(|s| {
                if s.used {
                    s.rest.clone()
                } else {
                    s.used = true;
                    s.first.clone()
                }
            })
            .collect()
    }

    fn line(&mut self, text: &str) -> io::Result<()> {
        let p = self.prefix();
        writeln!(self.out, "{}", format!("{p}{text}").trim_end())
    }
}

fn load_highlighter(name: &str) -> (SyntaxSet, Theme) {
    let mut ts = ThemeSet::load_defaults();
    let theme = ts.themes.remove(name).unwrap_or_else(|| {
        eprintln!("viso: unknown theme {name}, using {DEFAULT_THEME}");
        ts.themes.remove(DEFAULT_THEME).expect("bundled theme")
    });
    (SyntaxSet::load_defaults_newlines(), theme)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a};{b}")
    }
}

fn styled(word: &str, sgr: &str) -> Tok {
    let n = word.chars().count();
    if sgr.is_empty() {
        Tok::Word(word.to_string(), n)
    } else {
        Tok::Word(format!("\x1b[{sgr}m{word}\x1b[0m"), n)
    }
}

/// Split `text` into words and (collapsed) spaces.
fn words(text: &str, sgr: &str, toks: &mut Vec<Tok>) {
    let mut word = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !word.is_empty() {
                toks.push(styled(&word, sgr));
                word.clear();
            }
            if !matches!(toks.last(), Some(Tok::Space)) {
                toks.push(Tok::Space);
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        toks.push(styled(&word, sgr));
    }
}

/// Greedy word wrap. A token wider than `width` overflows on a line of its own.
fn wrap(toks: &[Tok], width: usize) -> Vec<Line> {
    let mut lines = vec![Line::default()];
    let mut space = false;
    for t in toks {
        let (s, n, transfer) = match t {
            Tok::Space => {
                space = true;
                continue;
            }
            Tok::Break => {
                lines.push(Line::default());
                space = false;
                continue;
            }
            Tok::Word(s, n) => (s.clone(), *n, String::new()),
            Tok::Math {
                cols,
                transfer,
                cells,
            } => (cells.clone(), *cols, transfer.clone()),
        };
        let cur = lines.last().unwrap();
        if cur.cols > 0 && cur.cols + usize::from(space) + n > width {
            lines.push(Line::default());
        }
        let cur = lines.last_mut().unwrap();
        if space && cur.cols > 0 {
            cur.text.push(' ');
            cur.cols += 1;
        }
        cur.text.push_str(&s);
        cur.cols += n;
        cur.transfers.push_str(&transfer);
        space = false;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(src: &str, width: usize) -> String {
        let mut out = Vec::new();
        Renderer::new(&mut out, width, None, DEFAULT_THEME)
            .render(&crate::md::parse(src))
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn wraps_and_keeps_prefixes() {
        assert_eq!(render("aaa bbb ccc ddd", 7), "aaa bbb\nccc ddd\n");
        assert_eq!(render("- aaa bbb ccc", 9), "• aaa bbb\n  ccc\n");
        assert_eq!(
            render("> - aaa bbb\n>   - ccc", 20),
            "│ • aaa bbb\n│   • ccc\n"
        );
        assert_eq!(render("1. a\n2. b", 20), "1. a\n2. b\n");
        assert_eq!(
            render("$x$ and $$a\nb$$ wrap", 5),
            "$x$\nand\n$a b$\nwrap\n"
        );
    }

    #[test]
    fn tables_align_columns() {
        let got = render(
            "| a | bb | ccc |\n|:--|:--:|--:|\n| 1 | 2 | 3 |\n| 10 | 20 | 30 |\n",
            40,
        );
        let bold = |s: &str| format!("\x1b[1m{s}\x1b[0m");
        let expected = format!(
            "{}  │ {} │ {}\n───┼────┼────\n1  │ 2  │   3\n10 │ 20 │  30\n",
            bold("a"),
            bold("bb"),
            bold("ccc")
        );
        assert_eq!(got, expected);
        // inside a quote, with math in a cell printed as LaTeX
        assert_eq!(
            render("> | x |\n> |---|\n> | $y$ |\n", 40),
            "│ \x1b[1mx\x1b[0m\n│ ───\n│ $y$\n"
        );
    }

    #[test]
    fn wide_tables_narrow_the_widest_column_and_wrap() {
        let got = render(
            "| k | description |\n|---|---|\n| 1 | one two three four five |\n",
            20,
        );
        let bold = |s: &str| format!("\x1b[1m{s}\x1b[0m");
        let rule = format!("──┼{}", "─".repeat(17));
        let expected = format!(
            "{} │ {}\n{rule}\n1 │ one two three\n  │ four five\n",
            bold("k"),
            bold("description")
        );
        assert_eq!(got, expected);
        // with two rows, a rule keeps the wrapped rows apart
        let got = render(
            "| k | description |\n|---|---|\n| 1 | one two three four five |\n| 2 | six |\n",
            20,
        );
        assert!(
            got.ends_with(&format!(
                "1 │ one two three\n  │ four five\n{rule}\n2 │ six\n"
            )),
            "{got}"
        );
        // a column is never narrower than its longest word, even if the table then overflows
        let got = render("| a | b |\n|---|---|\n| unbreakableword | x |\n", 8);
        assert!(got.contains("unbreakableword │ x"), "{got}");
    }

    #[test]
    fn images_are_transferred_before_their_line() {
        let img = Images {
            cell_w: 8,
            cell_h: 16,
            fg: [200, 200, 200],
            display_scale: 1.0,
        };
        let mut out = Vec::new();
        Renderer::new(&mut out, 40, Some(img), DEFAULT_THEME)
            .render(&crate::md::parse("a $x$ b\n\n> $$\\frac{a}{b}$$"))
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        let mut lines = s.lines();
        // inline: transfer, then the text line with one row of placeholder cells in it
        let l1 = lines.next().unwrap();
        assert!(l1.starts_with("\x1b_Ga=T,U=1,f=100,q=2,i="), "{l1}");
        let text = &l1[l1.rfind("\x1b\\").unwrap() + 2..];
        assert!(
            text.starts_with("a \x1b[38;2;") && text.ends_with("\x1b[39m b"),
            "{text}"
        );
        assert!(text.contains("\u{10EEEE}\u{305}\u{305}"), "{text}");
        assert_eq!(lines.next(), Some(""));
        // display: transfer, then placeholder rows carrying the quote prefix
        let l3 = lines.next().unwrap();
        assert!(l3.contains(",c=") && l3.contains(",r="), "{l3}");
        assert!(
            l3[l3.rfind("\x1b\\").unwrap() + 2..].starts_with("│ \x1b[38;2;"),
            "{l3}"
        );
        let rest: Vec<&str> = lines.collect();
        assert!(
            !rest.is_empty()
                && rest
                    .iter()
                    .all(|l| l.starts_with("│ \x1b[38;2;") && l.ends_with("\x1b[39m")),
            "{rest:?}"
        );
        // the display formula is taller than one row, and each row names its own index
        assert!(
            rest[0].contains("\u{10EEEE}\u{30D}\u{305}"),
            "{:?}",
            rest[0]
        );
    }

    #[test]
    fn hyperlinks_wrap_every_word_under_one_id() {
        let mut out = Vec::new();
        let mut r = Renderer::new(&mut out, 40, None, DEFAULT_THEME);
        r.hyperlinks = true;
        r.render(&crate::md::parse("[a b](u) [c](v)")).unwrap();
        let osc = |id: u8, url: &str, word: &str| {
            format!("\x1b]8;id={id};{url}\x1b\\\x1b[4m{word}\x1b[0m\x1b]8;;\x1b\\")
        };
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "{} {} {}\n",
                osc(1, "u", "a"),
                osc(1, "u", "b"),
                osc(2, "v", "c")
            )
        );
    }

    #[test]
    fn light_terminals_get_a_darker_code_color() {
        let mut out = Vec::new();
        let mut r = Renderer::new(&mut out, 40, None, DEFAULT_THEME);
        r.light = true;
        r.render(&crate::md::parse("`x`")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[38;5;130mx\x1b[0m\n");
    }

    #[test]
    fn transfers_go_to_their_own_sink_and_block_images_are_placed() {
        let img = Images {
            cell_w: 8,
            cell_h: 16,
            fg: [200, 200, 200],
            display_scale: 1.0,
        };
        let (mut text, mut sink) = (Vec::new(), Vec::new());
        let mut r = Renderer::new(&mut text, 40, Some(img), DEFAULT_THEME);
        r.transfers = Some(&mut sink);
        r.base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        r.render(&crate::md::parse(
            "a $x$ b\n\n$$y$$\n\n![t](tiny.png)\n\n![t](nope.png)\n\n![t](tiny.jpg) x",
        ))
        .unwrap();
        let (text, sink) = (
            String::from_utf8(text).unwrap(),
            String::from_utf8(sink).unwrap(),
        );
        assert!(!text.contains("\x1b_G"), "{text}");
        assert_eq!(sink.matches("\x1b_Ga=T,").count(), 3, "{sink}");
        assert!(sink.ends_with("\x1b\\"));
        let lines: Vec<&str> = text.lines().collect();
        // the 2×2 png: one cell; the missing file and the inline image fall back to links
        assert!(lines[4].contains("\u{10EEEE}\u{305}\u{305}") && lines[4].ends_with("\x1b[39m"));
        assert_eq!(lines[6], "\x1b[4mt\x1b[0m \x1b[2m(nope.png)\x1b[0m");
        assert_eq!(lines[8], "\x1b[4mt\x1b[0m \x1b[2m(tiny.jpg)\x1b[0m x");
    }

    #[test]
    fn snapshot_no_images_width_60() {
        let got = render(include_str!("../tests/fixture.md"), 60);
        let expected = include_str!("../tests/expected.txt");
        assert!(
            got == expected,
            "output differs from tests/expected.txt:\n{got}"
        );
    }
}
