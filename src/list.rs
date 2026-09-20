//! Long lists that only build the rows on screen.
use std::{hash::Hash, ops::Range};

use crate::{
    element::{div, Element, ElementId},
    shell::Cx,
};

/// Extra rows built above and below the viewport so fast scrolling never shows a gap.
const OVERSCAN: usize = 2;

/// What a scroll container looked like in the latest frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScrollInfo {
    pub offset: (f32, f32),
    pub viewport: (f32, f32),
}

impl Cx<'_> {
    /// The rows of a `count`-row list with `row_height` rows that are on screen (plus a few spare).
    /// Before the list has been laid out once, the window's height stands in for its viewport.
    pub fn visible_rows(&self, id: impl Hash, count: usize, row_height: f32) -> Range<usize> {
        let info = self.scroll_info.get(&ElementId::new(id)).copied();
        let viewport_h = info.map_or(self.size().1, |i| i.viewport.1);
        let max_offset = (count as f32 * row_height - viewport_h).max(0.);
        let offset = info.map_or(0., |i| i.offset.1).clamp(0., max_offset);
        let first = (offset / row_height).floor() as usize;
        let last = ((offset + viewport_h) / row_height).ceil() as usize;
        first.saturating_sub(OVERSCAN)..(last + OVERSCAN).min(count)
    }
}

/// A vertically scrolling list of `count` rows, each `row_height` tall. `row(i)` is called only for
/// the rows on screen, so a list of a million items costs no more than one of a hundred.
///
/// The returned element is the scroll container: size it with the usual methods (`h`, `grow`, ...).
///
/// ```ignore
/// uniform_list(cx, "files", self.files.len(), 24., |i| text(&self.files[i]))
///     .grow()
/// ```
pub fn uniform_list<S>(
    cx: &Cx,
    id: impl Hash,
    count: usize,
    row_height: f32,
    mut row: impl FnMut(usize) -> Element<S>,
) -> Element<S> {
    let id = ElementId::new(id);
    let range = cx.visible_rows(id, count, row_height);
    let rows = range.clone().map(|i| row(i).h(row_height));
    div()
        .id(id)
        .overflow_y_scroll()
        .child(div().h(count as f32 * row_height).pt(range.start as f32 * row_height).children(rows))
}
