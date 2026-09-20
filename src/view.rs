//! The high-level way to write an app: implement [`View`] for your state and call [`run_view`].
use crate::{
    element::{Cursor, Element, MouseEvent},
    error::Result,
    event::{Event, MouseButton, ScrollDelta},
    layout::{self, Capture, Laid, ScrollStore},
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
    /// The element a mouse press is being dragged from.
    captured: Option<(Capture, MouseButton, u32)>,
}

impl<S: View> Host<S> {
    pub fn new(state: S) -> Self {
        Host { state, laid: None, mouse: (0., 0.), scroll: ScrollStore::new(), pressed: None, captured: None }
    }
}

impl<S: View> App for Host<S> {
    fn event(&mut self, event: Event, cx: &mut Cx) {
        let Some(laid) = &self.laid else { return };
        let modifiers = cx.modifiers;
        let mouse_event = |i: usize, pos: (f32, f32), button: MouseButton, click_count: u32| {
            let b = laid.bounds_of(i);
            MouseEvent { pos, local: (pos.0 - b.x, pos.1 - b.y), button, click_count, modifiers }
        };
        match event {
            Event::MouseMoved { pos } => {
                self.mouse = pos;
                cx.set_cursor(laid.cursor_at(pos));
                if let Some((capture, button, click_count)) = self.captured {
                    if let Some(i) = laid.resolve(capture) {
                        if let Some(handler) = laid.on_drag(i) {
                            handler(&mut self.state, cx, mouse_event(i, pos, button, click_count));
                        }
                    }
                }
            }
            Event::MousePressed { button, pos, click_count } => {
                if button == MouseButton::Left {
                    self.pressed = laid.click_target(pos);
                }
                if let Some(i) = laid.press_target(pos) {
                    if let Some(handler) = laid.on_mouse_down(i) {
                        handler(&mut self.state, cx, mouse_event(i, pos, button, click_count));
                    }
                    if laid.captures(i) {
                        self.captured = Some((laid.key_of(i), button, click_count));
                    }
                }
            }
            Event::MouseReleased { button, pos } => {
                if let Some((capture, _, click_count)) = self.captured.filter(|(_, b, _)| *b == button) {
                    self.captured = None;
                    if let Some(i) = laid.resolve(capture) {
                        if let Some(handler) = laid.on_mouse_up(i) {
                            handler(&mut self.state, cx, mouse_event(i, pos, button, click_count));
                        }
                    }
                }
                if button == MouseButton::Left {
                    let target = laid.click_target(pos);
                    if target.is_some() && target == self.pressed {
                        if let Some(handler) = target.and_then(|i| laid.handler(i)) {
                            handler(&mut self.state, cx);
                        }
                    }
                    self.pressed = None;
                }
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
