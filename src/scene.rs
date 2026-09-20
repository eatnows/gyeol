//! A renderer-agnostic display list: what to draw, in logical pixels, back to front.
//!
//! Building a [`Scene`] needs no GPU or window, so layout and painting logic can be tested headlessly.

/// An sRGB color with straight (non-premultiplied) alpha, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0., g: 0., b: 0., a: 0. };

    /// `0xRRGGBB`, fully opaque.
    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xff) as f32 / 255.,
            g: ((rgb >> 8) & 0xff) as f32 / 255.,
            b: (rgb & 0xff) as f32 / 255.,
            a: 1.,
        }
    }

    pub const fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    pub(crate) fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, pos: (f32, f32)) -> bool {
        pos.0 >= self.x && pos.0 < self.x + self.w && pos.1 >= self.y && pos.1 < self.y + self.h
    }
}

/// A filled rectangle with optional rounded corners and border.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub bounds: Rect,
    pub background: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub corner_radius: f32,
}

impl Quad {
    pub fn new(bounds: Rect, background: Color) -> Quad {
        Quad { bounds, background, border_color: Color::TRANSPARENT, border_width: 0., corner_radius: 0. }
    }

    pub fn rounded(mut self, radius: f32) -> Quad {
        self.corner_radius = radius;
        self
    }

    pub fn bordered(mut self, width: f32, color: Color) -> Quad {
        self.border_width = width;
        self.border_color = color;
        self
    }
}

/// A run of text starting at `origin` (its top-left corner), in one style.
#[derive(Clone, Debug)]
pub struct Text {
    pub origin: (f32, f32),
    pub content: String,
    pub size: f32,
    pub color: Color,
}

impl Text {
    pub fn new(origin: (f32, f32), content: impl Into<String>, size: f32, color: Color) -> Text {
        Text { origin, content: content.into(), size, color }
    }
}

/// Everything to draw in one frame. Quads paint first, then text on top.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub background: Option<Color>,
    pub quads: Vec<Quad>,
    pub texts: Vec<Text>,
}
