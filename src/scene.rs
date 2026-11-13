//! A renderer-agnostic display list: what to draw, in logical pixels, back to front.
//!
//! Building a [`Scene`] needs no GPU or window, so layout and painting logic can be tested headlessly.

use crate::shaper::TextStyle;

/// An sRGB color with straight (non-premultiplied) alpha, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

    /// This color with its alpha multiplied by `factor` (`0.0..=1.0`).
    pub fn faded(self, factor: f32) -> Color {
        Color { a: self.a * factor, ..self }
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

/// Most color stops in one [`Gradient`]; extra stops are ignored.
pub const MAX_GRADIENT_STOPS: usize = 8;
/// Most gradient layers on one [`Quad`]; extra layers are ignored.
pub const MAX_GRADIENT_LAYERS: usize = 4;

/// A list with room for at most `N` items, so the types that hold one stay `Copy`. Pushing onto a full
/// list does nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capped<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for Capped<T, N> {
    fn default() -> Self {
        Capped { items: [T::default(); N], len: 0 }
    }
}

impl<T: Copy + Default, const N: usize> Capped<T, N> {
    /// Adds `item` at the end; returns whether there was room.
    pub fn push(&mut self, item: T) -> bool {
        if self.len == N {
            return false;
        }
        self.items[self.len] = item;
        self.len += 1;
        true
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.items[..self.len].iter_mut()
    }
}

impl<T: Copy + Default, const N: usize> FromIterator<T> for Capped<T, N> {
    /// Keeps the first `N` items.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Capped::default();
        for item in iter {
            if !list.push(item) {
                break;
            }
        }
        list
    }
}

/// One color of a [`Gradient`] and where it sits along the gradient line, `0.0..=1.0`.
/// `position: None` is placed by the CSS rules (see [`Gradient::resolved_stops`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub position: Option<f32>,
}

/// How big a radial gradient is: where its last stop (100%) sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    /// A circle reaching the farthest corner of the box (CSS `circle farthest-corner`).
    FarthestCorner,
    /// A circle reaching the nearest side of the box (CSS `circle closest-side`).
    ClosestSide,
    /// A circle of this radius in logical pixels (CSS `circle 120px`).
    Circle(f32),
    /// An ellipse with these horizontal and vertical radii in logical pixels (CSS `ellipse 500px 360px`).
    Ellipse(f32, f32),
    /// An ellipse whose radii are these fractions of the box's width and height (CSS `ellipse 50% 50%`).
    EllipseFraction(f32, f32),
}

/// The shape of a [`Gradient`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    /// CSS `linear-gradient(<angle>, ...)`: `angle` is in degrees, 0 points up, 90 right, 180 down.
    Linear { angle: f32 },
    /// CSS `radial-gradient(<size> at <x> <y>, ...)`: `center` is the position as fractions of the box
    /// (`(0.12, 0.1)` is `at 12% 10%`, `(0.5, 0.5)` the middle).
    Radial { center: (f32, f32), size: RadialSize },
}

impl Default for GradientKind {
    fn default() -> Self {
        GradientKind::Linear { angle: 180. }
    }
}

/// A gradient with up to [`MAX_GRADIENT_STOPS`] color stops, linear or radial. Colors are interpolated
/// premultiplied, like CSS, so `color -> transparent` fades out without going gray.
///
/// ```
/// # use gyeol::{Color, Gradient, RadialSize};
/// // radial-gradient(500px 360px at 12% 10%, rgba(120,90,255,.5), transparent 70%)
/// let glow = Gradient::radial((0.12, 0.10), RadialSize::Ellipse(500., 360.))
///     .stop(Color::hex(0x785aff).with_alpha(0.5))
///     .stop_at(Color::TRANSPARENT, 0.7);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Capped<GradientStop, MAX_GRADIENT_STOPS>,
}

impl Gradient {
    /// A linear gradient at `angle` degrees (0 up, 90 right, 180 down), with no stops yet.
    pub fn linear(angle: f32) -> Gradient {
        Gradient { kind: GradientKind::Linear { angle }, stops: Capped::default() }
    }

    /// A radial gradient centered at `center` (fractions of the box), with no stops yet.
    pub fn radial(center: (f32, f32), size: RadialSize) -> Gradient {
        Gradient { kind: GradientKind::Radial { center, size }, stops: Capped::default() }
    }

    /// This gradient with every stop's alpha multiplied by `factor`.
    pub fn faded(mut self, factor: f32) -> Gradient {
        for stop in self.stops.iter_mut() {
            stop.color = stop.color.faded(factor);
        }
        self
    }

    /// Adds a stop whose position is worked out by CSS: the first is at 0, the last at 1, and stops
    /// without a position are spread evenly between their neighbours.
    pub fn stop(mut self, color: Color) -> Gradient {
        self.stops.push(GradientStop { color, position: None });
        self
    }

    /// Adds a stop at `position` (`0.0..=1.0`, i.e. `70%` is `0.7`).
    pub fn stop_at(mut self, color: Color, position: f32) -> Gradient {
        self.stops.push(GradientStop { color, position: Some(position) });
        self
    }

    /// The stops with every position filled in, following CSS: the first defaults to 0 and the last to 1,
    /// positions are clamped to `0..=1` and never go backwards, and runs of unpositioned stops are
    /// spread evenly between the stops around them.
    pub fn resolved_stops(&self) -> Vec<(f32, Color)> {
        let stops = self.stops.as_slice();
        let mut at: Vec<Option<f32>> = stops.iter().map(|s| s.position.map(|p| p.clamp(0., 1.))).collect();
        if let Some(first) = at.first_mut() {
            first.get_or_insert(0.);
        }
        if let Some(last) = at.last_mut() {
            last.get_or_insert(1.);
        }
        let mut highest = 0f32;
        for p in at.iter_mut().flatten() {
            *p = p.max(highest);
            highest = *p;
        }
        let mut i = 0;
        while i < at.len() {
            if at[i].is_some() {
                i += 1;
                continue;
            }
            let start = i - 1;
            let end = (i..at.len()).find(|&j| at[j].is_some()).expect("the last stop has a position");
            let (from, to) = (at[start].expect("set"), at[end].expect("set"));
            for (k, slot) in at[start + 1..end].iter_mut().enumerate() {
                *slot = Some(from + (to - from) * (k + 1) as f32 / (end - start) as f32);
            }
            i = end;
        }
        stops.iter().zip(at).map(|(s, p)| (p.expect("every position is resolved"), s.color)).collect()
    }
}

/// Most outer shadows, and most inset shadows, on one [`Quad`]; extra ones are ignored.
pub const MAX_SHADOWS: usize = 4;

/// One CSS `box-shadow`: `offset-x offset-y blur-radius spread-radius color`, outer or inset.
///
/// `blur` is the CSS blur radius, a gaussian with standard deviation `blur / 2`; `0` is a hard edge.
/// `spread` grows the shadow's shape on every side (shrinks it when negative; an inset shadow's
/// spread shrinks the lit area instead).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
    pub offset: (f32, f32),
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

impl Shadow {
    /// This shadow with its color's alpha multiplied by `factor`.
    pub fn faded(mut self, factor: f32) -> Shadow {
        self.color = self.color.faded(factor);
        self
    }

    /// `box-shadow: x y blur spread color`.
    pub fn new(x: f32, y: f32, blur: f32, spread: f32, color: Color) -> Shadow {
        Shadow { offset: (x, y), blur, spread, color }
    }
}

/// A CSS `backdrop-filter: blur(radius) saturate(saturate)`: what is already painted behind a quad is
/// blurred and saturated, within the quad's rounded shape, before the quad's own fill is drawn.
/// `radius` is the CSS `blur()` argument: the gaussian's standard deviation in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropBlur {
    pub radius: f32,
    /// 1 leaves colors as they are, 0 is grayscale, above 1 boosts them.
    pub saturate: f32,
}

/// A filled rectangle with optional rounded corners and border.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub bounds: Rect,
    /// Fill color, the bottom layer.
    pub background: Color,
    /// Gradient layers painted over `background`, first (index 0) at the bottom. CSS lists the top
    /// layer first, so reverse the order when porting a `background:` list.
    pub gradients: Capped<Gradient, MAX_GRADIENT_LAYERS>,
    /// Outer shadows (CSS `box-shadow`), drawn behind the quad and only outside its shape. First
    /// (index 0) at the bottom; CSS lists the top shadow first, so reverse the order when porting.
    pub shadows: Capped<Shadow, MAX_SHADOWS>,
    /// Inset shadows (CSS `box-shadow: inset ...`), drawn over the fill, inside the border and clipped
    /// to the rounded shape. First (index 0) at the bottom.
    pub inset_shadows: Capped<Shadow, MAX_SHADOWS>,
    /// Blurs and saturates what is behind the quad; the fill (usually translucent) is drawn over it.
    pub backdrop_blur: Option<BackdropBlur>,
    pub border_color: Color,
    pub border_width: f32,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub corner_radii: [f32; 4],
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
}

impl Quad {
    pub fn new(bounds: Rect, background: Color) -> Quad {
        Quad { bounds, background, gradients: Capped::default(), shadows: Capped::default(), inset_shadows: Capped::default(), backdrop_blur: None, border_color: Color::TRANSPARENT, border_width: 0., corner_radii: [0.; 4], clip: None }
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

    /// Fills with a two-stop linear gradient instead of the flat background and any earlier gradient
    /// layers (CSS `linear-gradient(<angle>, from, to)`). For more control use [`Quad::gradient_layer`].
    pub fn gradient(mut self, angle: f32, from: Color, to: Color) -> Quad {
        self.background = Color::TRANSPARENT;
        self.gradients.clear();
        self.gradients.push(Gradient::linear(angle).stop(from).stop(to));
        self
    }

    /// Paints `gradient` over everything added so far (the background color and earlier layers).
    /// At most [`MAX_GRADIENT_LAYERS`] layers are kept.
    pub fn gradient_layer(mut self, gradient: Gradient) -> Quad {
        self.gradients.push(gradient);
        self
    }

    /// Sets the one inner shadow or highlight (CSS `box-shadow: inset x y blur color`), replacing any
    /// inset shadows. For several, or a spread, use [`Quad::inset_shadow_layer`].
    pub fn inset_shadowed(mut self, x: f32, y: f32, blur: f32, color: Color) -> Quad {
        self.inset_shadows.clear();
        self.inset_shadows.push(Shadow::new(x, y, blur, 0., color));
        self
    }

    /// Adds an inset shadow over the ones added so far. At most [`MAX_SHADOWS`] are kept.
    pub fn inset_shadow_layer(mut self, shadow: Shadow) -> Quad {
        self.inset_shadows.push(shadow);
        self
    }

    /// Blurs and saturates what is behind the quad (CSS `backdrop-filter: blur(radius) saturate(saturate)`).
    pub fn backdrop_blurred(mut self, radius: f32, saturate: f32) -> Quad {
        self.backdrop_blur = Some(BackdropBlur { radius, saturate });
        self
    }

    /// Sets the one outer drop shadow (CSS `box-shadow: x y blur color`), replacing any outer shadows.
    /// For several, or a spread, use [`Quad::shadow_layer`].
    pub fn shadowed(mut self, x: f32, y: f32, blur: f32, color: Color) -> Quad {
        self.shadows.clear();
        self.shadows.push(Shadow::new(x, y, blur, 0., color));
        self
    }

    /// Adds an outer shadow over the ones added so far. At most [`MAX_SHADOWS`] are kept.
    pub fn shadow_layer(mut self, shadow: Shadow) -> Quad {
        self.shadows.push(shadow);
        self
    }
}

/// A run of text starting at `origin` (its top-left corner), in one style.
#[derive(Clone, Debug)]
pub struct Text {
    pub origin: (f32, f32),
    pub content: String,
    pub style: TextStyle,
    pub color: Color,
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
    /// Wraps the text into lines at most this wide; `None` draws it on one line.
    pub wrap_width: Option<f32>,
}

impl Text {
    pub fn new(origin: (f32, f32), content: impl Into<String>, style: impl Into<TextStyle>, color: Color) -> Text {
        Text { origin, content: content.into(), style: style.into(), color, clip: None, wrap_width: None }
    }

    /// Wraps the text at `width` logical pixels (see [`Shaper::wrapped_size`](crate::Shaper::wrapped_size)).
    pub fn wrapped(mut self, width: Option<f32>) -> Text {
        self.wrap_width = width;
        self
    }

    pub fn clipped(mut self, clip: Option<Rect>) -> Text {
        self.clip = clip;
        self
    }
}

/// One drawing command in a stroked path. Coordinates are logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// A cubic Bezier segment: first control, second control, then end point.
    CubicTo(f32, f32, f32, f32, f32, f32),
}

/// A single-colour stroked path. It is useful for lightweight diagrams such as a commit graph.
#[derive(Clone, Debug)]
pub struct Path {
    pub commands: Vec<PathCommand>,
    pub color: Color,
    pub width: f32,
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
}

impl Path {
    pub fn stroke(color: Color, width: f32) -> Path {
        Path { commands: Vec::new(), color, width, clip: None }
    }

    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::MoveTo(x, y));
        self
    }

    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::LineTo(x, y));
        self
    }

    pub fn cubic_to(mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::CubicTo(cx0, cy0, cx1, cy1, x, y));
        self
    }

    pub fn clipped(mut self, clip: Option<Rect>) -> Self {
        self.clip = clip;
        self
    }

    pub(crate) fn translated(mut self, x: f32, y: f32) -> Self {
        for command in &mut self.commands {
            match command {
                PathCommand::MoveTo(px, py) | PathCommand::LineTo(px, py) => {
                    *px += x;
                    *py += y;
                }
                PathCommand::CubicTo(cx0, cy0, cx1, cy1, px, py) => {
                    *cx0 += x;
                    *cy0 += y;
                    *cx1 += x;
                    *cy1 += y;
                    *px += x;
                    *py += y;
                }
            }
        }
        self
    }
}

/// One thing to draw.
#[derive(Clone, Debug)]
pub enum Item {
    Quad(Quad),
    Text(Text),
    Path(Path),
    /// A vector icon tinted with one color; see [`crate::icon`].
    Icon(crate::icon::Icon),
    /// A pixel image the app owns; see [`crate::image`].
    Image(crate::image::ImageDraw),
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

    pub fn push_path(&mut self, path: Path) {
        self.items.push(Item::Path(path));
    }

    pub fn push_icon(&mut self, icon: crate::icon::Icon) {
        self.items.push(Item::Icon(icon));
    }

    pub fn push_image(&mut self, image: crate::image::ImageDraw) {
        self.items.push(Item::Image(image));
    }

    pub fn quads(&self) -> impl Iterator<Item = &Quad> {
        self.items.iter().filter_map(|i| if let Item::Quad(q) = i { Some(q) } else { None })
    }

    pub fn texts(&self) -> impl Iterator<Item = &Text> {
        self.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t) } else { None })
    }

    pub fn icons(&self) -> impl Iterator<Item = &crate::icon::Icon> {
        self.items.iter().filter_map(|i| if let Item::Icon(icon) = i { Some(icon) } else { None })
    }

    pub fn images(&self) -> impl Iterator<Item = &crate::image::ImageDraw> {
        self.items.iter().filter_map(|i| if let Item::Image(image) = i { Some(image) } else { None })
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.items.iter().filter_map(|i| if let Item::Path(p) = i { Some(p) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_gradient_is_recorded_and_defaults_to_none() {
        let (a, b) = (Color::hex(0xff0000), Color::hex(0x0000ff));
        let quad = Quad::new(Rect::new(0., 0., 10., 10.), a);
        assert!(quad.gradients.is_empty());
        let one = quad.gradient(150., a, b).gradients;
        assert_eq!(one.as_slice(), &[Gradient::linear(150.).stop(a).stop(b)]);
    }

    #[test]
    fn a_quad_shadow_is_recorded_and_defaults_to_none() {
        let black = Color::hex(0).with_alpha(0.3);
        let quad = Quad::new(Rect::new(0., 0., 10., 10.), black);
        assert!(quad.shadows.is_empty());
        assert_eq!(quad.shadowed(0., 8., 24., black).shadows.as_slice(), &[Shadow::new(0., 8., 24., 0., black)]);
    }

    #[test]
    fn a_quad_inset_shadow_is_separate_from_the_outer_shadow() {
        let white = Color::hex(0xffffff).with_alpha(0.6);
        let quad = Quad::new(Rect::new(0., 0., 10., 10.), white).inset_shadowed(0., 1., 0., white);
        assert_eq!(quad.inset_shadows.as_slice(), &[Shadow::new(0., 1., 0., 0., white)]);
        assert!(quad.shadows.is_empty());
    }

    #[test]
    fn a_quad_backdrop_blur_is_recorded_and_defaults_to_none() {
        let quad = Quad::new(Rect::new(0., 0., 10., 10.), Color::hex(0xffffff).with_alpha(0.3));
        assert_eq!(quad.backdrop_blur, None);
        assert_eq!(quad.backdrop_blurred(24., 1.9).backdrop_blur, Some(BackdropBlur { radius: 24., saturate: 1.9 }));
    }

    fn positions(g: &Gradient) -> Vec<f32> {
        g.resolved_stops().iter().map(|(p, _)| *p).collect()
    }

    #[test]
    fn missing_stop_positions_are_spread_evenly_like_css() {
        let c = Color::hex(0xffffff);
        assert_eq!(positions(&Gradient::linear(0.).stop(c).stop(c)), vec![0., 1.]);
        assert_eq!(positions(&Gradient::linear(0.).stop(c).stop(c).stop(c)), vec![0., 0.5, 1.]);
        assert_eq!(positions(&Gradient::linear(0.).stop(c).stop(c).stop(c).stop(c).stop(c)), vec![0., 0.25, 0.5, 0.75, 1.]);
        // linear-gradient(c, c 20%, c, c 80%, c): the unpositioned one in the middle sits halfway.
        let g = Gradient::linear(0.).stop(c).stop_at(c, 0.2).stop(c).stop_at(c, 0.8).stop(c);
        let p = positions(&g);
        assert!([0., 0.2, 0.5, 0.8, 1.].iter().zip(&p).all(|(a, b)| (a - b).abs() < 1e-6), "{p:?}");
        // A position behind its predecessor is pushed up to it, and out-of-range ones are clamped.
        assert_eq!(positions(&Gradient::linear(0.).stop_at(c, 0.6).stop_at(c, 0.2).stop_at(c, 3.)), vec![0.6, 0.6, 1.]);
        assert!(Gradient::linear(0.).resolved_stops().is_empty());
        assert_eq!(positions(&Gradient::linear(0.).stop(c)), vec![0.], "a single stop is a flat color");
    }

    #[test]
    fn gradients_and_layers_are_capped() {
        let c = Color::hex(0);
        let mut g = Gradient::radial((0.5, 0.5), RadialSize::ClosestSide);
        for _ in 0..20 {
            g = g.stop(c);
        }
        assert_eq!(g.stops.len(), MAX_GRADIENT_STOPS);
        let mut quad = Quad::new(Rect::default(), c);
        for _ in 0..9 {
            quad = quad.gradient_layer(g);
        }
        assert_eq!(quad.gradients.len(), MAX_GRADIENT_LAYERS);
        let layered = quad.gradient(90., c, c);
        assert_eq!(layered.gradients.len(), 1, "the old two-stop call replaces the layers");
    }

    #[test]
    fn a_path_keeps_its_commands_and_can_be_translated() {
        let path = Path::stroke(Color::hex(0x123456), 2.)
            .move_to(1., 2.)
            .cubic_to(3., 4., 5., 6., 7., 8.)
            .translated(10., 20.);
        assert_eq!(path.commands, vec![
            PathCommand::MoveTo(11., 22.),
            PathCommand::CubicTo(13., 24., 15., 26., 17., 28.),
        ]);
    }
}
