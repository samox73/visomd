//! kitty graphics protocol: PNG transfer as a virtual placement, then Unicode placeholder cells
//! that the terminal fills with the image wherever the text lands.

use std::fmt::Write;

const PLACEHOLDER: char = '\u{10EEEE}';
/// Largest placement in cells per axis: the diacritics table has this many entries.
pub const MAX_CELLS: usize = DIACRITICS.len();

/// Row/column diacritics from kitty's rowcolumn-diacritics.txt; entry `n` encodes row or column `n`.
const DIACRITICS: [char; 297] = [
    '\u{0305}',
    '\u{030D}',
    '\u{030E}',
    '\u{0310}',
    '\u{0312}',
    '\u{033D}',
    '\u{033E}',
    '\u{033F}',
    '\u{0346}',
    '\u{034A}',
    '\u{034B}',
    '\u{034C}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035B}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036A}',
    '\u{036B}',
    '\u{036C}',
    '\u{036D}',
    '\u{036E}',
    '\u{036F}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059C}',
    '\u{059D}',
    '\u{059E}',
    '\u{059F}',
    '\u{05A0}',
    '\u{05A1}',
    '\u{05A8}',
    '\u{05A9}',
    '\u{05AB}',
    '\u{05AC}',
    '\u{05AF}',
    '\u{05C4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065A}',
    '\u{065B}',
    '\u{065D}',
    '\u{065E}',
    '\u{06D6}',
    '\u{06D7}',
    '\u{06D8}',
    '\u{06D9}',
    '\u{06DA}',
    '\u{06DB}',
    '\u{06DC}',
    '\u{06DF}',
    '\u{06E0}',
    '\u{06E1}',
    '\u{06E2}',
    '\u{06E4}',
    '\u{06E7}',
    '\u{06E8}',
    '\u{06EB}',
    '\u{06EC}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073A}',
    '\u{073D}',
    '\u{073F}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074A}',
    '\u{07EB}',
    '\u{07EC}',
    '\u{07ED}',
    '\u{07EE}',
    '\u{07EF}',
    '\u{07F0}',
    '\u{07F1}',
    '\u{07F3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081B}',
    '\u{081C}',
    '\u{081D}',
    '\u{081E}',
    '\u{081F}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082A}',
    '\u{082B}',
    '\u{082C}',
    '\u{082D}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0F82}',
    '\u{0F83}',
    '\u{0F86}',
    '\u{0F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// Image ids in `1..2^24`, so they fit in the placeholder's foreground color. Sequential from a
/// clock-derived start: unique within a run, and different runs in one terminal rarely collide.
pub struct Ids(u32);

impl Ids {
    pub fn new() -> Ids {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ids((t.subsec_nanos() ^ t.as_secs() as u32) & 0xFF_FFFF)
    }

    pub fn next(&mut self) -> u32 {
        self.0 = self.0 % 0xFF_FFFF + 1;
        self.0
    }
}

/// Transmit `png` as image `id` with a virtual placement of `cols`×`rows` cells. Nothing is drawn
/// until placeholder cells appear; `q=2` suppresses the reply since stdout is never read back.
pub fn transmit(id: u32, png: &[u8], cols: u16, rows: u16) -> String {
    let b64 = base64(png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    let mut out = String::with_capacity(b64.len() + 64 * chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        out.push_str("\x1b_G");
        if i == 0 {
            write!(out, "a=T,U=1,f=100,q=2,i={id},c={cols},r={rows},").unwrap();
        }
        let more = u8::from(i + 1 < chunks.len());
        write!(out, "m={more};").unwrap();
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push_str("\x1b\\");
    }
    out
}

/// One string per row of placeholder cells for image `id`: the id in the foreground color, each
/// cell `U+10EEEE` + row diacritic + column diacritic, color reset at the end.
pub fn placeholder(id: u32, cols: u16, rows: u16) -> Vec<String> {
    (0..rows as usize)
        .map(|r| {
            let mut s = format!(
                "\x1b[38;2;{};{};{}m",
                (id >> 16) & 255,
                (id >> 8) & 255,
                id & 255
            );
            for &col in &DIACRITICS[..cols as usize] {
                s.push(PLACEHOLDER);
                s.push(DIACRITICS[r]);
                s.push(col);
            }
            s.push_str("\x1b[39m");
            s
        })
        .collect()
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let v = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(T[(v >> 18) as usize & 63] as char);
        out.push(T[(v >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(v >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[v as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn placeholder_matches_kitten_icat() {
        // `kitten icat --unicode-placeholder --image-id 1 --place 2x1@0x0` emits these cell bytes
        // (minus a third, most-significant-byte diacritic, which ids below 2^24 never need).
        let icat_cells = b"\xf4\x8e\xbb\xae\xcc\x85\xcc\x85\xf4\x8e\xbb\xae\xcc\x85\xcc\x8d";
        let rows = placeholder(1, 2, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0],
            format!(
                "\x1b[38;2;0;0;1m{}\x1b[39m",
                std::str::from_utf8(icat_cells).unwrap()
            )
        );
        assert_eq!(
            placeholder(0x0A0B0C, 1, 2)[1],
            "\x1b[38;2;10;11;12m\u{10EEEE}\u{30D}\u{305}\x1b[39m"
        );
    }

    #[test]
    fn transmit_chunks_and_flags() {
        let big = vec![0u8; 5000]; // 6668 base64 bytes: two chunks
        let s = transmit(7, &big, 3, 1);
        assert!(s.starts_with("\x1b_Ga=T,U=1,f=100,q=2,i=7,c=3,r=1,m=1;"));
        assert!(s.contains("\x1b\\\x1b_Gm=0;"));
        assert!(s.ends_with("\x1b\\"));
        assert!(
            transmit(7, b"x", 1, 1).starts_with("\x1b_Ga=T,U=1,f=100,q=2,i=7,c=1,r=1,m=0;eA==")
        );
    }

    #[test]
    fn ids_stay_in_range_and_unique() {
        let mut ids = Ids(0xFF_FFFE);
        let a: Vec<u32> = (0..3).map(|_| ids.next()).collect();
        assert_eq!(a, [0xFF_FFFF, 1, 2]);
    }
}
