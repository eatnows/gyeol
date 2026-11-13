//! Describing a UI: [`div`] and [`text`] elements with a chainable style API.
//!
//! An [`Element<S>`] tree is rebuilt from the app state `S` every frame. Handlers such as
//! [`Element::on_click`] receive `&mut S`, so an interaction is just a change to the state; the next
//! frame's tree reflects it.
use std::hash::{Hash, Hasher};

use crate::{
    icon::IconData,
    event::{Modifiers, MouseButton},
    pointer::PointerKind,
    scene::{BackdropBlur, Capped, Color, Gradient, Path, Shadow, MAX_GRADIENT_LAYERS, MAX_SHADOWS},
    shell::Cx,
};

/// A size along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// As big as the content (or the flex algorithm) says.
    Auto,
    /// Logical pixels.
    Px(f32),
    /// A fraction of the parent, `0.0..=1.0`.
    Fraction(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

/// Placement across the main axis' perpendicular ("cross axis").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
    Stretch,
}

/// Placement along the main axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// What happens to content that does not fit inside an element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overflow {
    /// Drawn outside the element's box.
    #[default]
    Visible,
    /// Cut off at the element's box.
    Hidden,
    /// Cut off, and scrollable with the wheel or trackpad. Give the element an [`Element::id`] so its
    /// scroll position survives from frame to frame.
    Scroll,
}

impl Overflow {
    /// Clipping applies on both axes, so an axis left visible becomes hidden.
    fn max_hidden(self) -> Overflow {
        if self == Overflow::Visible { Overflow::Hidden } else { self }
    }
}

/// Identifies an element across frames (for scroll positions). Made from any hashable value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElementId(pub u64);

impl ElementId {
    pub fn new(value: impl Hash) -> ElementId {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        ElementId(hasher.finish())
    }
}

/// The mouse cursor shown over an element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    /// East-west arrows, for dragging a vertical divider (CSS `col-resize`).
    ResizeHorizontal,
    /// North-south arrows, for dragging a horizontal divider (CSS `row-resize`).
    ResizeVertical,
    /// An open hand: something that can be dragged (CSS `grab`).
    Grab,
    /// A closed hand: something being dragged (CSS `grabbing`).
    Grabbing,
    /// A circle with a slash: the action is not available (CSS `not-allowed`).
    NotAllowed,
}

/// Everything about how an element is laid out and painted. Edges are `[top, right, bottom, left]`.
#[derive(Clone, Debug)]
pub struct Style {
    pub direction: Direction,
    pub wrap: bool,
    /// Taken out of the flow and placed by [`Style::inset`] relative to its parent.
    pub absolute: bool,
    /// Painted above everything else and not clipped by ancestors; see [`Element::overlay`].
    pub overlay: bool,
    /// Distances from the parent's edges when `absolute`: top, right, bottom, left.
    pub inset: [Option<f32>; 4],
    pub gap: f32,
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub width: Length,
    pub height: Length,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    pub grow: f32,
    pub shrink: f32,
    /// The size along the main axis before free space is shared out; `Auto` means the content's size.
    pub basis: Length,
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub justify: Option<Justify>,
    pub background: Option<Color>,
    /// Gradient layers over `background`, bottom first.
    pub gradients: Capped<Gradient, MAX_GRADIENT_LAYERS>,
    /// Outer shadows, bottom first.
    pub shadows: Capped<Shadow, MAX_SHADOWS>,
    /// Inset shadows, bottom first.
    pub inset_shadows: Capped<Shadow, MAX_SHADOWS>,
    pub backdrop_blur: Option<BackdropBlur>,
    pub border_width: f32,
    pub border_color: Color,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    pub hover_background: Option<Color>,
    /// Multiplies the alpha of this element and everything below it; 1 is opaque. See [`Element::opacity`].
    pub opacity: f32,
    /// Paint overrides applied while the pointer is over the element or one of its descendants.
    pub hover: HoverStyle,
    /// Inherited by descendants that do not set their own.
    pub text_color: Option<Color>,
    pub text_size: Option<f32>,
    pub text_family: Option<&'static str>,
    pub text_bold: Option<bool>,
    pub text_italic: Option<bool>,
    pub cursor: Cursor,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// Color of the scroll thumb drawn while the content overflows; none draws no thumb.
    pub scrollbar: Option<Color>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            direction: Direction::Column,
            wrap: false,
            absolute: false,
            overlay: false,
            inset: [None; 4],
            gap: 0.,
            padding: [0.; 4],
            margin: [0.; 4],
            width: Length::Auto,
            height: Length::Auto,
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            grow: 0.,
            shrink: 0.,
            basis: Length::Auto,
            align_items: None,
            align_self: None,
            justify: None,
            background: None,
            gradients: Capped::default(),
            shadows: Capped::default(),
            inset_shadows: Capped::default(),
            backdrop_blur: None,
            border_width: 0.,
            border_color: Color::TRANSPARENT,
            radii: [0.; 4],
            hover_background: None,
            opacity: 1.,
            hover: HoverStyle::default(),
            text_color: None,
            text_size: None,
            text_family: None,
            text_bold: None,
            text_italic: None,
            cursor: Cursor::Default,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            scrollbar: None,
        }
    }
}

/// Paint-affecting style that replaces the element's own while it is hovered (the pointer is over it
/// or a descendant, like CSS `:hover`). Everything unset keeps the normal style. Set through
/// [`Element::hover`].
///
/// Layer lists replace the normal ones whole, so `bg_layers([])` removes the gradients on hover
/// and `shadows([glow])` swaps the shadows for one glow.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoverStyle {
    pub background: Option<Color>,
    pub gradients: Option<Capped<Gradient, MAX_GRADIENT_LAYERS>>,
    pub shadows: Option<Capped<Shadow, MAX_SHADOWS>>,
    pub inset_shadows: Option<Capped<Shadow, MAX_SHADOWS>>,
    pub border_color: Option<Color>,
    /// Replaces the element's own [`Element::opacity`] while hovered.
    pub opacity: Option<f32>,
    /// The text color of the element's text and icons, and of descendants that do not set their own.
    pub text_color: Option<Color>,
}

impl HoverStyle {
    pub fn bg(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Replaces the gradient layers (bottom first); an empty list removes them.
    pub fn bg_layers(mut self, layers: impl IntoIterator<Item = Gradient>) -> Self {
        self.gradients = Some(layers.into_iter().collect());
        self
    }

    /// Replaces the outer shadows; an empty list removes them.
    pub fn shadows(mut self, shadows: impl IntoIterator<Item = Shadow>) -> Self {
        self.shadows = Some(shadows.into_iter().collect());
        self
    }

    /// Replaces the inset shadows; an empty list removes them.
    pub fn inset_shadows(mut self, shadows: impl IntoIterator<Item = Shadow>) -> Self {
        self.inset_shadows = Some(shadows.into_iter().collect());
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = Some(opacity.clamp(0., 1.));
        self
    }

    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }
}

pub type Handler<S> = Box<dyn Fn(&mut S, &mut Cx)>;

/// A mouse press, move or release aimed at an element.
#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    /// Position in the window, in logical pixels.
    pub pos: (f32, f32),
    /// Position relative to the element's top-left corner (scrolling included).
    pub local: (f32, f32),
    /// The element's size in logical pixels, so `local.0 / size.0` is how far across it the pointer
    /// is (a slider or scrub bar). While the pointer is captured by a drag this is the captured
    /// element's size, wherever the pointer is.
    pub size: (f32, f32),
    pub button: MouseButton,
    /// What is pointing: a mouse, a pen tip or a pen's eraser. See [`crate::pointer`] for platform support.
    pub pointer: PointerKind,
    /// 0 to 1. A mouse reports 1 while a button is down, 0 otherwise; a pen reports its pressure.
    pub pressure: f32,
    /// How far a pen leans (x and y, -1 to 1); `None` for a mouse or a pen that cannot tell. Every sample
    /// since the previous event is in [`Cx::pointer_samples`].
    pub tilt: Option<(f32, f32)>,
    /// 1 for a single click, 2 for a double click, and so on.
    pub click_count: u32,
    pub modifiers: Modifiers,
}

pub type MouseHandler<S> = Box<dyn Fn(&mut S, &mut Cx, MouseEvent)>;

pub(crate) enum Kind<S> {
    Div(Vec<Element<S>>),
    Text(String),
    Paths(Vec<Path>),
    Icon(std::sync::Arc<IconData>),
}

/// A node of the UI tree. Build one with [`div`] or [`text`].
pub struct Element<S> {
    pub(crate) id: Option<ElementId>,
    pub(crate) style: Style,
    pub(crate) kind: Kind<S>,
    pub(crate) on_click: Option<Handler<S>>,
    pub(crate) on_mouse_down: Option<MouseHandler<S>>,
    pub(crate) on_drag: Option<MouseHandler<S>>,
    pub(crate) on_mouse_up: Option<MouseHandler<S>>,
    pub(crate) on_mouse_down_outside: Option<Handler<S>>,
}

/// A container. Children stack vertically; call [`Element::row`] for a horizontal row.
pub fn div<S>() -> Element<S> {
    Element { id: None, style: Style::default(), kind: Kind::Div(Vec::new()), on_click: None, on_mouse_down: None, on_drag: None, on_mouse_up: None, on_mouse_down_outside: None }
}

/// A single line of text, in the size and color inherited from its parents.
pub fn text<S>(content: impl Into<String>) -> Element<S> {
    Element { id: None, style: Style::default(), kind: Kind::Text(content.into()), on_click: None, on_mouse_down: None, on_drag: None, on_mouse_up: None, on_mouse_down_outside: None }
}

/// A layout element that paints stroked paths in its own local coordinate system.
///
/// Paths are moved to the element's top-left after layout, so they naturally participate in
/// absolute positioning, scrolling and clipping like any other element.
pub fn paths<S>(items: Vec<Path>) -> Element<S> {
    Element { id: None, style: Style::default(), kind: Kind::Paths(items), on_click: None, on_mouse_down: None, on_drag: None, on_mouse_up: None, on_mouse_down_outside: None }
}

/// A vector icon, drawn crisply at any size and device scale in the text color it inherits (SVG
/// `currentColor`). It is a normal layout box: `.size(16.)`, `.w()`/`.h()` set its size (the default is
/// the SVG's `viewBox` size), and `.text_color(c)` on it or an ancestor tints it. Needs the `svg` feature;
/// see [`IconData::parse`] for the SVG subset.
///
/// ```ignore
/// icon(include_str!("chevron-right.svg")).size(16.).text_color(Color::hex(0xffffff))
/// ```
#[cfg(feature = "svg")]
pub fn icon<S>(svg: &str) -> Element<S> {
    icon_from(IconData::cached(svg))
}

/// Like [`icon`], for an icon that is already parsed (or built by hand), without the `svg` feature.
pub fn icon_from<S>(data: std::sync::Arc<IconData>) -> Element<S> {
    let mut style = Style::default();
    style.width = Length::Px(data.view_box.w);
    style.height = Length::Px(data.view_box.h);
    Element { id: None, style, kind: Kind::Icon(data), on_click: None, on_mouse_down: None, on_drag: None, on_mouse_up: None, on_mouse_down_outside: None }
}

impl<S> Element<S> {
    /// Names this element so state kept for it (its scroll position) survives from frame to frame.
    /// Ids must be unique among the elements that need one.
    pub fn id(mut self, id: impl Hash) -> Self {
        self.id = Some(ElementId::new(id));
        self
    }

    pub fn child(mut self, child: Element<S>) -> Self {
        if let Kind::Div(children) = &mut self.kind {
            children.push(child);
        }
        self
    }

    pub fn children(mut self, more: impl IntoIterator<Item = Element<S>>) -> Self {
        if let Kind::Div(children) = &mut self.kind {
            children.extend(more);
        }
        self
    }

    /// Called when the primary mouse button is pressed and released over this element (or a descendant
    /// without a handler of its own).
    pub fn on_click(mut self, handler: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self.style.cursor = Cursor::Pointer;
        self
    }

    /// Called when a mouse button goes down over this element (or a descendant without a handler of
    /// its own). If the element also has [`Element::on_drag`] or [`Element::on_mouse_up`], it then
    /// receives the mouse until the button is released, even outside its bounds. Such elements need
    /// an [`Element::id`] so the drag survives the tree being rebuilt each frame.
    pub fn on_mouse_down(mut self, handler: impl Fn(&mut S, &mut Cx, MouseEvent) + 'static) -> Self {
        self.on_mouse_down = Some(Box::new(handler));
        self
    }

    /// Called for every mouse move while a press that started on this element is held.
    pub fn on_drag(mut self, handler: impl Fn(&mut S, &mut Cx, MouseEvent) + 'static) -> Self {
        self.on_drag = Some(Box::new(handler));
        self
    }

    /// Called when the button of a press that started on this element is released.
    /// Runs when the mouse is pressed anywhere but on this element or one of its descendants, so a menu
    /// or popover can close itself. "On" means what the pointer would hit: a press inside this element's
    /// box that lands on an [`Element::overlay`] above it (which is not its descendant) counts as outside,
    /// and a press on a descendant that sticks out of the box does not. Fires for every mouse button, before
    /// the press is delivered to the element that was hit.
    pub fn on_mouse_down_outside(mut self, handler: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_mouse_down_outside = Some(Box::new(handler));
        self
    }

    pub fn on_mouse_up(mut self, handler: impl Fn(&mut S, &mut Cx, MouseEvent) + 'static) -> Self {
        self.on_mouse_up = Some(Box::new(handler));
        self
    }

    // ---- layout ----------------------------------------------------------------------------

    /// Takes this element out of its parent's flow and positions it with [`Element::top`],
    /// [`Element::left`], ... relative to the parent (for overlays, carets, popups). It paints above
    /// the siblings that come before it.
    pub fn absolute(mut self) -> Self {
        self.style.absolute = true;
        self
    }

    /// Makes this element a layer that floats over the whole window, for dropdown menus, popovers and
    /// tooltips:
    ///
    /// - it is positioned like an [`Element::absolute`] element, relative to its parent's box
    ///   (`.top()`, `.left()`, ... place it), and takes no room in the layout;
    /// - it and its subtree paint above everything that is not an overlay, wherever it sits in the tree,
    ///   and are **not** clipped by `overflow_hidden` or scrolling ancestors;
    /// - it gets pointer events before the elements underneath: they are neither hovered nor clicked
    ///   where the overlay covers them (even where it is transparent);
    /// - of several overlays, the one later in the tree is above the earlier ones, and an overlay inside
    ///   another is above it.
    ///
    /// Events still bubble to the overlay's ancestors in the tree, so a button that contains its own menu
    /// stays hovered while the menu is. Pair with [`Element::on_mouse_down_outside`] to close it.
    pub fn overlay(mut self) -> Self {
        self.style.overlay = true;
        self
    }

    pub fn top(mut self, px: f32) -> Self {
        self.style.inset[0] = Some(px);
        self
    }

    pub fn right(mut self, px: f32) -> Self {
        self.style.inset[1] = Some(px);
        self
    }

    pub fn bottom(mut self, px: f32) -> Self {
        self.style.inset[2] = Some(px);
        self
    }

    pub fn left(mut self, px: f32) -> Self {
        self.style.inset[3] = Some(px);
        self
    }

    /// Absolute, filling the parent (all four edges at `px`).
    pub fn inset(mut self, px: f32) -> Self {
        self.style.absolute = true;
        self.style.inset = [Some(px); 4];
        self
    }

    pub fn row(mut self) -> Self {
        self.style.direction = Direction::Row;
        self
    }

    pub fn col(mut self) -> Self {
        self.style.direction = Direction::Column;
        self
    }

    pub fn wrap(mut self) -> Self {
        self.style.wrap = true;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn p(mut self, all: f32) -> Self {
        self.style.padding = [all; 4];
        self
    }

    pub fn px(mut self, x: f32) -> Self {
        self.style.padding[1] = x;
        self.style.padding[3] = x;
        self
    }

    pub fn py(mut self, y: f32) -> Self {
        self.style.padding[0] = y;
        self.style.padding[2] = y;
        self
    }

    pub fn pt(mut self, v: f32) -> Self {
        self.style.padding[0] = v;
        self
    }

    pub fn pb(mut self, v: f32) -> Self {
        self.style.padding[2] = v;
        self
    }

    pub fn m(mut self, all: f32) -> Self {
        self.style.margin = [all; 4];
        self
    }

    pub fn ml(mut self, v: f32) -> Self {
        self.style.margin[3] = v;
        self
    }

    pub fn mr(mut self, v: f32) -> Self {
        self.style.margin[1] = v;
        self
    }

    pub fn mt(mut self, v: f32) -> Self {
        self.style.margin[0] = v;
        self
    }

    pub fn mb(mut self, v: f32) -> Self {
        self.style.margin[2] = v;
        self
    }

    pub fn w(mut self, px: f32) -> Self {
        self.style.width = Length::Px(px);
        self
    }

    pub fn h(mut self, px: f32) -> Self {
        self.style.height = Length::Px(px);
        self
    }

    pub fn size(self, px: f32) -> Self {
        self.w(px).h(px)
    }

    /// Width as a fraction of the parent's width, `0.0..=1.0` (`w_frac(0.37)` is `width: 37%`). The parent
    /// needs a definite width for this to resolve.
    pub fn w_frac(mut self, fraction: f32) -> Self {
        self.style.width = Length::Fraction(fraction);
        self
    }

    /// Height as a fraction of the parent's height, `0.0..=1.0`.
    pub fn h_frac(mut self, fraction: f32) -> Self {
        self.style.height = Length::Fraction(fraction);
        self
    }

    pub fn w_full(mut self) -> Self {
        self.style.width = Length::Fraction(1.);
        self
    }

    pub fn h_full(mut self) -> Self {
        self.style.height = Length::Fraction(1.);
        self
    }

    pub fn min_w(mut self, px: f32) -> Self {
        self.style.min_width = Some(px);
        self
    }

    pub fn min_h(mut self, px: f32) -> Self {
        self.style.min_height = Some(px);
        self
    }

    pub fn max_w(mut self, px: f32) -> Self {
        self.style.max_width = Some(px);
        self
    }

    pub fn max_h(mut self, px: f32) -> Self {
        self.style.max_height = Some(px);
        self
    }

    /// Takes a share of the free space along the parent's main axis, starting from nothing (like CSS
    /// `flex: 1`), so its own content size does not push the others out.
    pub fn grow(self) -> Self {
        self.grow_by(1.)
    }

    /// Like [`Element::grow`] with a flex-grow factor: free space is shared out in proportion to the
    /// factors (`grow_by(2.)` takes twice what a `grow()` sibling does).
    pub fn grow_by(mut self, factor: f32) -> Self {
        self.style.grow = factor;
        self.style.shrink = 1.;
        self.style.basis = Length::Px(0.);
        self
    }

    /// Lets this element get smaller than its size when its parent runs out of room. Elements keep
    /// their size by default.
    pub fn shrink(mut self) -> Self {
        self.style.shrink = 1.;
        self
    }

    pub fn items(mut self, align: Align) -> Self {
        self.style.align_items = Some(align);
        self
    }

    pub fn items_center(self) -> Self {
        self.items(Align::Center)
    }

    pub fn align_self(mut self, align: Align) -> Self {
        self.style.align_self = Some(align);
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = Some(justify);
        self
    }

    pub fn justify_center(self) -> Self {
        self.justify(Justify::Center)
    }

    pub fn justify_between(self) -> Self {
        self.justify(Justify::SpaceBetween)
    }

    pub fn justify_end(self) -> Self {
        self.justify(Justify::End)
    }

    // ---- overflow ----------------------------------------------------------------------------

    /// Cuts off content outside this element's box.
    pub fn overflow_hidden(mut self) -> Self {
        self.style.overflow_x = Overflow::Hidden;
        self.style.overflow_y = Overflow::Hidden;
        self
    }

    /// Scrolls vertically when the content is taller than the element. Also give it a fixed or
    /// bounded height and an [`Element::id`].
    pub fn overflow_y_scroll(mut self) -> Self {
        self.style.overflow_y = Overflow::Scroll;
        self.style.overflow_x = self.style.overflow_x.max_hidden();
        self
    }

    pub fn overflow_x_scroll(mut self) -> Self {
        self.style.overflow_x = Overflow::Scroll;
        self.style.overflow_y = self.style.overflow_y.max_hidden();
        self
    }

    /// Draws a scroll thumb in `color` while the content overflows.
    pub fn scrollbar(mut self, color: Color) -> Self {
        self.style.scrollbar = Some(color);
        self
    }

    // ---- painting --------------------------------------------------------------------------

    pub fn bg(mut self, color: Color) -> Self {
        self.style.background = Some(color);
        self
    }

    /// Fills with a two-stop linear gradient (CSS angle in degrees: 0 up, 90 right, 180 down) instead of
    /// the background color and any layers set so far. For stops, radial gradients or stacking use
    /// [`Element::bg_layer`] / [`Element::bg_layers`].
    pub fn bg_gradient(mut self, angle: f32, from: Color, to: Color) -> Self {
        self.style.background = None;
        self.style.gradients.clear();
        self.style.gradients.push(Gradient::linear(angle).stop(from).stop(to));
        self
    }

    /// Paints `gradient` over the background color and the layers added so far (CSS multiple
    /// backgrounds, but bottom first: CSS lists the top layer first). At most 4 layers are kept.
    pub fn bg_layer(mut self, gradient: Gradient) -> Self {
        self.style.gradients.push(gradient);
        self
    }

    /// Replaces the gradient layers, bottom first.
    pub fn bg_layers(mut self, layers: impl IntoIterator<Item = Gradient>) -> Self {
        self.style.gradients = layers.into_iter().collect();
        self
    }

    /// Sets the one blurred drop shadow (CSS `box-shadow: x y blur color`), replacing any set so far.
    /// For a spread or several shadows use [`Element::shadow_layer`] / [`Element::shadows`].
    pub fn shadow(mut self, x: f32, y: f32, blur: f32, color: Color) -> Self {
        self.style.shadows.clear();
        self.style.shadows.push(Shadow::new(x, y, blur, 0., color));
        self
    }

    /// Adds an outer shadow over the ones added so far (up to 4). CSS lists the top shadow first; here
    /// the first added is at the bottom.
    pub fn shadow_layer(mut self, shadow: Shadow) -> Self {
        self.style.shadows.push(shadow);
        self
    }

    /// Replaces the outer shadows, bottom first.
    pub fn shadows(mut self, shadows: impl IntoIterator<Item = Shadow>) -> Self {
        self.style.shadows = shadows.into_iter().collect();
        self
    }

    /// Sets the one inner shadow or highlight (CSS `box-shadow: inset x y blur color`), replacing any
    /// set so far. For a spread or several use [`Element::inset_shadow_layer`] / [`Element::inset_shadows`].
    pub fn inset_shadow(mut self, x: f32, y: f32, blur: f32, color: Color) -> Self {
        self.style.inset_shadows.clear();
        self.style.inset_shadows.push(Shadow::new(x, y, blur, 0., color));
        self
    }

    /// Adds an inset shadow over the ones added so far (up to 4).
    pub fn inset_shadow_layer(mut self, shadow: Shadow) -> Self {
        self.style.inset_shadows.push(shadow);
        self
    }

    /// Replaces the inset shadows, bottom first.
    pub fn inset_shadows(mut self, shadows: impl IntoIterator<Item = Shadow>) -> Self {
        self.style.inset_shadows = shadows.into_iter().collect();
        self
    }

    /// Blurs and saturates what is painted behind the element (CSS `backdrop-filter: blur(r) saturate(s)`),
    /// within its rounded shape, before its own background is drawn. `radius` is the gaussian's standard
    /// deviation in logical pixels.
    pub fn backdrop_blur(mut self, radius: f32, saturate: f32) -> Self {
        self.style.backdrop_blur = Some(BackdropBlur { radius, saturate });
        self
    }

    /// Fades this element and its whole subtree (CSS `opacity`): the alpha of backgrounds, gradients,
    /// shadows, borders, text, icons and paths is multiplied by `opacity` (`0.0..=1.0`), and by the
    /// opacity of every ancestor.
    ///
    /// This is a per-item approximation: each thing is faded on its own, so where two items inside the
    /// subtree overlap you see the lower one through the upper one, which a true group opacity (render the
    /// subtree, then fade the result) would hide. A backdrop blur is not faded.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.style.opacity = opacity.clamp(0., 1.);
        self
    }

    /// Overrides paint while hovered: `div().hover(|h| h.bg_layers([glow]).shadows([halo]))`. The
    /// element repaints when the pointer enters and leaves. Works alongside [`Element::hover_bg`].
    pub fn hover(mut self, style: impl FnOnce(HoverStyle) -> HoverStyle) -> Self {
        self.style.hover = style(self.style.hover);
        self
    }

    pub fn hover_bg(mut self, color: Color) -> Self {
        self.style.hover_background = Some(color);
        self
    }

    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.style.border_width = width;
        self.style.border_color = color;
        self
    }

    pub fn rounded(mut self, radius: f32) -> Self {
        self.style.radii = [radius; 4];
        self
    }

    /// Sets each corner's radius: top-left, top-right, bottom-right, bottom-left.
    pub fn rounded_corners(mut self, radii: [f32; 4]) -> Self {
        self.style.radii = radii;
        self
    }

    /// Rounds only the top corners (e.g. a header inside a rounded box).
    pub fn rounded_top(mut self, radius: f32) -> Self {
        self.style.radii[0] = radius;
        self.style.radii[1] = radius;
        self
    }

    /// Rounds only the bottom corners.
    pub fn rounded_bottom(mut self, radius: f32) -> Self {
        self.style.radii[2] = radius;
        self.style.radii[3] = radius;
        self
    }

    pub fn text_color(mut self, color: Color) -> Self {
        self.style.text_color = Some(color);
        self
    }

    pub fn text_size(mut self, size: f32) -> Self {
        self.style.text_size = Some(size);
        self
    }

    /// Sets the font family (by name) for this element's text and its descendants'.
    pub fn text_family(mut self, family: &'static str) -> Self {
        self.style.text_family = Some(family);
        self
    }

    pub fn text_bold(mut self) -> Self {
        self.style.text_bold = Some(true);
        self
    }

    pub fn text_italic(mut self) -> Self {
        self.style.text_italic = Some(true);
        self
    }

    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.style.cursor = cursor;
        self
    }
}
