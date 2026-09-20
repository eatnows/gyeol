//! Turns an [`Element`] tree into positions (via taffy's flexbox) and a [`Scene`].
use taffy::{prelude::*, tree::LayoutOutput, TaffyTree};

use crate::{
    element::{Align, Cursor, Direction, Element, Handler, Justify, Kind, Length, Style},
    scene::{Color, Quad, Rect, Scene, Text},
    shaper::Shaper,
};

const DEFAULT_TEXT_COLOR: Color = Color::hex(0x000000);
const DEFAULT_TEXT_SIZE: f32 = 14.;

struct TextLeaf {
    content: String,
    size: f32,
}

/// One laid-out element, in paint order (parents before their children).
pub(crate) struct Node<S> {
    pub bounds: Rect,
    parent: Option<usize>,
    background: Option<Color>,
    hover_background: Option<Color>,
    border_width: f32,
    border_color: Color,
    radius: f32,
    text: Option<(String, f32, Color)>,
    cursor: Cursor,
    on_click: Option<Handler<S>>,
    taffy_id: NodeId,
}

/// The result of laying out one frame: kept until the next frame so mouse input can be resolved
/// against what the user actually sees.
pub(crate) struct Laid<S> {
    nodes: Vec<Node<S>>,
}

impl<S> Laid<S> {
    /// The frontmost element under `pos`.
    fn topmost_at(&self, pos: (f32, f32)) -> Option<usize> {
        self.nodes.iter().rposition(|n| n.bounds.contains(pos))
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

    /// The cursor to show at `pos`: that of the innermost element that asks for one.
    pub fn cursor_at(&self, pos: (f32, f32)) -> Cursor {
        self.chain(self.topmost_at(pos)).map(|i| self.nodes[i].cursor).find(|c| *c != Cursor::Default).unwrap_or_default()
    }
}

/// Lays `root` out in a `size` window and paints it, marking elements under `mouse` as hovered.
pub(crate) fn layout_and_paint<S>(
    root: Element<S>,
    size: (f32, f32),
    mouse: (f32, f32),
    background: Color,
    shaper: &mut Shaper,
) -> (Laid<S>, Scene) {
    let mut tree: TaffyTree<TextLeaf> = TaffyTree::new();
    let mut nodes: Vec<Node<S>> = Vec::new();

    let mut root = root;
    root.style.width = Length::Px(size.0);
    root.style.height = Length::Px(size.1);
    let root_id = build(root, None, (DEFAULT_TEXT_COLOR, DEFAULT_TEXT_SIZE), &mut tree, &mut nodes);

    let available = taffy::Size { width: AvailableSpace::Definite(size.0), height: AvailableSpace::Definite(size.1) };
    let _ = tree.compute_layout_with_measure(root_id, available, |input, _, leaf, _| {
        let (w, h) = match leaf {
            Some(leaf) => (shaper.width(&leaf.content, leaf.size), Shaper::line_height(leaf.size)),
            None => (0., 0.),
        };
        LayoutOutput::from_outer_size(taffy::Size {
            width: input.known_dimensions.width.unwrap_or(w),
            height: input.known_dimensions.height.unwrap_or(h),
        })
    });

    // Absolute positions: a node's layout is relative to its parent.
    for i in 0..nodes.len() {
        let layout = tree.layout(nodes[i].taffy_id).expect("node exists");
        let (px, py) = nodes[i].parent.map_or((0., 0.), |p| (nodes[p].bounds.x, nodes[p].bounds.y));
        nodes[i].bounds = Rect::new(px + layout.location.x, py + layout.location.y, layout.size.width, layout.size.height);
    }

    let laid = Laid { nodes };
    let hovered: Vec<usize> = laid.chain(laid.topmost_at(mouse)).collect();
    let mut scene = Scene { background: Some(background), ..Default::default() };
    for (i, node) in laid.nodes.iter().enumerate() {
        let fill = if hovered.contains(&i) { node.hover_background.or(node.background) } else { node.background };
        if fill.is_some() || node.border_width > 0. {
            let mut quad = Quad::new(node.bounds, fill.unwrap_or(Color::TRANSPARENT)).rounded(node.radius);
            if node.border_width > 0. {
                quad = quad.bordered(node.border_width, node.border_color);
            }
            scene.quads.push(quad);
        }
        if let Some((content, size, color)) = &node.text {
            scene.texts.push(Text::new((node.bounds.x, node.bounds.y), content.clone(), *size, *color));
        }
    }
    (laid, scene)
}

fn build<S>(
    el: Element<S>,
    parent: Option<usize>,
    inherited: (Color, f32),
    tree: &mut TaffyTree<TextLeaf>,
    nodes: &mut Vec<Node<S>>,
) -> NodeId {
    let Element { style, kind, on_click } = el;
    let text_color = style.text_color.unwrap_or(inherited.0);
    let text_size = style.text_size.unwrap_or(inherited.1);
    let index = nodes.len();
    let taffy_style = to_taffy(&style);

    // The arena slot is reserved first so children can point at their parent; the taffy id is filled in below.
    nodes.push(Node {
        bounds: Rect::default(),
        parent,
        background: style.background,
        hover_background: style.hover_background,
        border_width: style.border_width,
        border_color: style.border_color,
        radius: style.radius,
        text: None,
        cursor: style.cursor,
        on_click,
        taffy_id: NodeId::new(0),
    });

    let id = match kind {
        Kind::Text(content) => {
            nodes[index].text = Some((content.clone(), text_size, text_color));
            tree.new_leaf_with_context(taffy_style, TextLeaf { content, size: text_size }).expect("leaf")
        }
        Kind::Div(children) => {
            let ids: Vec<NodeId> = children.into_iter().map(|c| build(c, Some(index), (text_color, text_size), tree, nodes)).collect();
            tree.new_with_children(taffy_style, &ids).expect("container")
        }
    };
    nodes[index].taffy_id = id;
    id
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

    let mut t = taffy::Style::default();
    t.display = Display::Flex;
    t.flex_direction = match s.direction {
        Direction::Row => FlexDirection::Row,
        Direction::Column => FlexDirection::Column,
    };
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
        layout_and_paint(root, SIZE, NOWHERE, Color::hex(0xffffff), shaper)
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
        assert_eq!(scene.quads.len(), 1, "only the bordered element paints a quad");
        assert_eq!(scene.quads[0].border_width, 2.);
    }

    #[test]
    fn text_is_as_big_as_the_shaper_says() {
        let mut shaper = Shaper::new();
        let root = div().row().items_center().text_size(20.).child(text("Hello 결"));
        let (laid, scene) = lay(root, &mut shaper);
        let (_, _, w, h) = b(&laid, 1);
        assert!((w - shaper.width("Hello 결", 20.)).abs() <= 1., "layout rounds to whole pixels: {w}");
        assert_eq!(h, Shaper::line_height(20.));
        assert_eq!(scene.texts.len(), 1);
        assert_eq!(scene.texts[0].size, 20., "size is inherited from the container");
    }

    #[test]
    fn text_color_is_inherited_and_can_be_overridden() {
        let mut shaper = Shaper::new();
        let red = Color::hex(0xff0000);
        let blue = Color::hex(0x0000ff);
        let root = div().text_color(red).child(text("a")).child(text("b").text_color(blue));
        let (_, scene) = lay(root, &mut shaper);
        assert_eq!(scene.texts[0].color, red);
        assert_eq!(scene.texts[1].color, blue);
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

        let (laid, scene) = layout_and_paint(build(), SIZE, (50., 25.), Color::hex(0xffffff), &mut shaper);
        assert_eq!(scene.quads.len(), 2);
        assert_eq!(scene.quads[0].background, hot, "the pointer is over the first box");
        assert_eq!(scene.quads[1].background, idle);
        assert_eq!(laid.click_target((5., 5.)), Some(1), "a click on the inner child bubbles up to the handler");
        assert_eq!(laid.click_target((150., 25.)), None, "the second box has no handler");
        assert_eq!(laid.cursor_at((50., 25.)), Cursor::Pointer);
        assert_eq!(laid.cursor_at((150., 25.)), Cursor::Default);

        let (_, away) = layout_and_paint(build(), SIZE, (350., 200.), Color::hex(0xffffff), &mut shaper);
        assert_eq!(away.quads[0].background, idle, "no hover when the pointer is elsewhere");
    }
}
