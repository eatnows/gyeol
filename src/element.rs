//! Describing a UI: [`div`] and [`text`] elements with a chainable style API.
//!
//! An [`Element<S>`] tree is rebuilt from the app state `S` every frame. Handlers such as
//! [`Element::on_click`] receive `&mut S`, so an interaction is just a change to the state; the next
//! frame's tree reflects it.
use std::hash::{Hash, Hasher};

use crate::{scene::Color, shell::Cx};

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
}

/// Everything about how an element is laid out and painted. Edges are `[top, right, bottom, left]`.
#[derive(Clone, Debug)]
pub struct Style {
    pub direction: Direction,
    pub wrap: bool,
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
    pub border_width: f32,
    pub border_color: Color,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    pub hover_background: Option<Color>,
    /// Inherited by descendants that do not set their own.
    pub text_color: Option<Color>,
    pub text_size: Option<f32>,
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
            border_width: 0.,
            border_color: Color::TRANSPARENT,
            radii: [0.; 4],
            hover_background: None,
            text_color: None,
            text_size: None,
            cursor: Cursor::Default,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            scrollbar: None,
        }
    }
}

pub type Handler<S> = Box<dyn Fn(&mut S, &mut Cx)>;

pub(crate) enum Kind<S> {
    Div(Vec<Element<S>>),
    Text(String),
}

/// A node of the UI tree. Build one with [`div`] or [`text`].
pub struct Element<S> {
    pub(crate) id: Option<ElementId>,
    pub(crate) style: Style,
    pub(crate) kind: Kind<S>,
    pub(crate) on_click: Option<Handler<S>>,
}

/// A container. Children stack vertically; call [`Element::row`] for a horizontal row.
pub fn div<S>() -> Element<S> {
    Element { id: None, style: Style::default(), kind: Kind::Div(Vec::new()), on_click: None }
}

/// A single line of text, in the size and color inherited from its parents.
pub fn text<S>(content: impl Into<String>) -> Element<S> {
    Element { id: None, style: Style::default(), kind: Kind::Text(content.into()), on_click: None }
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

    // ---- layout ----------------------------------------------------------------------------

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
    pub fn grow(mut self) -> Self {
        self.style.grow = 1.;
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

    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.style.cursor = cursor;
        self
    }
}
