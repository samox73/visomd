//! LaTeX → RaTeX display list → PNG → row-aligned RGBA canvas.
//!
//! Inline: exactly one terminal row tall, ink on the text baseline. Display: a whole number of
//! rows, ink centred. Both are padded right to a cell boundary so the kitty placement box has the
//! canvas's aspect ratio and the terminal never rescales.

use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::{Color, DisplayList, MathStyle};

/// Inline em as a fraction of the row: the math x-height then matches the terminal font's.
const INLINE_EM_ROWS: f64 = 0.9;
/// Text baseline position in a row, from the top.
const INLINE_BASELINE_ROWS: f64 = 0.77;
/// Fraction of the row an inline expression too tall for the row may fill when shrunk and centred.
const INLINE_FIT: f64 = 0.96;
/// Height cap for display math in rows.
const DISPLAY_MAX_ROWS: f64 = 4.0;

/// Straight-alpha RGBA, transparent background.
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        Canvas {
            w,
            h,
            rgba: vec![0; (w * h * 4) as usize],
        }
    }

    fn px(&self, x: u32, y: u32) -> &[u8] {
        let i = ((y * self.w + x) * 4) as usize;
        &self.rgba[i..i + 4]
    }

    /// Bounding box of the visible pixels: (x0, y0, x1, y1) inclusive.
    fn ink_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let inked_row = |y: u32| (0..self.w).any(|x| self.px(x, y)[3] > 0);
        let inked_col = |x: u32| (0..self.h).any(|y| self.px(x, y)[3] > 0);
        let y0 = (0..self.h).find(|&y| inked_row(y))?;
        let y1 = (0..self.h).rev().find(|&y| inked_row(y))?;
        let x0 = (0..self.w).find(|&x| inked_col(x))?;
        let x1 = (0..self.w).rev().find(|&x| inked_col(x))?;
        Some((x0, y0, x1, y1))
    }

    fn crop(&self, x0: u32, y0: u32, w: u32, h: u32) -> Canvas {
        let mut out = Canvas::new(w, h);
        for y in 0..h {
            let src = (((y0 + y) * self.w + x0) * 4) as usize;
            let dst = (y * w * 4) as usize;
            out.rgba[dst..dst + (w * 4) as usize]
                .copy_from_slice(&self.rgba[src..src + (w * 4) as usize]);
        }
        out
    }

    /// Copy `src` with its top-left at (x, y), clipped to this canvas.
    fn blit(&mut self, src: &Canvas, x: u32, y: u32) {
        let w = src.w.min(self.w.saturating_sub(x));
        for sy in 0..src.h.min(self.h.saturating_sub(y)) {
            let s = (sy * src.w * 4) as usize;
            let d = (((y + sy) * self.w + x) * 4) as usize;
            self.rgba[d..d + (w * 4) as usize].copy_from_slice(&src.rgba[s..s + (w * 4) as usize]);
        }
    }

    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.w, self.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(&self.rgba).expect("png data");
        writer.finish().expect("png end");
        out
    }
}

fn decode(png: &[u8]) -> Option<Canvas> {
    let mut reader = png::Decoder::new(png).read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    rgba.truncate(info.buffer_size());
    Some(Canvas {
        w: info.width,
        h: info.height,
        rgba,
    })
}

/// Parse and lay out `tex`; `width`, `height`, `depth` of the result are in em.
fn typeset(tex: &str, style: MathStyle, fg: [u8; 3]) -> Option<DisplayList> {
    let tex = tex.trim();
    if tex.is_empty() {
        return None;
    }
    let ast = parse(tex).ok()?;
    let color = Color::new(
        f32::from(fg[0]) / 255.0,
        f32::from(fg[1]) / 255.0,
        f32::from(fg[2]) / 255.0,
        1.0,
    );
    let opts = LayoutOptions::default().with_style(style).with_color(color);
    Some(to_display_list(&layout(&ast, &opts)))
}

/// Rasterise at `em` px per em, cropped to the inked columns. Height is kept, so the baseline stays
/// `list.height * em` below the top. None when nothing visible was drawn.
fn raster(list: &DisplayList, em: f64) -> Option<Canvas> {
    let png = render_to_png(
        list,
        &RenderOptions {
            font_size: em as f32,
            padding: 0.0,
            background_color: Color::new(0.0, 0.0, 0.0, 0.0),
            font_dir: String::new(),
            device_pixel_ratio: 1.0,
        },
    )
    .ok()?;
    let img = decode(&png)?;
    let (x0, _, x1, _) = img.ink_bounds()?;
    Some(img.crop(x0, 0, x1 - x0 + 1, img.h))
}

/// Inline math in TeX text style on one row of `cell_h` px, baseline shared with the prose. An
/// expression that pokes out of the row at full size (a subscript with a descender, a hat) is
/// shifted just enough to stay inside; only one too tall for the row altogether is shrunk and
/// centred.
pub fn inline(tex: &str, cell_w: u32, cell_h: u32, fg: [u8; 3]) -> Option<Canvas> {
    let list = typeset(tex, MathStyle::Text, fg)?;
    let row = f64::from(cell_h);
    let (height, depth) = (list.height.max(0.0), list.depth.max(0.0));
    let total = (height + depth).max(1e-6);
    let full = INLINE_EM_ROWS * row;
    let (em, top) = if total * full <= row {
        let top = INLINE_BASELINE_ROWS * row - height * full;
        (full, top.clamp(0.0, row - total * full).round())
    } else {
        let em = INLINE_FIT * row / total;
        (em, ((row - total * em) / 2.0).round())
    };
    let ink = raster(&list, em)?;
    let cols = ink.w.div_ceil(cell_w).max(1);
    let mut canvas = Canvas::new(cols * cell_w, cell_h);
    canvas.blit(&ink, 0, top as u32);
    Some(canvas)
}

/// Display math at `scale` times the inline size (1 = the prose size, as in LaTeX), scaled down to
/// at most 4 rows and `max_w` px wide, cropped to its ink and centred in a whole number of rows.
pub fn display(
    tex: &str,
    cell_w: u32,
    cell_h: u32,
    max_w: u32,
    scale: f64,
    fg: [u8; 3],
) -> Option<Canvas> {
    let list = typeset(tex, MathStyle::Display, fg)?;
    let row = f64::from(cell_h);
    let mut em = INLINE_EM_ROWS * scale.max(0.1) * row;
    em = em.min(DISPLAY_MAX_ROWS * row / (list.height + list.depth).max(1e-6));
    if list.width > 0.0 {
        em = em.min(f64::from(max_w) / list.width);
    }
    let img = raster(&list, em)?;
    let (_, y0, _, y1) = img.ink_bounds()?;
    let ink = img.crop(0, y0, img.w, y1 - y0 + 1);
    let rows = ink.h.div_ceil(cell_h).max(1);
    let cols = ink.w.div_ceil(cell_w).max(1);
    let mut canvas = Canvas::new(cols * cell_w, rows * cell_h);
    canvas.blit(&ink, 0, (rows * cell_h - ink.h) / 2);
    Some(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: [u8; 3] = [200, 200, 200];

    /// First and last inked rows, and empty rows above and below.
    fn ink_rows(c: &Canvas) -> (u32, u32) {
        let (_, y0, _, y1) = c.ink_bounds().unwrap();
        (y0, y1)
    }
    fn margins(c: &Canvas) -> (u32, u32) {
        let (y0, y1) = ink_rows(c);
        (y0, c.h - 1 - y1)
    }

    #[test]
    fn inline_is_one_row_and_ink_tight() {
        for tex in [
            r"x",
            r"\frac{1}{x^2}",
            r"\int_0^\infty dx",
            r"\left(n+\tfrac12\right)",
            r"a^2 + b^2",
        ] {
            let c = inline(tex, 8, 16, FG).expect(tex);
            assert_eq!(c.h, 16, "height of {tex}");
            assert_eq!(c.w % 8, 0, "cell-aligned width of {tex}");
            let (x0, _, x1, _) = c.ink_bounds().unwrap();
            assert_eq!(x0, 0, "ink touches the left edge: {tex}");
            assert!(
                c.w - 1 - x1 < 8,
                "ink within one cell of the right edge: {tex}"
            );
        }
    }

    #[test]
    fn inline_shares_one_size_and_baseline() {
        let x = ink_rows(&inline("x", 8, 32, FG).unwrap());
        let eq = ink_rows(&inline("x = 1", 8, 32, FG).unwrap());
        assert_eq!(x.1, eq.1, "same baseline");
        assert_eq!(x, ink_rows(&inline("xx", 8, 32, FG).unwrap()), "same size");
        assert_eq!(
            inline(r"\left(n + \tfrac{1}{2}\right)", 8, 32, FG)
                .unwrap()
                .h,
            32
        );
    }

    #[test]
    fn subscript_with_descender_keeps_the_baseline() {
        // a 26 px row leaves 6 px under the baseline; the q descender needs a little more, so the
        // expression shifts up by that pixel instead of being centred (which lifted it ~5 px)
        let x = ink_rows(&inline("x", 12, 26, FG).unwrap());
        let wq = ink_rows(&inline(r"\omega_q", 12, 26, FG).unwrap());
        assert!(
            x.0.abs_diff(wq.0) <= 2,
            "x-height tops: x at {}, omega_q at {}",
            x.0,
            wq.0
        );
        assert_eq!(inline(r"\omega_q", 12, 26, FG).unwrap().h, 26);
    }

    #[test]
    fn inline_too_tall_for_the_baseline_is_centred() {
        let c = inline(
            r"\delta\left[c_0/(n-1)\right] \approx \frac{c_0}{n-1}\sqrt{\frac{2}{n-1}}",
            10,
            36,
            FG,
        )
        .unwrap();
        assert_eq!(c.h, 36);
        let (top, bottom) = margins(&c);
        assert!(top.abs_diff(bottom) <= 2, "{top}px above, {bottom}px below");
        assert!(top + bottom <= 4, "fills the row: {top}+{bottom}px empty");
    }

    #[test]
    fn display_is_capped_centred_and_fits_the_width() {
        let tall = r"\sigma^2(m) = \frac{1}{n}\left[\gamma_0 + 2\sum_{t=1}^{n-1}\left(1-\frac{t}{n}\right)\gamma_t\right] \xrightarrow{n\gg\tau} \frac{2\tau_{\text{int}}\gamma_0}{n}";
        for tex in [tall, "x = 1"] {
            let c = display(tex, 10, 36, 600, 2.0, FG).unwrap();
            assert_eq!(c.h % 36, 0, "{tex}: whole rows");
            assert!(c.h <= 4 * 36, "{tex}: {}px", c.h);
            assert!(
                c.w <= 600 && c.w.is_multiple_of(10),
                "{tex}: {}px wide",
                c.w
            );
            let (top, bottom) = margins(&c);
            assert!(
                top.abs_diff(bottom) <= 1,
                "{tex}: {top}px above, {bottom}px below"
            );
        }
        let narrow = display(tall, 10, 36, 200, 2.0, FG).unwrap();
        assert!(narrow.w <= 200, "scaled to the width: {}px", narrow.w);
        // at scale 1 display glyphs match inline ones: `x` has the same ink height in both
        let (x0, x1) = ink_rows(&inline("x", 10, 36, FG).unwrap());
        let (d0, d1) = ink_rows(&display("x", 10, 36, 600, 1.0, FG).unwrap());
        assert!(
            (x1 - x0).abs_diff(d1 - d0) <= 1,
            "inline {} vs display {} px",
            x1 - x0,
            d1 - d0
        );
    }

    #[test]
    fn bad_input_is_none_and_png_round_trips() {
        assert!(inline("", 8, 16, FG).is_none());
        assert!(inline("   ", 8, 16, FG).is_none());
        let c = inline("x", 8, 16, FG).unwrap();
        let back = decode(&c.png()).unwrap();
        assert_eq!((back.w, back.h), (c.w, c.h));
        assert_eq!(back.rgba, c.rgba);
    }
}
