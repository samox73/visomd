//! pulldown-cmark events → a block tree. One recursive walk; every `Start` is consumed by the
//! level that owns it, so any `End` seen at a level is that level's own.

pub use pulldown_cmark::Alignment;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::iter::Peekable;

#[derive(Debug, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    List {
        ordered: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Rule,
    DisplayMath(String),
    /// A paragraph that is one image and nothing else.
    Image {
        alt: Vec<Inline>,
        url: String,
    },
    Table {
        align: Vec<Alignment>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

#[derive(Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Code(String),
    Link {
        text: Vec<Inline>,
        url: String,
    },
    /// An image among other text; rendered like a link.
    Image {
        alt: Vec<Inline>,
        url: String,
    },
    Math(String),
    Break,
}

type Events<'a> = Peekable<Parser<'a>>;

pub fn parse(src: &str) -> Vec<Block> {
    blocks(&mut Parser::new_ext(src, Options::ENABLE_MATH | Options::ENABLE_TABLES).peekable())
}

fn is_inline(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn blocks(ev: &mut Events) -> Vec<Block> {
    let mut out = Vec::new();
    loop {
        match ev.peek() {
            None | Some(Event::End(_)) => return out,
            Some(Event::Rule) => {}
            Some(Event::Start(t)) if !is_inline(t) => {}
            // bare inline content, e.g. the text of a tight list item
            Some(_) => {
                let p = inlines(ev);
                push_paragraph(&mut out, p);
                continue;
            }
        }
        match ev.next().unwrap() {
            Event::Rule => out.push(Block::Rule),
            Event::Start(Tag::Paragraph) => {
                let p = inlines(ev);
                ev.next();
                push_paragraph(&mut out, p);
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let (i, _) = inlines(ev);
                ev.next();
                out.push(Block::Heading(level as u8, i));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let b = blocks(ev);
                ev.next();
                out.push(Block::Quote(b));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => {
                        l.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut text = String::new();
                for e in ev.by_ref() {
                    match e {
                        Event::Text(t) => text.push_str(&t),
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                out.push(Block::Code { lang, text });
            }
            Event::Start(Tag::List(ordered)) => {
                let mut items = Vec::new();
                while let Some(e) = ev.next() {
                    match e {
                        Event::Start(Tag::Item) => {
                            items.push(blocks(ev));
                            ev.next();
                        }
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                out.push(Block::List { ordered, items });
            }
            Event::Start(Tag::Table(align)) => {
                let (mut head, mut rows) = (Vec::new(), Vec::new());
                while let Some(e) = ev.next() {
                    match e {
                        Event::Start(Tag::TableHead) => head = cells(ev),
                        Event::Start(Tag::TableRow) => rows.push(cells(ev)),
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                out.push(Block::Table { align, head, rows });
            }
            // table, html block, footnote definition, ...: transparent container
            Event::Start(_) => {
                out.extend(blocks(ev));
                ev.next();
            }
            _ => unreachable!("peeked above"),
        }
    }
}

/// The cells of a table head or row, consuming its `End`.
fn cells(ev: &mut Events) -> Vec<Vec<Inline>> {
    let mut row = Vec::new();
    while let Some(e) = ev.next() {
        match e {
            Event::Start(Tag::TableCell) => {
                row.push(inlines(ev).0);
                ev.next();
            }
            Event::End(_) => break,
            _ => {}
        }
    }
    row
}

/// Inline content up to (not including) the enclosing `End` or the next block-level `Start`.
/// The flag is true when the content is display math only, so the caller can promote it.
fn inlines(ev: &mut Events) -> (Vec<Inline>, bool) {
    let mut out = Vec::new();
    let mut display_only = true;
    loop {
        match ev.peek() {
            None | Some(Event::End(_)) | Some(Event::Rule) => break,
            Some(Event::Start(t)) if !is_inline(t) => break,
            _ => {}
        }
        let e = ev.next().unwrap();
        display_only &= matches!(
            e,
            Event::DisplayMath(_) | Event::SoftBreak | Event::HardBreak
        ) || matches!(&e, Event::Text(t) if t.trim().is_empty());
        match e {
            Event::Start(Tag::Emphasis) => {
                out.push(Inline::Emph(inlines(ev).0));
                ev.next();
            }
            Event::Start(Tag::Strong) => {
                out.push(Inline::Strong(inlines(ev).0));
                ev.next();
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                out.push(Inline::Link {
                    text: inlines(ev).0,
                    url: dest_url.to_string(),
                });
                ev.next();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                out.push(Inline::Image {
                    alt: inlines(ev).0,
                    url: dest_url.to_string(),
                });
                ev.next();
            }
            // strikethrough, super/subscript: plain text
            Event::Start(_) => {
                out.extend(inlines(ev).0);
                ev.next();
            }
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                out.push(Inline::Text(t.to_string()))
            }
            Event::Code(t) => out.push(Inline::Code(t.to_string())),
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push(Inline::Math(t.trim().to_string()))
            }
            Event::SoftBreak => out.push(Inline::Text(" ".into())),
            Event::HardBreak => out.push(Inline::Break),
            Event::FootnoteReference(t) => out.push(Inline::Text(format!("[^{t}]"))),
            Event::TaskListMarker(done) => {
                out.push(Inline::Text(if done { "[x] " } else { "[ ] " }.into()))
            }
            Event::End(_) | Event::Rule => unreachable!("peeked above"),
        }
    }
    let has_math = out.iter().any(|i| matches!(i, Inline::Math(_)));
    (out, display_only && has_math)
}

/// Display math and a lone image are promoted to blocks of their own.
fn push_paragraph(out: &mut Vec<Block>, (mut inl, display): (Vec<Inline>, bool)) {
    if display {
        out.extend(inl.into_iter().filter_map(|i| match i {
            Inline::Math(t) => Some(Block::DisplayMath(t)),
            _ => None,
        }));
    } else if let [Inline::Image { alt, url }] = &mut inl[..] {
        out.push(Block::Image {
            alt: std::mem::take(alt),
            url: std::mem::take(url),
        });
    } else if !inl.is_empty() {
        out.push(Block::Paragraph(inl));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_block_tree() {
        let b = parse(include_str!("../tests/fixture.md"));
        assert_eq!(b.len(), 13, "{b:#?}");
        assert!(matches!(&b[0], Block::Heading(1, i) if i == &[Inline::Text("Title".into())]));
        assert!(matches!(&b[1], Block::Heading(2, _)));
        let Block::Paragraph(inl) = &b[2] else {
            panic!("{:?}", b[2])
        };
        assert_eq!(
            inl.iter().filter(|i| matches!(i, Inline::Math(_))).count(),
            3
        );
        assert!(inl.iter().any(|i| matches!(i, Inline::Emph(_))));
        assert!(inl.iter().any(|i| matches!(i, Inline::Strong(_))));
        assert!(
            inl.iter()
                .any(|i| matches!(i, Inline::Code(c) if c == "inline code"))
        );
        assert!(
            inl.iter()
                .any(|i| matches!(i, Inline::Link { url, .. } if url == "https://example.com"))
        );
        assert!(matches!(&b[3], Block::DisplayMath(t) if t.starts_with("\\Sigma(\\tau)")));
        let Block::List {
            ordered: None,
            items,
        } = &b[4]
        else {
            panic!("{:?}", b[4])
        };
        assert_eq!(items.len(), 3);
        let [
            Block::Paragraph(_),
            Block::List {
                ordered: Some(1),
                items: nested,
            },
        ] = &items[1][..]
        else {
            panic!("{:?}", items[1])
        };
        assert_eq!(nested.len(), 2);
        let Block::Quote(q) = &b[5] else {
            panic!("{:?}", b[5])
        };
        assert!(
            matches!(&q[..], [Block::Paragraph(i)] if i.contains(&Inline::Math("\\omega_q".into())))
        );
        assert!(
            matches!(&b[6], Block::Paragraph(i) if i.contains(&Inline::Math("\\omega_q".into())))
        );
        let Block::Table { align, head, rows } = &b[7] else {
            panic!("{:?}", b[7])
        };
        assert_eq!(
            align,
            &[Alignment::Left, Alignment::Center, Alignment::Right]
        );
        assert_eq!(head.len(), 3);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], vec![Inline::Math("\\alpha".into())]);
        assert!(
            matches!(&b[8], Block::Code { lang, text } if lang == "rust" && text.contains("println"))
        );
        assert_eq!(b[9], Block::Rule);
        assert!(matches!(&b[10], Block::Paragraph(i) if i.len() == 3));
        assert_eq!(
            b[11],
            Block::Image {
                alt: vec![Inline::Text("a tiny png".into())],
                url: "tiny.png".into()
            }
        );
        assert!(matches!(&b[12], Block::Paragraph(i) if i.len() == 3
                && matches!(&i[1], Inline::Image { url, .. } if url == "tiny.jpg")));
    }

    #[test]
    fn display_math_mixed_with_text_stays_inline() {
        let b = parse("Consider $$E = mc^2$$ here.\n\n$$a$$\n$$b$$\n");
        assert!(matches!(&b[0], Block::Paragraph(i) if i.len() == 3));
        assert_eq!(b[1], Block::DisplayMath("a".into()));
        assert_eq!(b[2], Block::DisplayMath("b".into()));
    }
}
