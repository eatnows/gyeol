//! Turns an [`Element`] tree into positions (via taffy's flexbox) and a [`Scene`].
use std::collections::HashMap;

use taffy::{prelude::*, tree::LayoutOutput, TaffyTree};

use crate::{
    element::{Align, Cursor, Direction, Element, ElementId, Handler, HoverStyle, Justify, Kind, Length, MouseHandler, Overflow, Style},
    scene::{BackdropBlur, Capped, Color, Gradient, Path, Quad, Rect, Scene, Shadow, Text, MAX_GRADIENT_LAYERS, MAX_SHADOWS},
    shaper::{Shaper, TextStyle},
};

/// Text properties that flow down from ancestors to the text inside them.
#[derive(Clone, Copy)]
struct Inherited {
    color: Color,
    style: TextStyle,
}

impl Default for Inherited {
    fn default() -> Self {
        Inherited { color: Color::hex(0x000000), style: TextStyle::new(14.) }
    }
}

const THUMB_WIDTH: f32 = 6.;
const THUMB_MIN: f32 = 24.;

/// Scroll positions of the elements that scroll, kept across frames.
pub(crate) type ScrollStore = HashMap<ElementId, (f32, f32)>;

struct TextLeaf {
    content: String,
    style: TextStyle,
}

/// One laid-out element, in paint order (parents before their children).
pub(crate) struct Node<S> {
    pub bounds: Rect,
    parent: Option<usize>,
    background: Option<Color>,
    hover_background: Option<Color>,
    hover: HoverStyle,
    opacity: f32,
    /// Whether `text_color` was set on this element itself (descendants inherit it).
    explicit_text_color: bool,
    gradients: Capped<Gradient, MAX_GRADIENT_LAYERS>,
    shadows: Capped<Shadow, MAX_SHADOWS>,
    inset_shadows: Capped<Shadow, MAX_SHADOWS>,
    backdrop_blur: Option<BackdropBlur>,
    border_width: f32,
    border_color: Color,
    radii: [f32; 4],
    text: Option<(String, TextStyle, Color)>,
    paths: Vec<Path>,
    /// A vector icon and the tint it inherited.
    icon: Option<(std::sync::Arc<crate::icon::IconData>, Color)>,
    /// A pixel image and how it is placed.
    image: Option<(crate::image::ImageBuffer, crate::image::ImageView)>,
    cursor: Cursor,
    on_click: Option<Handler<S>>,
    on_mouse_down: Option<MouseHandler<S>>,
    on_drag: Option<MouseHandler<S>>,
    on_mouse_up: Option<MouseHandler<S>>,
    on_mouse_down_outside: Option<Handler<S>>,
    overlay: bool,
    taffy_id: NodeId,
    // ---- overflow and scrolling
    id: Option<ElementId>,
    overflow: (Overflow, Overflow),
    scrollbar: Option<Color>,
    /// How far the content is scrolled, and the furthest it can go.
    scroll_offset: (f32, f32),
    scroll_max: (f32, f32),
    /// The area this element itself may paint in (set by ancestors that cut off their content).
    clip: Option<Rect>,
    /// Where children are positioned from (this element's top-left, moved by the scroll offset).
    content_origin: (f32, f32),
    /// The area children may paint in.
    child_clip: Option<Rect>,
}

impl<S> Node<S> {
    fn clips(&self) -> bool {
        self.overflow.0 != Overflow::Visible || self.overflow.1 != Overflow::Visible
    }

    fn scrolls(&self) -> (bool, bool) {
        (self.overflow.0 == Overflow::Scroll, self.overflow.1 == Overflow::Scroll)
    }
}

/// A reference to an element that outlives one frame's layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Capture {
    Id(ElementId),
    /// Position in the arena, valid only while the tree keeps the same size.
    Index(usize, usize),
}

/// The result of laying out one frame: kept until the next frame so mouse input can be resolved
/// against what the user actually sees.
pub(crate) struct Laid<S> {
    nodes: Vec<Node<S>>,
    /// Node indices from the back to the front of the window: normal elements in tree order, then each
    /// overlay's subtree (overlays in tree order). Painting walks it forwards, hit-testing backwards.
    order: Vec<usize>,
}

impl<S> Laid<S> {
    /// The frontmost visible element under `pos` (content cut off by a scroll container does not count).
    fn topmost_at(&self, pos: (f32, f32)) -> Option<usize> {
        self.order.iter().rev().copied().find(|&i| {
            let n = &self.nodes[i];
            n.bounds.contains(pos) && n.clip.is_none_or(|c| c.contains(pos))
        })
    }

    /// `index` and all its ancestors.
    fn chain(&self, index: Option<usize>) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(index, |&i| self.nodes[i].parent)
    }

    /// The element that would receive a click at `pos`: the frontmost one with a handler, looking up
    /// through its ancestors.
    pub fn click_target(&self, pos: (f32, f32)) -> Option<usize> {
        self.chain(self.topmost_at(pos)).find(|&i| self.nodes[i].on_click.is_some())
    }

    pub fn handler(&self, index: usize) -> Option<&Handler<S>> {
        self.nodes[index].on_click.as_ref()
    }

    /// The element a press at `pos` goes to: the frontmost one with a mouse-down handler, looking up
    /// through its ancestors.
    pub fn press_target(&self, pos: (f32, f32)) -> Option<usize> {
        self.chain(self.topmost_at(pos)).find(|&i| self.nodes[i].on_mouse_down.is_some())
    }

    pub fn on_mouse_down(&self, index: usize) -> Option<&MouseHandler<S>> {
        self.nodes[index].on_mouse_down.as_ref()
    }

    pub fn on_drag(&self, index: usize) -> Option<&MouseHandler<S>> {
        self.nodes[index].on_drag.as_ref()
    }

    pub fn on_mouse_up(&self, index: usize) -> Option<&MouseHandler<S>> {
        self.nodes[index].on_mouse_up.as_ref()
    }

    /// Whether a press on `index` should keep receiving mouse moves and the release.
    pub fn captures(&self, index: usize) -> bool {
        self.nodes[index].on_drag.is_some() || self.nodes[index].on_mouse_up.is_some()
    }

    /// How to find `index` again next frame: by its id when it has one, by position otherwise.
    pub fn key_of(&self, index: usize) -> Capture {
        self.nodes[index].id.map_or(Capture::Index(index, self.nodes.len()), Capture::Id)
    }

    /// The element a capture refers to in this frame, if it still exists.
    pub fn resolve(&self, capture: Capture) -> Option<usize> {
        match capture {
            Capture::Id(id) => self.nodes.iter().position(|n| n.id == Some(id)),
            Capture::Index(i, len) => (len == self.nodes.len()).then_some(i),
        }
    }

    pub fn bounds_of(&self, index: usize) -> Rect {
        self.nodes[index].bounds
    }

    /// The handlers of the elements a press at `pos` is outside of: not on them or their descendants.
    pub fn outside_handlers(&self, pos: (f32, f32)) -> Vec<&Handler<S>> {
        let hit: Vec<usize> = self.chain(self.topmost_at(pos)).collect();
        self.nodes.iter().enumerate().filter(|(i, _)| !hit.contains(i)).filter_map(|(_, n)| n.on_mouse_down_outside.as_ref()).collect()
    }

    /// The cursor to show at `pos`: that of the innermost element that asks for one.
    pub fn cursor_at(&self, pos: (f32, f32)) -> Cursor {
        self.chain(self.topmost_at(pos)).map(|i| self.nodes[i].cursor).find(|c| *c != Cursor::Default).unwrap_or_default()
    }

    /// The viewport (position and size) of every scroll container in this frame.
    pub fn viewports(&self) -> impl Iterator<Item = (ElementId, Rect)> + '_ {
        self.nodes.iter().filter_map(|n| n.id.filter(|_| n.scrolls() != (false, false)).map(|id| (id, n.bounds)))
    }

    /// The viewport size and furthest scroll offsets of the container `id` in this frame.
    pub fn scroll_limits(&self, id: ElementId) -> Option<((f32, f32), (f32, f32))> {
        self.nodes.iter().find(|n| n.id == Some(id)).map(|n| ((n.bounds.w, n.bounds.h), n.scroll_max))
    }

    /// Scrolls whatever is under `pos` by `delta` logical pixels (positive scrolls towards the start).
    /// Each axis goes to the innermost scroll container that can still move that way, so a list that
    /// hit its end hands the rest to the container around it.
    pub fn scroll_by(&self, pos: (f32, f32), delta: (f32, f32), store: &mut ScrollStore) {
        for axis in 0..2 {
            let d = if axis == 0 { delta.0 } else { delta.1 };
            if d == 0. {
                continue;
            }
            for i in self.chain(self.topmost_at(pos)) {
                let node = &self.nodes[i];
                let scrollable = if axis == 0 { node.scrolls().0 } else { node.scrolls().1 };
                let Some(id) = node.id.filter(|_| scrollable) else { continue };
                let (cur, max) = if axis == 0 { (node.scroll_offset.0, node.scroll_max.0) } else { (node.scroll_offset.1, node.scroll_max.1) };
                let next = (cur - d).clamp(0., max);
                if next != cur {
                    let entry = store.entry(id).or_insert(node.scroll_offset);
                    if axis == 0 { entry.0 = next } else { entry.1 = next }
                    break;
                }
            }
        }
    }
}

/// Lays `root` out in a `size` window and paints it, marking elements under `mouse` as hovered.
pub(crate) fn layout_and_paint<S>(
    root: Element<S>,
    size: (f32, f32),
    mouse: (f32, f32),
    background: Color,
    shaper: &mut Shaper,
    scroll: &mut ScrollStore,
) -> (Laid<S>, Scene) {
    let mut tree: TaffyTree<TextLeaf> = TaffyTree::new();
    let mut nodes: Vec<Node<S>> = Vec::new();

    let mut root = root;
    root.style.width = Length::Px(size.0);
    root.style.height = Length::Px(size.1);
    let root_id = build(root, None, Inherited::default(), &mut tree, &mut nodes);

    let available = taffy::Size { width: AvailableSpace::Definite(size.0), height: AvailableSpace::Definite(size.1) };
    let _ = tree.compute_layout_with_measure(root_id, available, |input, _, leaf, _| {
        let (w, h) = match leaf {
            Some(leaf) => (shaper.width(&leaf.content, leaf.style), Shaper::line_height(leaf.style.size)),
            None => (0., 0.),
        };
        LayoutOutput::from_outer_size(taffy::Size {
            width: input.known_dimensions.width.unwrap_or(w),
            height: input.known_dimensions.height.unwrap_or(h),
        })
    });

    // Absolute positions. A node's layout is relative to its parent, whose content may be scrolled.
    for i in 0..nodes.len() {
        let layout = tree.layout(nodes[i].taffy_id).expect("node exists");
        let (origin, parent_clip) = nodes[i].parent.map_or(((0., 0.), None), |p| (nodes[p].content_origin, nodes[p].child_clip));
        // An overlay floats above its ancestors' clips.
        let parent_clip = if nodes[i].overlay { None } else { parent_clip };
        let bounds = Rect::new(origin.0 + layout.location.x, origin.1 + layout.location.y, layout.size.width, layout.size.height);

        let (scroll_x, scroll_y) = nodes[i].scrolls();
        let max = (if scroll_x { layout.scroll_width() } else { 0. }, if scroll_y { layout.scroll_height() } else { 0. });
        // A scroll container without an id still scrolls, but its position is lost when the tree changes.
        let key = (scroll_x || scroll_y).then(|| nodes[i].id.unwrap_or(ElementId::new(("gyeol-anonymous-scroll", i))));
        let offset = key.map_or((0., 0.), |k| {
            let stored = scroll.get(&k).copied().unwrap_or((0., 0.));
            let clamped = (stored.0.clamp(0., max.0), stored.1.clamp(0., max.1));
            scroll.insert(k, clamped);
            clamped
        });
        let bw = nodes[i].border_width;
        let inner = Rect::new(bounds.x + bw, bounds.y + bw, (bounds.w - 2. * bw).max(0.), (bounds.h - 2. * bw).max(0.));

        let node = &mut nodes[i];
        node.id = key.or(node.id);
        node.bounds = bounds;
        node.clip = parent_clip;
        node.scroll_offset = offset;
        node.scroll_max = max;
        node.content_origin = (bounds.x - offset.0, bounds.y - offset.1);
        node.child_clip = if node.clips() { Some(parent_clip.map_or(inner, |c| c.intersect(&inner))) } else { parent_clip };
    }

    // Where each element's subtree ends, so scroll thumbs can be painted on top of it.
    let mut end: Vec<usize> = (0..nodes.len()).map(|i| i + 1).collect();
    for i in (0..nodes.len()).rev() {
        if let Some(p) = nodes[i].parent {
            end[p] = end[p].max(end[i]);
        }
    }

    // The layer each node paints in: 0 for normal elements, otherwise the index of its innermost overlay
    // ancestor-or-self. Sorting by it (stably) keeps tree order within a layer and puts overlays last.
    let mut layer = vec![0usize; nodes.len()];
    for i in 0..nodes.len() {
        layer[i] = if nodes[i].overlay { i } else { nodes[i].parent.map_or(0, |p| layer[p]) };
    }
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| layer[i]);
    let laid = Laid { nodes, order };
    let hovered: Vec<usize> = laid.chain(laid.topmost_at(mouse)).collect();
    // The text color of node `i`: the nearest hovered ancestor-or-self that overrides it on hover, unless
    // an element between them sets its own color; otherwise the color it inherited when built.
    let text_color = |i: usize, inherited: Color| {
        for a in laid.chain(Some(i)) {
            if let Some(color) = hovered.contains(&a).then(|| laid.nodes[a].hover.text_color).flatten() {
                return color;
            }
            if laid.nodes[a].explicit_text_color {
                break;
            }
        }
        inherited
    };
    // Each element's opacity times its ancestors' (parents come first), honoring hover overrides.
    let mut fade = vec![1f32; laid.nodes.len()];
    for (i, node) in laid.nodes.iter().enumerate() {
        let own = if hovered.contains(&i) { node.hover.opacity.unwrap_or(node.opacity) } else { node.opacity };
        fade[i] = node.parent.map_or(1., |p| fade[p]) * own;
    }
    let mut scene = Scene { background: Some(background), ..Default::default() };
    let mut thumbs: Vec<(usize, Quad)> = Vec::new();
    let mut current_layer = 0;
    for &i in &laid.order {
        let node = &laid.nodes[i];
        if layer[i] != current_layer {
            // Moving on to an overlay: finish the scroll thumbs of the layer below it.
            while let Some((_, thumb)) = thumbs.pop() {
                scene.push_quad(thumb);
            }
            current_layer = layer[i];
        }
        while thumbs.last().is_some_and(|(at, _)| *at <= i) {
            scene.push_quad(thumbs.pop().expect("checked").1);
        }
        // Hover overrides (the pointer is over this element or a descendant) replace the normal paint.
        let hover = hovered.contains(&i).then_some(&node.hover);
        let fill = match hover {
            Some(h) => h.background.or(node.hover_background).or(node.background),
            None => node.background,
        };
        let gradients = hover.and_then(|h| h.gradients).unwrap_or(node.gradients);
        let shadows = hover.and_then(|h| h.shadows).unwrap_or(node.shadows);
        let inset_shadows = hover.and_then(|h| h.inset_shadows).unwrap_or(node.inset_shadows);
        let border_color = hover.and_then(|h| h.border_color).unwrap_or(node.border_color);
        if fill.is_some() || !gradients.is_empty() || !shadows.is_empty() || !inset_shadows.is_empty() || node.backdrop_blur.is_some() || node.border_width > 0. {
            let f = fade[i];
            let mut quad = Quad::new(node.bounds, fill.unwrap_or(Color::TRANSPARENT).faded(f)).rounded_corners(node.radii).clipped(node.clip);
            if f < 1. {
                // Fade every paint that has alpha.
                quad.gradients = gradients.iter().map(|g| g.faded(f)).collect();
                quad.shadows = shadows.iter().map(|s| s.faded(f)).collect();
                quad.inset_shadows = inset_shadows.iter().map(|s| s.faded(f)).collect();
            } else {
                quad.gradients = gradients;
                quad.shadows = shadows;
                quad.inset_shadows = inset_shadows;
            }
            quad.backdrop_blur = node.backdrop_blur;
            if node.border_width > 0. {
                quad = quad.bordered(node.border_width, border_color.faded(f));
            }
            scene.push_quad(quad);
        }
        if let Some((content, size, color)) = &node.text {
            scene.push_text(Text::new((node.bounds.x, node.bounds.y), content.clone(), *size, text_color(i, *color).faded(fade[i])).clipped(node.clip));
        }
        for path in &node.paths {
            let mut path = path.clone().translated(node.bounds.x, node.bounds.y).clipped(node.clip);
            path.color = path.color.faded(fade[i]);
            scene.push_path(path);
        }
        if let Some((data, color)) = &node.icon {
            scene.push_icon(crate::icon::Icon::new(data.clone(), node.bounds, text_color(i, *color).faded(fade[i])).clipped(node.clip));
        }
        if let Some((buffer, view)) = &node.image {
            // The image never paints outside its element.
            let clip = Some(node.clip.map_or(node.bounds, |c| c.intersect(&node.bounds)));
            scene.push_image(crate::image::ImageDraw {
                buffer: buffer.clone(),
                origin: (node.bounds.x + view.pan.0, node.bounds.y + view.pan.1),
                zoom: view.zoom,
                filter: view.filter,
                checkerboard: view.checkerboard,
                opacity: fade[i],
                clip,
            });
        }
        if let (Some(color), true) = (node.scrollbar, node.scroll_max.1 > 0.) {
            let (view_h, content_h) = (node.bounds.h, node.bounds.h + node.scroll_max.1);
            let thumb_h = (view_h * view_h / content_h).max(THUMB_MIN).min(view_h);
            let y = node.bounds.y + (node.scroll_offset.1 / node.scroll_max.1) * (view_h - thumb_h);
            let x = node.bounds.x + node.bounds.w - THUMB_WIDTH - 2.;
            let thumb = Quad::new(Rect::new(x, y, THUMB_WIDTH, thumb_h), color.faded(fade[i])).rounded(THUMB_WIDTH / 2.).clipped(node.clip);
            thumbs.push((end[i], thumb));
            thumbs.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        }
    }
    while let Some((_, thumb)) = thumbs.pop() {
        scene.push_quad(thumb);
    }
    (laid, scene)
}

fn build<S>(
    el: Element<S>,
    parent: Option<usize>,
    inherited: Inherited,
    tree: &mut TaffyTree<TextLeaf>,
    nodes: &mut Vec<Node<S>>,
) -> NodeId {
    let Element { id, style, kind, on_click, on_mouse_down, on_drag, on_mouse_up, on_mouse_down_outside } = el;
    let mut text_style = inherited.style;
    text_style.size = style.text_size.unwrap_or(text_style.size);
    text_style.family = style.text_family.or(text_style.family);
    text_style.bold = style.text_bold.unwrap_or(text_style.bold);
    text_style.italic = style.text_italic.unwrap_or(text_style.italic);
    let text_color = style.text_color.unwrap_or(inherited.color);
    let below = Inherited { color: text_color, style: text_style };
    let index = nodes.len();
    let taffy_style = to_taffy(&style);

    // The arena slot is reserved first so children can point at their parent; the taffy id is filled in below.
    nodes.push(Node {
        bounds: Rect::default(),
        parent,
        background: style.background,
        hover_background: style.hover_background,
        hover: style.hover,
        opacity: style.opacity,
        explicit_text_color: style.text_color.is_some(),
        gradients: style.gradients,
        shadows: style.shadows,
        inset_shadows: style.inset_shadows,
        backdrop_blur: style.backdrop_blur,
        border_width: style.border_width,
        border_color: style.border_color,
        radii: style.radii,
        text: None,
        paths: Vec::new(),
        icon: None,
        image: None,
        cursor: style.cursor,
        on_click,
        on_mouse_down,
        on_drag,
        on_mouse_up,
        on_mouse_down_outside,
        overlay: style.overlay,
        taffy_id: NodeId::new(0),
        id,
        overflow: (style.overflow_x, style.overflow_y),
        scrollbar: style.scrollbar,
        scroll_offset: (0., 0.),
        scroll_max: (0., 0.),
        clip: None,
        content_origin: (0., 0.),
        child_clip: None,
    });

    let taffy_id = match kind {
        Kind::Text(content) => {
            nodes[index].text = Some((content.clone(), text_style, text_color));
            tree.new_leaf_with_context(taffy_style, TextLeaf { content, style: text_style }).expect("leaf")
        }
        Kind::Div(children) => {
            let ids: Vec<NodeId> = children.into_iter().map(|c| build(c, Some(index), below, tree, nodes)).collect();
            tree.new_with_children(taffy_style, &ids).expect("container")
        }
        Kind::Paths(paths) => {
            nodes[index].paths = paths;
            tree.new_leaf(taffy_style).expect("path leaf")
        }
        Kind::Icon(data) => {
            nodes[index].icon = Some((data, text_color));
            tree.new_leaf(taffy_style).expect("icon leaf")
        }
        Kind::Image(buffer, view) => {
            nodes[index].image = Some((buffer, view));
            tree.new_leaf(taffy_style).expect("image leaf")
        }
    };
    nodes[index].taffy_id = taffy_id;
    taffy_id
}

fn to_taffy(s: &Style) -> taffy::Style {
    let dimension = |l: Length| match l {
        Length::Auto => Dimension::auto(),
        Length::Px(v) => Dimension::length(v),
        Length::Fraction(f) => Dimension::percent(f),
    };
    let bound = |v: Option<f32>| v.map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length);
    let edges = |e: [f32; 4]| taffy::Rect {
        top: LengthPercentage::length(e[0]),
        right: LengthPercentage::length(e[1]),
        bottom: LengthPercentage::length(e[2]),
        left: LengthPercentage::length(e[3]),
    };

    let overflow = |o: Overflow| match o {
        Overflow::Visible => taffy::Overflow::Visible,
        Overflow::Hidden => taffy::Overflow::Hidden,
        Overflow::Scroll => taffy::Overflow::Scroll,
    };

    let mut t = taffy::Style::default();
    t.display = Display::Flex;
    t.overflow = taffy::Point { x: overflow(s.overflow_x), y: overflow(s.overflow_y) };
    t.scrollbar_width = 0.;
    t.flex_direction = match s.direction {
        Direction::Row => FlexDirection::Row,
        Direction::Column => FlexDirection::Column,
    };
    if s.absolute || s.overlay {
        t.position = Position::Absolute;
    }
    let inset = |v: Option<f32>| v.map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length);
    t.inset = taffy::Rect { top: inset(s.inset[0]), right: inset(s.inset[1]), bottom: inset(s.inset[2]), left: inset(s.inset[3]) };
    t.flex_wrap = if s.wrap { FlexWrap::Wrap } else { FlexWrap::NoWrap };
    t.gap = taffy::Size { width: LengthPercentage::length(s.gap), height: LengthPercentage::length(s.gap) };
    t.padding = edges(s.padding);
    t.border = taffy::Rect {
        top: LengthPercentage::length(s.border_width),
        right: LengthPercentage::length(s.border_width),
        bottom: LengthPercentage::length(s.border_width),
        left: LengthPercentage::length(s.border_width),
    };
    t.margin = taffy::Rect {
        top: LengthPercentageAuto::length(s.margin[0]),
        right: LengthPercentageAuto::length(s.margin[1]),
        bottom: LengthPercentageAuto::length(s.margin[2]),
        left: LengthPercentageAuto::length(s.margin[3]),
    };
    t.size = taffy::Size { width: dimension(s.width), height: dimension(s.height) };
    t.min_size = taffy::Size { width: bound(s.min_width), height: bound(s.min_height) };
    t.max_size = taffy::Size { width: bound(s.max_width), height: bound(s.max_height) };
    t.flex_grow = s.grow;
    t.flex_shrink = s.shrink;
    t.flex_basis = dimension(s.basis);
    let align = |a: Align| match a {
        Align::Start => AlignItems::FLEX_START,
        Align::End => AlignItems::FLEX_END,
        Align::Center => AlignItems::CENTER,
        Align::Stretch => AlignItems::STRETCH,
    };
    t.align_items = s.align_items.map(align);
    t.align_self = s.align_self.map(align);
    t.justify_content = s.justify.map(|j| match j {
        Justify::Start => JustifyContent::FLEX_START,
        Justify::End => JustifyContent::FLEX_END,
        Justify::Center => JustifyContent::CENTER,
        Justify::SpaceBetween => JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => JustifyContent::SPACE_EVENLY,
    });
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{div, text};

    const SIZE: (f32, f32) = (400., 300.);
    const NOWHERE: (f32, f32) = (-1., -1.);

    fn lay(root: Element<()>, shaper: &mut Shaper) -> (Laid<()>, Scene) {
        layout_and_paint(root, SIZE, NOWHERE, Color::hex(0xffffff), shaper, &mut ScrollStore::new())
    }

    fn b(laid: &Laid<()>, i: usize) -> (f32, f32, f32, f32) {
        let r = laid.nodes[i].bounds;
        (r.x, r.y, r.w, r.h)
    }

    #[test]
    fn a_column_stacks_children_inside_padding_with_gaps() {
        let mut shaper = Shaper::new();
        let root = div().p(10.).gap(5.).child(div().h(20.).w_full()).child(div().h(30.));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 0), (0., 0., 400., 300.), "the root fills the window");
        assert_eq!(b(&laid, 1), (10., 10., 380., 20.));
        assert_eq!(b(&laid, 2), (10., 35., 380., 30.), "stretched across, below the first plus the gap");
    }

    #[test]
    fn grow_takes_the_free_space_of_a_row() {
        let mut shaper = Shaper::new();
        let root = div().row().gap(10.).child(div().w(100.)).child(div().grow());
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1).2, 100.);
        assert_eq!(b(&laid, 2), (110., 0., 290., 300.));
    }

    #[test]
    fn centering_on_both_axes() {
        let mut shaper = Shaper::new();
        let root = div().items_center().justify_center().child(div().w(50.).h(20.));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1), (175., 140., 50., 20.));
    }

    #[test]
    fn justify_between_pushes_children_apart() {
        let mut shaper = Shaper::new();
        let root = div().row().justify_between().child(div().w(40.).h(10.)).child(div().w(40.).h(10.));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1).0, 0.);
        assert_eq!(b(&laid, 2).0, 360.);
    }

    #[test]
    fn a_border_pushes_content_inward() {
        let mut shaper = Shaper::new();
        let root = div().border(2., Color::hex(0)).child(div().h(10.));
        let (laid, scene) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1), (2., 2., 396., 10.));
        assert_eq!(scene.quads().count(), 1, "only the bordered element paints a quad");
        assert_eq!(scene.quads().next().unwrap().border_width, 2.);
    }

    #[test]
    fn text_is_as_big_as_the_shaper_says() {
        let mut shaper = Shaper::new();
        let root = div().row().items_center().text_size(20.).child(text("Hello 결"));
        let (laid, scene) = lay(root, &mut shaper);
        let (_, _, w, h) = b(&laid, 1);
        assert!((w - shaper.width("Hello 결", 20.)).abs() <= 1., "layout rounds to whole pixels: {w}");
        assert_eq!(h, Shaper::line_height(20.));
        assert_eq!(scene.texts().count(), 1);
        assert_eq!(scene.texts().next().unwrap().style.size, 20., "size is inherited from the container");
    }

    #[test]
    fn text_color_is_inherited_and_can_be_overridden() {
        let mut shaper = Shaper::new();
        let red = Color::hex(0xff0000);
        let blue = Color::hex(0x0000ff);
        let root = div().text_color(red).child(text("a")).child(text("b").text_color(blue));
        let (_, scene) = lay(root, &mut shaper);
        let colors: Vec<Color> = scene.texts().map(|t| t.color).collect();
        assert_eq!(colors, vec![red, blue]);
    }

    #[test]
    fn hover_changes_paint_and_clicks_go_to_the_frontmost_handler() {
        let mut shaper = Shaper::new();
        let idle = Color::hex(0xeeeeee);
        let hot = Color::hex(0xff8800);
        let build = || -> Element<()> {
            div()
                .row()
                .child(div().w(100.).h(50.).bg(idle).hover_bg(hot).on_click(|_, _| {}).child(div().size(10.)))
                .child(div().w(100.).h(50.).bg(idle).hover_bg(hot))
        };

        let (laid, scene) = layout_and_paint(build(), SIZE, (50., 25.), Color::hex(0xffffff), &mut shaper, &mut ScrollStore::new());
        let quads: Vec<&Quad> = scene.quads().collect();
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].background, hot, "the pointer is over the first box");
        assert_eq!(quads[1].background, idle);
        assert_eq!(laid.click_target((5., 5.)), Some(1), "a click on the inner child bubbles up to the handler");
        assert_eq!(laid.click_target((150., 25.)), None, "the second box has no handler");
        assert_eq!(laid.cursor_at((50., 25.)), Cursor::Pointer);
        assert_eq!(laid.cursor_at((150., 25.)), Cursor::Default);

        let (_, away) = layout_and_paint(build(), SIZE, (350., 200.), Color::hex(0xffffff), &mut shaper, &mut ScrollStore::new());
        assert_eq!(away.quads().next().unwrap().background, idle, "no hover when the pointer is elsewhere");
    }

    #[test]
    fn a_gradient_background_reaches_the_painted_quad() {
        let mut shaper = Shaper::new();
        let (a, b) = (Color::hex(0xffffff), Color::hex(0x000000));
        let (_, scene) = lay(div().w(50.).h(50.).bg_gradient(140., a, b), &mut shaper);
        assert_eq!(scene.quads().next().unwrap().gradients.as_slice(), &[Gradient::linear(140.).stop(a).stop(b)]);
    }

    #[test]
    fn a_shadow_reaches_the_painted_quad_even_without_a_fill() {
        let mut shaper = Shaper::new();
        let color = Color::hex(0x000000).with_alpha(0.25);
        let (_, scene) = lay(div().w(50.).h(50.).shadow(0., 4., 12., color), &mut shaper);
        let quad = scene.quads().next().expect("a shadow alone still paints a quad");
        assert_eq!(quad.shadows.as_slice(), &[Shadow::new(0., 4., 12., 0., color)]);
    }

    #[test]
    fn an_inset_shadow_reaches_the_painted_quad() {
        let mut shaper = Shaper::new();
        let white = Color::hex(0xffffff).with_alpha(0.6);
        let (_, scene) = lay(div().w(50.).h(50.).bg(Color::hex(0x336699)).inset_shadow(0., 1., 0., white), &mut shaper);
        let quad = scene.quads().next().unwrap();
        assert_eq!(quad.inset_shadows.as_slice(), &[Shadow::new(0., 1., 0., 0., white)]);
        assert!(quad.shadows.is_empty());
    }

    #[test]
    fn a_backdrop_blur_reaches_the_painted_quad_after_what_is_behind_it() {
        let mut shaper = Shaper::new();
        let root = div()
            .child(div().w(100.).h(100.).bg(Color::hex(0xff0000)))
            .child(div().w(50.).h(50.).bg(Color::hex(0xffffff).with_alpha(0.3)).backdrop_blur(24., 1.9));
        let (_, scene) = lay(root, &mut shaper);
        let quads: Vec<&Quad> = scene.quads().collect();
        assert_eq!(quads[0].backdrop_blur, None);
        assert_eq!(quads[1].backdrop_blur, Some(BackdropBlur { radius: 24., saturate: 1.9 }), "later items blur earlier ones");
    }

    #[test]
    fn background_layers_stack_in_the_order_given_over_the_background_color() {
        let mut shaper = Shaper::new();
        let glow = Gradient::radial((0.1, 0.1), crate::scene::RadialSize::Ellipse(500., 360.)).stop(Color::hex(0xff00ff)).stop_at(Color::TRANSPARENT, 0.7);
        let base = Gradient::linear(160.).stop(Color::hex(0x111122)).stop(Color::hex(0x221133));
        let el = div().w(50.).h(50.).bg(Color::hex(0x000000)).bg_layer(base).bg_layer(glow);
        let (_, scene) = lay(el, &mut shaper);
        let quad = scene.quads().next().unwrap();
        assert_eq!(quad.background, Color::hex(0x000000));
        assert_eq!(quad.gradients.as_slice(), &[base, glow]);

        let (_, scene) = lay(div().w(50.).h(50.).bg(Color::hex(0)).bg_layer(base).bg_layers([glow]), &mut shaper);
        assert_eq!(scene.quads().next().unwrap().gradients.as_slice(), &[glow], "bg_layers replaces");
        let (_, scene) = lay(div().w(50.).h(50.).bg(Color::hex(0)).bg_gradient(90., Color::hex(1), Color::hex(2)), &mut shaper);
        let quad = scene.quads().next().unwrap();
        assert_eq!((quad.background, quad.gradients.len()), (Color::TRANSPARENT, 1), "bg_gradient replaces the color");
    }

    #[test]
    fn several_shadows_with_spreads_reach_the_quad_in_order() {
        let mut shaper = Shaper::new();
        let (a, b) = (Shadow::new(0., 2., 4., 0., Color::hex(0)), Shadow::new(0., 30., 80., -10., Color::hex(0x111111)));
        let inner = Shadow::new(0., 1., 0., 0., Color::hex(0xffffff));
        let el = div().w(50.).h(50.).shadow_layer(a).shadow_layer(b).inset_shadows([inner, inner]);
        let (_, scene) = lay(el, &mut shaper);
        let quad = scene.quads().next().unwrap();
        assert_eq!(quad.shadows.as_slice(), &[a, b]);
        assert_eq!(quad.inset_shadows.len(), 2);
        let (_, scene) = lay(div().w(50.).h(50.).shadow_layer(a).shadow(1., 1., 1., Color::hex(0)), &mut shaper);
        assert_eq!(scene.quads().next().unwrap().shadows.len(), 1, "the old call replaces");
    }

    fn square_icon(size: f32) -> std::sync::Arc<crate::icon::IconData> {
        use crate::icon::{IconCommand, IconData, IconShape, IconStroke, LineCap, LineJoin};
        let stroke = IconStroke { width: 2., cap: LineCap::Round, join: LineJoin::Round, miter_limit: 4., opacity: 1. };
        let shape = IconShape { commands: vec![IconCommand::MoveTo(2., 2.), IconCommand::LineTo(size - 2., size - 2.)], transform: [1., 0., 0., 1., 0., 0.], fill: None, stroke: Some(stroke) };
        std::sync::Arc::new(IconData::new(Rect::new(0., 0., size, size), vec![shape]))
    }

    #[test]
    fn an_icon_is_a_layout_box_sized_by_its_view_box_or_size_and_tinted_by_the_inherited_color() {
        use crate::element::icon_from;
        let mut shaper = Shaper::new();
        let red = Color::hex(0xff0000);
        let blue = Color::hex(0x0000ff);
        let root = div()
            .row()
            .gap(10.)
            .text_color(red)
            .child(icon_from(square_icon(24.)))
            .child(icon_from(square_icon(24.)).size(16.).text_color(blue))
            .child(div().w(30.).h(30.));
        let (laid, scene) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1), (0., 0., 24., 24.), "defaults to the viewBox size");
        assert_eq!(b(&laid, 2), (34., 0., 16., 16.), ".size() overrides it and the box takes part in the flex layout");
        assert_eq!(b(&laid, 3), (60., 0., 30., 30.));
        let icons: Vec<_> = scene.icons().collect();
        assert_eq!(icons.len(), 2);
        assert_eq!((icons[0].bounds, icons[0].color), (Rect::new(0., 0., 24., 24.), red), "inherits the text color");
        assert_eq!((icons[1].bounds, icons[1].color), (Rect::new(34., 0., 16., 16.), blue), "its own color wins");
    }

    #[test]
    fn icons_are_clipped_scrolled_and_hit_like_any_box() {
        use crate::element::icon_from;
        let mut shaper = Shaper::new();
        let mut store = ScrollStore::new();
        let row = |i: usize| icon_from(square_icon(24.)).on_click(move |_, _| { let _ = i; });
        let root: Element<()> = div().child(div().id("s").h(40.).overflow_y_scroll().child(row(0)).child(row(1)).child(row(2)));
        let (laid, scene) = layout_and_paint(root, SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut store);
        let clips: Vec<_> = scene.icons().map(|i| i.clip).collect();
        assert_eq!(clips.len(), 3);
        assert!(clips.iter().all(|c| *c == Some(Rect::new(0., 0., 400., 40.))), "cut off at the container: {clips:?}");
        assert_eq!(laid.click_target((5., 5.)), Some(2), "an icon can be a click target (the first icon is node 2)");
    }

    #[test]
    fn a_hover_style_replaces_gradients_shadows_and_borders_only_while_hovered() {
        let mut shaper = Shaper::new();
        let (idle_line, glow) = (Color::hex(0x333333), Color::hex(0xff8800));
        let accent = Gradient::linear(90.).stop(Color::TRANSPARENT).stop(glow).stop(Color::TRANSPARENT);
        let halo = Shadow::new(0., 0., 12., 2., glow.with_alpha(0.6));
        let build = || -> Element<()> {
            div().col().child(
                div()
                    .h(2.)
                    .bg(idle_line)
                    .border(1., idle_line)
                    .shadow_layer(Shadow::new(0., 1., 2., 0., Color::hex(0)))
                    .hover(|h| h.bg_layers([accent]).shadows([halo]).inset_shadows([halo]).border_color(glow).bg(Color::hex(0x111111))),
            )
        };
        let paint = |mouse| layout_and_paint(build(), SIZE, mouse, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
        let _ = &mut shaper;

        let idle = paint(NOWHERE);
        let q = idle.quads().next().unwrap();
        assert_eq!((q.background, q.border_color), (idle_line, idle_line));
        assert!(q.gradients.is_empty() && q.shadows.len() == 1 && q.inset_shadows.is_empty());

        let hot = paint((50., 1.));
        let q = hot.quads().next().unwrap();
        assert_eq!(q.gradients.as_slice(), &[accent]);
        assert_eq!(q.shadows.as_slice(), &[halo], "replaced, not added to");
        assert_eq!(q.inset_shadows.as_slice(), &[halo]);
        assert_eq!((q.background, q.border_color), (Color::hex(0x111111), glow));
    }

    #[test]
    fn an_empty_hover_list_removes_the_layers_and_hover_bg_keeps_working() {
        let el = || -> Element<()> {
            div().child(div().h(10.).bg(Color::hex(0x222222)).hover_bg(Color::hex(0x444444)).bg_layer(Gradient::linear(0.).stop(Color::hex(1)).stop(Color::hex(2))).hover(|h| h.bg_layers([])))
        };
        let paint = |mouse| layout_and_paint(el(), SIZE, mouse, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
        assert_eq!(paint(NOWHERE).quads().next().unwrap().gradients.len(), 1);
        let hot = paint((5., 5.));
        let q = hot.quads().next().unwrap();
        assert!(q.gradients.is_empty());
        assert_eq!(q.background, Color::hex(0x444444), "hover_bg still applies without a hover background override");

        // A background in the hover style wins over hover_bg.
        let both = || -> Element<()> { div().child(div().h(10.).bg(Color::hex(0x222222)).hover_bg(Color::hex(0x444444)).hover(|h| h.bg(Color::hex(0x666666)))) };
        let scene = layout_and_paint(both(), SIZE, (5., 5.), Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
        assert_eq!(scene.quads().next().unwrap().background, Color::hex(0x666666));
    }

    #[test]
    fn a_hover_text_color_reaches_text_and_icons_below_unless_they_set_their_own() {
        let (idle, hot, own) = (Color::hex(0x888888), Color::hex(0xffffff), Color::hex(0x00ff00));
        let el = || -> Element<()> {
            div().child(
                div()
                    .h(60.)
                    .text_color(idle)
                    .hover(|h| h.text_color(hot))
                    .child(text("inherits"))
                    .child(crate::element::icon_from(square_icon(16.)))
                    .child(text("own").text_color(own)),
            )
        };
        let colors = |mouse| {
            let scene = layout_and_paint(el(), SIZE, mouse, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
            (scene.texts().map(|t| t.color).collect::<Vec<_>>(), scene.icons().map(|i| i.color).collect::<Vec<_>>())
        };
        assert_eq!(colors(NOWHERE), (vec![idle, own], vec![idle]));
        assert_eq!(colors((5., 5.)), (vec![hot, own], vec![hot]), "text and icon follow the hover color, the explicit one keeps its own");
    }

    fn alpha_close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn opacity_multiplies_the_alpha_of_every_kind_of_item_in_the_subtree() {
        use crate::element::{icon_from, paths};
        let red = Color::hex(0xff0000);
        let glow = Color::hex(0x00ff00).with_alpha(0.8);
        let el = || -> Element<()> {
            div().child(
                div()
                    .opacity(0.5)
                    .col()
                    .child(
                        div()
                            .w(50.)
                            .h(20.)
                            .bg(red.with_alpha(0.6))
                            .border(1., red)
                            .bg_layer(Gradient::linear(90.).stop(red).stop(Color::TRANSPARENT))
                            .shadow_layer(Shadow::new(0., 2., 4., 0., glow))
                            .inset_shadow_layer(Shadow::new(0., 1., 0., 0., glow))
                            .opacity(0.5), // nested: 0.5 * 0.5
                    )
                    .child(text("hi").text_color(red))
                    .child(icon_from(square_icon(16.)).text_color(red))
                    .child(paths(vec![Path::stroke(red, 2.).move_to(0., 0.).line_to(10., 10.)])),
            )
        };
        let scene = layout_and_paint(el(), SIZE, NOWHERE, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
        let q = scene.quads().next().unwrap();
        assert!(alpha_close(q.background.a, 0.6 * 0.25), "bg: {}", q.background.a);
        assert!(alpha_close(q.border_color.a, 0.25));
        let stop_alphas: Vec<f32> = q.gradients.iter().flat_map(|g| g.stops.iter().map(|s| s.color.a)).collect();
        assert!(stop_alphas.iter().zip([0.25, 0.]).all(|(a, b)| alpha_close(*a, b)), "{stop_alphas:?}");
        assert!(alpha_close(q.shadows.as_slice()[0].color.a, 0.8 * 0.25) && alpha_close(q.inset_shadows.as_slice()[0].color.a, 0.8 * 0.25));
        assert!(alpha_close(scene.texts().next().unwrap().color.a, 0.5), "text takes the parent's 0.5");
        assert!(alpha_close(scene.icons().next().unwrap().color.a, 0.5));
        assert!(alpha_close(scene.paths().next().unwrap().color.a, 0.5));
        // Colors keep their hue, only alpha changes.
        assert_eq!((q.border_color.r, q.border_color.g), (1., 0.));
    }

    #[test]
    fn full_opacity_changes_nothing_and_zero_hides_everything_and_hover_can_fade_in() {
        let build = |o: f32| -> Element<()> { div().child(div().w(30.).h(30.).bg(Color::hex(0xff0000)).opacity(o).child(text("x"))) };
        let paint = |el, mouse| layout_and_paint(el, SIZE, mouse, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new()).1;
        let full = paint(build(1.), NOWHERE);
        assert_eq!((full.quads().next().unwrap().background.a, full.texts().next().unwrap().color.a), (1., 1.));
        let none = paint(build(0.), NOWHERE);
        assert_eq!((none.quads().next().unwrap().background.a, none.texts().next().unwrap().color.a), (0., 0.));
        assert_eq!(paint(build(7.), NOWHERE).quads().next().unwrap().background.a, 1., "clamped to 1");

        // A dim element that brightens while hovered.
        let el = || -> Element<()> { div().child(div().w(30.).h(30.).bg(Color::hex(0xff0000)).opacity(0.4).hover(|h| h.opacity(1.))) };
        assert!(alpha_close(paint(el(), NOWHERE).quads().next().unwrap().background.a, 0.4));
        assert_eq!(paint(el(), (5., 5.)).quads().next().unwrap().background.a, 1.);
    }

    #[test]
    fn fractions_resolve_against_the_parent_and_margins_and_grow_factors_shift_the_layout() {
        let mut shaper = Shaper::new();
        // A 200x40 track with a fill that is 37% wide and half as tall.
        let root = div().col().child(div().w(200.).h(40.).child(div().w_frac(0.37).h_frac(0.5).bg(Color::hex(0xff0000))));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 2), (0., 0., 74., 20.));

        // A row of three: a fixed box with margins, then flex children sharing the rest 1 : 2.
        let root = div().row().child(div().w(40.).h(10.).ml(5.).mr(15.)).child(div().h(10.).grow()).child(div().h(10.).grow_by(2.));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1), (5., 0., 40., 10.), "ml pushes it right");
        // 400 - (5 + 40 + 15) = 340 shared 1 : 2, starting after the right margin.
        let (second, third) = (b(&laid, 2), b(&laid, 3));
        assert_eq!(second.0, 60.);
        assert!((second.2 - 340. / 3.).abs() <= 1. && (third.2 - 680. / 3.).abs() <= 1., "{second:?} {third:?}");
        assert!((third.0 - (60. + second.2)).abs() <= 1.);
    }

    /// A 100x50 clipped container holding a quad and a menu overlay that sticks out of it, followed by
    /// a sibling that is painted later in the tree.
    fn with_menu() -> Element<()> {
        div()
            .col()
            .child(
                div()
                    .w(100.)
                    .h(50.)
                    .overflow_hidden()
                    .child(div().w(100.).h(50.).bg(Color::hex(0x111111)))
                    .child(div().overlay().left(60.).top(30.).w(100.).h(80.).bg(Color::hex(0xff0000)).on_click(|_, _| {}).child(div().w(20.).h(20.).bg(Color::hex(0x00ff00)))),
            )
            .child(div().w(200.).h(200.).bg(Color::hex(0x2222ff)).hover_bg(Color::hex(0x8888ff)).on_click(|_, _| {}))
    }

    #[test]
    fn an_overlay_paints_last_is_positioned_like_an_absolute_element_and_is_not_clipped() {
        let mut shaper = Shaper::new();
        let (laid, scene) = lay(with_menu(), &mut shaper);
        let colors: Vec<Color> = scene.quads().map(|q| q.background).collect();
        let hex = Color::hex;
        assert_eq!(colors, vec![hex(0x111111), hex(0x2222ff), hex(0xff0000), hex(0x00ff00)], "the overlay and its child come after the later sibling");
        let quads: Vec<&Quad> = scene.quads().collect();
        assert_eq!(quads[0].clip, Some(Rect::new(0., 0., 100., 50.)), "normal children are cut off at the container");
        assert_eq!(quads[2].clip, None, "an overlay ignores its ancestors' clips");
        assert_eq!(quads[3].clip, None, "and so do its descendants");
        assert_eq!(quads[2].bounds, Rect::new(60., 30., 100., 80.), "left/top relative to the parent, extending past it");
        assert_eq!(b(&laid, 5), (0., 50., 200., 200.), "the overlay takes no room in the flow: the sibling sits right below the container");
    }

    #[test]
    fn an_overlay_gets_hit_testing_priority_and_blocks_hover_and_clicks_underneath() {
        let mut shaper = Shaper::new();
        let paint = |mouse| layout_and_paint(with_menu(), SIZE, mouse, Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new());
        let _ = &mut shaper;
        // (80, 70) is outside the 100x50 container, over the sibling, and inside the overlay.
        let (laid, scene) = paint((80., 70.));
        let overlay = laid.click_target((80., 70.)).expect("something is clickable there");
        assert_eq!(laid.bounds_of(overlay), Rect::new(60., 30., 100., 80.), "the overlay, not the sibling below it");
        let sibling = scene.quads().find(|q| q.bounds.w == 200.).unwrap();
        assert_eq!(sibling.background, Color::hex(0x2222ff), "the covered sibling is not hovered");
        // Where the overlay does not reach, the sibling is clickable and hovers as usual.
        let (laid, scene) = paint((10., 120.));
        assert_eq!(laid.bounds_of(laid.click_target((10., 120.)).unwrap()), Rect::new(0., 50., 200., 200.));
        assert_eq!(scene.quads().find(|q| q.bounds.w == 200.).unwrap().background, Color::hex(0x8888ff));
    }

    #[test]
    fn later_overlays_are_above_earlier_ones_and_nested_overlays_above_their_parent() {
        let mut shaper = Shaper::new();
        let tile = |c: u32| div().overlay().left(0.).top(0.).w(50.).h(50.).bg(Color::hex(c)).on_click(|_, _| {});
        let root = div()
            .child(tile(0xaa0000))
            .child(div().w(10.).h(10.).bg(Color::hex(0x00aa00)))
            .child(tile(0x0000aa).child(div().w(30.).h(30.).bg(Color::hex(0x00aaaa)).child(tile(0xaaaa00))));
        let (laid, scene) = lay(root, &mut shaper);
        let colors: Vec<Color> = scene.quads().map(|q| q.background).collect();
        let hex = Color::hex;
        assert_eq!(colors, vec![hex(0x00aa00), hex(0xaa0000), hex(0x0000aa), hex(0x00aaaa), hex(0xaaaa00)]);
        let top = laid.click_target((5., 5.)).unwrap();
        assert_eq!(laid.bounds_of(top), Rect::new(0., 0., 50., 50.));
        assert_eq!(laid.bounds_of(top), laid.bounds_of(laid.click_target((5., 5.)).unwrap()));
    }

    fn list(rows: usize) -> Element<()> {
        let mut inner = div().id("list").h(100.).overflow_y_scroll();
        for _ in 0..rows {
            inner = inner.child(div().h(30.).on_click(|_, _| {}));
        }
        div().child(inner)
    }

    #[test]
    fn a_scroll_container_knows_its_range_clips_children_and_moves_them() {
        let mut shaper = Shaper::new();
        let mut store = ScrollStore::new();
        let (laid, _) = layout_and_paint(list(10), SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut store);
        assert_eq!(b(&laid, 1), (0., 0., 400., 100.));
        assert_eq!(laid.nodes[1].scroll_max, (0., 200.), "10 rows of 30 in a 100 tall viewport");
        assert_eq!(laid.nodes[2].clip, Some(Rect::new(0., 0., 400., 100.)), "children are cut off at the container");
        assert_eq!(b(&laid, 3).1, 30.);

        store.insert(ElementId::new("list"), (0., 50.));
        let (laid, _) = layout_and_paint(list(10), SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut store);
        assert_eq!(b(&laid, 2).1, -50., "scrolled by 50");
        assert_eq!(b(&laid, 3).1, -20.);

        store.insert(ElementId::new("list"), (0., 9999.));
        let (laid, _) = layout_and_paint(list(10), SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut store);
        assert_eq!(store[&ElementId::new("list")], (0., 200.), "an out-of-range position is clamped");
        assert_eq!(b(&laid, 11).1, 70., "the last row ends at the bottom of the viewport");
    }

    #[test]
    fn content_scrolled_out_of_view_cannot_be_clicked() {
        let mut shaper = Shaper::new();
        let (laid, _) = layout_and_paint(list(10), SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut ScrollStore::new());
        assert_eq!(laid.click_target((10., 50.)), Some(3), "y=50 falls in the second row (30..60)");
        assert_eq!(laid.click_target((10., 280.)), None, "the ninth row lies below the viewport, cut off");
    }

    #[test]
    fn wheel_goes_to_the_innermost_container_that_can_move_then_to_the_outer_one() {
        let mut shaper = Shaper::new();
        let mut store = ScrollStore::new();
        let build = || -> Element<()> {
            let mut inner = div().id("inner").h(50.).overflow_y_scroll();
            for _ in 0..4 {
                inner = inner.child(div().h(30.));
            }
            let mut outer = div().id("outer").h(150.).overflow_y_scroll().child(inner);
            for _ in 0..6 {
                outer = outer.child(div().h(40.));
            }
            div().child(outer)
        };
        let at = (10., 20.);
        let (laid, _) = layout_and_paint(build(), SIZE, at, Color::hex(0xffffff), &mut shaper, &mut store);
        laid.scroll_by(at, (0., -30.), &mut store);
        assert_eq!(store[&ElementId::new("inner")], (0., 30.));
        assert!(store.get(&ElementId::new("outer")).is_none_or(|o| o.1 == 0.), "the inner list took it");

        let (laid, _) = layout_and_paint(build(), SIZE, at, Color::hex(0xffffff), &mut shaper, &mut store);
        laid.scroll_by(at, (0., -100.), &mut store);
        assert_eq!(store[&ElementId::new("inner")], (0., 70.), "the inner list scrolls to its end");
        let (laid, _) = layout_and_paint(build(), SIZE, at, Color::hex(0xffffff), &mut shaper, &mut store);
        laid.scroll_by(at, (0., -30.), &mut store);
        assert_eq!(store[&ElementId::new("outer")], (0., 30.), "at its end, the inner list passes the wheel outward");
    }

    #[test]
    fn a_thumb_is_painted_on_top_only_when_asked_for_and_needed() {
        let mut shaper = Shaper::new();
        let thumb = Color::hex(0x123456);
        let build = |rows: usize, bar: bool| -> Element<()> {
            let mut inner = div().id("t").h(100.).overflow_y_scroll();
            if bar {
                inner = inner.scrollbar(thumb);
            }
            for _ in 0..rows {
                inner = inner.child(div().h(30.).bg(Color::hex(0xffffff)));
            }
            div().child(inner)
        };
        let mut count = |rows, bar| {
            let (_, scene) = layout_and_paint(build(rows, bar), SIZE, NOWHERE, Color::hex(0xffffff), &mut shaper, &mut ScrollStore::new());
            let last_is_thumb = scene.quads().last().is_some_and(|q| q.background == thumb);
            (scene.quads().filter(|q| q.background == thumb).count(), last_is_thumb)
        };
        assert_eq!(count(10, true), (1, true), "drawn after the rows it overlaps");
        assert_eq!(count(2, true), (0, false), "nothing to scroll: no thumb");
        assert_eq!(count(10, false), (0, false), "not asked for");
    }

    #[test]
    fn absolute_elements_leave_the_flow_and_sit_relative_to_their_parent() {
        let mut shaper = Shaper::new();
        let root = div()
            .p(10.)
            .child(div().w(200.).h(100.).child(div().w(50.).h(20.)).child(div().absolute().left(10.).top(20.).w(30.).h(40.))
                .child(div().absolute().right(5.).bottom(5.).w(30.).h(40.))
                .child(div().inset(0.)))
            .child(div().h(10.));
        let (laid, _) = lay(root, &mut shaper);
        assert_eq!(b(&laid, 1), (10., 10., 200., 100.));
        assert_eq!(b(&laid, 2), (10., 10., 50., 20.), "in-flow children ignore the absolute ones");
        assert_eq!(b(&laid, 3), (20., 30., 30., 40.), "left/top from the parent's corner");
        assert_eq!(b(&laid, 4), (175., 65., 30., 40.), "right/bottom from the parent's far corner");
        assert_eq!(b(&laid, 5), (10., 10., 200., 100.), "inset(0) fills the parent");
        assert_eq!(b(&laid, 6), (10., 110., 380., 10.), "the next sibling sits right below the parent");
    }

    #[test]
    fn an_image_surface_is_sized_by_its_buffer_placed_by_its_view_and_clipped_to_its_box() {
        use crate::{image::{ImageBuffer, ImageView}, element::image_surface};
        let buffer = ImageBuffer::new(40, 30);
        let view = ImageView::new().zoom(2.).pan(-10., 5.);
        let root: Element<()> = div().p(10.).child(image_surface(&buffer, view)).child(image_surface(&buffer, view).w(50.).h(20.).opacity(0.5));
        let (_, scene) = layout_and_paint(root, (200., 200.), (0., 0.), Color::hex(0xffffff), &mut Shaper::new(), &mut ScrollStore::new());
        let images: Vec<_> = scene.images().collect();
        assert_eq!(images.len(), 2);
        // First: 40x30 at (10, 10); the image's corner is at pan from there.
        assert_eq!((images[0].origin, images[0].zoom, images[0].clip), ((0., 15.), 2., Some(Rect::new(10., 10., 40., 30.))));
        // Second: resized by the style, below the first; the clip is its own box, not the image's.
        assert_eq!((images[1].origin, images[1].clip), ((0., 45.), Some(Rect::new(10., 40., 50., 20.))));
        assert!(images[1].opacity < 0.6 && images[0].opacity == 1.);
        assert_eq!(images[0].buffer.id(), buffer.id(), "the element shares the app's buffer");
    }
}
