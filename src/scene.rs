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

    /// The overlap of two rectangles (empty, with zero size, when they do not overlap).
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        Rect::new(x0, y0, (x1 - x0).max(0.), (y1 - y0).max(0.))
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
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub corner_radii: [f32; 4],
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
}

impl Quad {
    pub fn new(bounds: Rect, background: Color) -> Quad {
        Quad { bounds, background, border_color: Color::TRANSPARENT, border_width: 0., corner_radii: [0.; 4], clip: None }
    }

    pub fn rounded(mut self, radius: f32) -> Quad {
        self.corner_radii = [radius; 4];
        self
    }

    pub fn rounded_corners(mut self, radii: [f32; 4]) -> Quad {
        self.corner_radii = radii;
        self
    }

    pub fn bordered(mut self, width: f32, color: Color) -> Quad {
        self.border_width = width;
        self.border_color = color;
        self
    }

    pub fn clipped(mut self, clip: Option<Rect>) -> Quad {
        self.clip = clip;
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
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
}

impl Text {
    pub fn new(origin: (f32, f32), content: impl Into<String>, size: f32, color: Color) -> Text {
        Text { origin, content: content.into(), size, color, clip: None }
    }

    pub fn clipped(mut self, clip: Option<Rect>) -> Text {
        self.clip = clip;
        self
    }
}

/// One thing to draw.
#[derive(Clone, Debug)]
pub enum Item {
    Quad(Quad),
    Text(Text),
}

/// Everything to draw in one frame, in painting order: later items cover earlier ones.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub background: Option<Color>,
    pub items: Vec<Item>,
}

impl Scene {
    pub fn push_quad(&mut self, quad: Quad) {
        self.items.push(Item::Quad(quad));
    }

    pub fn push_text(&mut self, text: Text) {
        self.items.push(Item::Text(text));
    }

    pub fn quads(&self) -> impl Iterator<Item = &Quad> {
        self.items.iter().filter_map(|i| if let Item::Quad(q) = i { Some(q) } else { None })
    }

    pub fn texts(&self) -> impl Iterator<Item = &Text> {
        self.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t) } else { None })
    }
}
