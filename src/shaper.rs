//! Text shaping and measuring. Needs no GPU, so layout code and tests can use it directly.
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, Weight};

/// How a run of text looks: size plus optional family, weight and slant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// A font family by name (`"Menlo"`); the system's default sans-serif when `None` or missing.
    pub family: Option<&'static str>,
    pub bold: bool,
    pub italic: bool,
}

impl TextStyle {
    pub fn new(size: f32) -> TextStyle {
        TextStyle { size, family: None, bold: false, italic: false }
    }

    pub fn family(mut self, family: &'static str) -> TextStyle {
        self.family = Some(family);
        self
    }

    pub fn bold(mut self) -> TextStyle {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> TextStyle {
        self.italic = true;
        self
    }

    fn key(&self) -> (u32, Option<&'static str>, bool, bool) {
        (self.size.to_bits(), self.family, self.bold, self.italic)
    }
}

impl From<f32> for TextStyle {
    fn from(size: f32) -> TextStyle {
        TextStyle::new(size)
    }
}

/// Shapes text with system fonts (with per-script fallback) and answers measuring questions.
///
/// Shaped text is cached while it keeps being used, so measuring the same string many times per
/// frame is cheap.
pub struct Shaper {
    font_system: FontSystem,
    shaped: HashMap<(String, (u32, Option<&'static str>, bool, bool)), (Buffer, u64)>,
    frame: u64,
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new()
    }
}

impl Shaper {
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        // `fontdb` (which cosmic-text sits on) has no built-in "sans-serif" generic-family alias on
        // macOS, so `TextStyle`'s default (`family: None`, meant to mean "the system's default
        // sans-serif") would otherwise fail to resolve to anything and fall back to whatever
        // arbitrary font the shaper picks next — not the real system UI font. macOS does ship the
        // system font (San Francisco) as a queryable face named "System Font"; point the alias at it.
        #[cfg(target_os = "macos")]
        font_system.db_mut().set_sans_serif_family("System Font");
        Shaper { font_system, shaped: HashMap::new(), frame: 0 }
    }

    /// Height of one line of text at `size`.
    pub fn line_height(size: f32) -> f32 {
        (size * 1.4).round()
    }

    /// The width of `text` laid out on one line.
    pub fn width(&mut self, text: &str, style: impl Into<TextStyle>) -> f32 {
        let (buffer, _) = self.buffer_and_fonts(text, style.into());
        buffer.layout_runs().next().map_or(0., |run| run.line_w)
    }

    /// The x offset of the caret before the character at byte index `byte` (clamped to the text).
    /// Positions inside a multi-character cluster snap to the cluster's start.
    pub fn caret_x(&mut self, text: &str, style: impl Into<TextStyle>, byte: usize) -> f32 {
        let (buffer, _) = self.buffer_and_fonts(text, style.into());
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
    pub fn hit(&mut self, text: &str, style: impl Into<TextStyle>, x: f32) -> usize {
        let (buffer, _) = self.buffer_and_fonts(text, style.into());
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
    pub(crate) fn buffer_and_fonts(&mut self, text: &str, style: TextStyle) -> (&Buffer, &mut FontSystem) {
        let frame = self.frame;
        let entry = self.shaped.entry((text.to_string(), style.key())).or_insert_with(|| {
            let size = style.size;
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, Self::line_height(size)));
            buffer.set_size(None, None);
            let mut attrs = Attrs::new();
            if let Some(name) = style.family {
                attrs = attrs.family(Family::Name(name));
            }
            if style.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if style.italic {
                attrs = attrs.style(FontStyle::Italic);
            }
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
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

    #[test]
    fn style_changes_the_measured_width() {
        let mut shaper = Shaper::new();
        let plain = shaper.width("Hello world", 16.);
        let bold = shaper.width("Hello world", TextStyle::new(16.).bold());
        assert!(bold > plain, "bold text is wider: {bold} vs {plain}");
        let mono = |s: &mut Shaper, t: &str| s.width(t, TextStyle::new(16.).family("Menlo"));
        assert_eq!(mono(&mut shaper, "iiiiii"), mono(&mut shaper, "WWWWWW"), "a monospace family gives every character the same width");
        assert_ne!(shaper.width("iiiiii", 16.), shaper.width("WWWWWW", 16.), "the default family is proportional");
    }
}
