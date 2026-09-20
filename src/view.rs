//! The high-level way to write an app: implement [`View`] for your state and call [`run_view`].
use crate::{
    element::{Cursor, Element},
    error::Result,
    event::{Event, MouseButton, ScrollDelta},
    layout::{self, Laid, ScrollStore},
    list::ScrollInfo,
    scene::{Color, Scene},
    shell::{run_with, App, Cx, WindowOptions},
};

/// How far one notch of a mouse wheel scrolls, in logical pixels.
const LINE_SCROLL: f32 = 40.;

/// An app whose whole UI is a function of its state.
pub trait View: Sized + 'static {
    /// Describes the UI for the current state. Called once per frame.
    fn view(&self, cx: &mut Cx) -> Element<Self>;

    /// The window's background color, behind everything.
    fn background(&self) -> Color {
        Color::hex(0xffffff)
    }
}

/// Opens a window titled `title` showing `state`'s [`View`] until it is closed.
pub fn run_view<S: View>(title: &str, state: S) -> Result<()> {
    run_view_with(WindowOptions::new(title), state)
}

/// Like [`run_view`], with control over the window's size.
pub fn run_view_with<S: View>(options: WindowOptions, state: S) -> Result<()> {
    run_with(options, Host::new(state))
}

pub(crate) struct Host<S: View> {
    pub state: S,
    /// The last frame's layout, which mouse events are resolved against.
    laid: Option<Laid<S>>,
    mouse: (f32, f32),
    scroll: ScrollStore,
    /// The element a click started on; the click only counts if it also ends there.
    pressed: Option<usize>,
}

impl<S: View> Host<S> {
    pub fn new(state: S) -> Self {
        Host { state, laid: None, mouse: (0., 0.), scroll: ScrollStore::new(), pressed: None }
    }
}

impl<S: View> App for Host<S> {
    fn event(&mut self, event: Event, cx: &mut Cx) {
        let Some(laid) = &self.laid else { return };
        match event {
            Event::MouseMoved { pos } => {
                self.mouse = pos;
                cx.set_cursor(laid.cursor_at(pos));
            }
            Event::MousePressed { button: MouseButton::Left, pos, .. } => self.pressed = laid.click_target(pos),
            Event::MouseReleased { button: MouseButton::Left, pos } => {
                let target = laid.click_target(pos);
                if target.is_some() && target == self.pressed {
                    if let Some(handler) = target.and_then(|i| laid.handler(i)) {
                        handler(&mut self.state, cx);
                    }
                }
                self.pressed = None;
            }
            Event::Scroll { delta, pos } => {
                let (dx, dy) = match delta {
                    ScrollDelta::Lines(x, y) => (x * LINE_SCROLL, y * LINE_SCROLL),
                    ScrollDelta::Pixels(x, y) => (x, y),
                };
                laid.scroll_by(pos, (dx, dy), &mut self.scroll);
            }
            _ => {}
        }
    }

    fn scene(&mut self, cx: &mut Cx) -> Scene {
        // Lists build only their visible rows, from where they are scrolled to *now* and the size they had last frame.
        if let Some(laid) = &self.laid {
            cx.scroll_info = laid
                .viewports()
                .map(|(id, viewport)| (id, ScrollInfo { offset: self.scroll.get(&id).copied().unwrap_or_default(), viewport }))
                .collect();
        }
        let root = self.state.view(cx);
        let (laid, scene) = layout::layout_and_paint(root, cx.size(), self.mouse, self.state.background(), cx.shaper, &mut self.scroll);
        self.laid = Some(laid);
        scene
    }
}

impl From<Cursor> for winit::window::CursorIcon {
    fn from(c: Cursor) -> Self {
        match c {
            Cursor::Default => winit::window::CursorIcon::Default,
            Cursor::Pointer => winit::window::CursorIcon::Pointer,
            Cursor::Text => winit::window::CursorIcon::Text,
        }
    }
}
