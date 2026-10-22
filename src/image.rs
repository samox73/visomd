//! Image files for `![alt](path)`: PNG bytes pass through, JPEG is decoded and re-encoded as PNG.

use std::path::Path;

use crate::kitty;

pub struct Image {
    pub png: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// The file at `path` as PNG with its pixel size; `None` for anything but a PNG or JPEG file.
// ponytail: no downscaling before transfer, so a 12-megapixel photo is a 16 MB escape stream. Add
// a box-filter resize when a note actually has one.
pub fn load(path: &Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    match bytes.get(..4)? {
        b"\x89PNG" => {
            // IHDR is always the first chunk: width and height at bytes 16..24, big-endian
            let be = |at: usize| {
                bytes
                    .get(at..at + 4)
                    .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
            };
            let (w, h) = (be(16)?, be(20)?);
            Some(Image { png: bytes, w, h })
        }
        [0xFF, 0xD8, 0xFF, _] => {
            let mut dec = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes[..]));
            let px = dec.decode().ok()?;
            let info = dec.info()?;
            let color = match dec.output_colorspace()?.num_components() {
                1 => png::ColorType::Grayscale,
                3 => png::ColorType::Rgb,
                _ => return None,
            };
            let (w, h) = (u32::from(info.width), u32::from(info.height));
            let mut png = Vec::new();
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&px).ok()?;
            writer.finish().ok()?;
            Some(Image { png, w, h })
        }
        _ => None,
    }
}

/// Placement of a `w`×`h` px image in at most `avail` columns: the columns its width needs, then
/// the rows that keep the aspect ratio at that width; both capped at `kitty::MAX_CELLS`. kitty
/// fits the image into the box, so the last row may be partly empty.
pub fn fit(w: u32, h: u32, cell_w: u32, cell_h: u32, avail: usize) -> (u16, u16) {
    let max = kitty::MAX_CELLS as u32;
    let cols = w.div_ceil(cell_w).clamp(1, (avail as u32).clamp(1, max));
    let scale = f64::from(cols * cell_w) / f64::from(w.max(1));
    let rows = (f64::from(h) * scale / f64::from(cell_h)).ceil() as u32;
    (cols as u16, rows.clamp(1, max) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(name)
    }

    #[test]
    fn png_passes_through_jpeg_is_converted_others_are_none() {
        let png = load(&fixture("tiny.png")).unwrap();
        assert_eq!((png.w, png.h), (2, 2));
        assert_eq!(png.png, std::fs::read(fixture("tiny.png")).unwrap());

        let jpg = load(&fixture("tiny.jpg")).unwrap();
        assert_eq!((jpg.w, jpg.h), (8, 4));
        let mut reader = png::Decoder::new(&jpg.png[..]).read_info().unwrap();
        let mut rgb = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut rgb).unwrap();
        assert_eq!(
            (info.width, info.height, info.color_type),
            (8, 4, png::ColorType::Rgb)
        );
        // a red-to-blue gradient: the first pixel is red, the last blue
        assert!(rgb[0] > 200 && rgb[2] < 60, "first pixel {:?}", &rgb[..3]);
        let last = &rgb[rgb.len() - 3..];
        assert!(last[2] > 200 && last[0] < 60, "last pixel {last:?}");

        assert!(load(&fixture("fixture.md")).is_none());
        assert!(load(&fixture("missing.png")).is_none());
    }

    #[test]
    fn fit_fills_the_width_and_keeps_the_aspect_ratio() {
        assert_eq!(fit(2, 2, 8, 16, 80), (1, 1));
        // 17 px needs 3 cells of 8; at 24 px wide the 16 px height is 22.6 px: 2 rows
        assert_eq!(fit(17, 16, 8, 16, 80), (3, 2));
        // 800 px into 50 columns of 8 = 400 px: scale 0.5, 300 px tall = 19 rows of 16
        assert_eq!(fit(800, 600, 8, 16, 50), (50, 19));
        let max = kitty::MAX_CELLS as u16;
        assert_eq!(fit(100_000, 10, 8, 16, 100_000), (max, 1));
        assert_eq!(fit(100, 100_000, 8, 16, 100_000), (13, max));
        assert_eq!(fit(100, 100, 8, 16, 0), (1, 1));
    }
}
