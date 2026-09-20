//! Text shaping and measuring. Needs no GPU, so layout code and tests can use it directly.
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};

/// Shapes text with system fonts (with per-script fallback) and answers measuring questions.
///
/// Shaped text is cached while it keeps being used, so measuring the same string many times per
/// frame is cheap.
pub struct Shaper {
    font_system: FontSystem,
    shaped: HashMap<(String, u32), (Buffer, u64)>,
    frame: u64,
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new()
    }
}

impl Shaper {
    pub fn new() -> Self {
        Shaper { font_system: FontSystem::new(), shaped: HashMap::new(), frame: 0 }
    }

    /// Height of one line of text at `size`.
    pub fn line_height(size: f32) -> f32 {
        (size * 1.4).round()
    }

    /// The width of `text` laid out on one line.
    pub fn width(&mut self, text: &str, size: f32) -> f32 {
        let (buffer, _) = self.buffer_and_fonts(text, size);
        buffer.layout_runs().next().map_or(0., |run| run.line_w)
    }

    /// The x offset of the caret before the character at byte index `byte` (clamped to the text).
    /// Positions inside a multi-character cluster snap to the cluster's start.
    pub fn caret_x(&mut self, text: &str, size: f32, byte: usize) -> f32 {
        let (buffer, _) = self.buffer_and_fonts(text, size);
        let Some(run) = buffer.layout_runs().next() else { return 0. };
        for glyph in run.glyphs {
            if byte <= glyph.start {
                return glyph.x;
            }
            if byte < glyph.end {
                return glyph.x;
            }
        }
        run.line_w
    }

    /// The byte index of the caret position closest to `x` (used for clicking into text).
    pub fn hit(&mut self, text: &str, size: f32, x: f32) -> usize {
        let (buffer, _) = self.buffer_and_fonts(text, size);
        let Some(run) = buffer.layout_runs().next() else { return 0 };
        for glyph in run.glyphs {
            if x < glyph.x + glyph.w / 2. {
                return glyph.start.min(text.len());
            }
        }
        text.len()
    }

    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Forgets text that was not used since the last frame began.
    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        self.shaped.retain(|_, (_, last_used)| *last_used == frame);
    }

    /// The shaped `text`, plus the font system (needed to rasterize its glyphs).
    pub(crate) fn buffer_and_fonts(&mut self, text: &str, size: f32) -> (&Buffer, &mut FontSystem) {
        let frame = self.frame;
        let entry = self.shaped.entry((text.to_string(), size.to_bits())).or_insert_with(|| {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, Self::line_height(size)));
            buffer.set_size(None, None);
            buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut self.font_system, false);
            (buffer, frame)
        });
        entry.1 = frame;
        (&entry.0, &mut self.font_system)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_positions_grow_with_the_text_and_hit_inverts_them() {
        let mut shaper = Shaper::new();
        let text = "Hello 한글 world";
        let mut last = -1.0;
        for (byte, _) in text.char_indices() {
            let x = shaper.caret_x(text, 16., byte);
            assert!(x > last, "caret x must increase at byte {byte}: {x} <= {last}");
            last = x;
            assert_eq!(shaper.hit(text, 16., x + 0.5), byte, "clicking just right of the caret at {byte}");
        }
        let end = shaper.caret_x(text, 16., text.len());
        assert!(end > last);
        assert!((end - shaper.width(text, 16.)).abs() < 0.01);
        assert_eq!(shaper.hit(text, 16., end + 50.), text.len(), "clicking past the end lands at the end");
        assert_eq!(shaper.hit(text, 16., -5.), 0);
    }

    #[test]
    fn empty_text_has_no_width() {
        let mut shaper = Shaper::new();
        assert_eq!(shaper.width("", 16.), 0.);
        assert_eq!(shaper.caret_x("", 16., 0), 0.);
        assert_eq!(shaper.hit("", 16., 10.), 0);
    }
}
