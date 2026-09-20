//! Opens a window and drives an [`App`] with winit's event loop.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, Ime as WinitIme, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key as WinitKey, NamedKey as WinitNamedKey},
    window::{Window, WindowId},
};

use crate::{
    error::{err, Result},
    event::{Event, Ime, Key, Modifiers, MouseButton, NamedKey, ScrollDelta},
    renderer::Renderer,
    scene::{Rect, Scene},
    shaper::Shaper,
};

/// What the window offers an app. The shell implements it on a real window; tests use a recorder.
pub trait Platform {
    fn set_ime_allowed(&self, allowed: bool);
    fn set_ime_cursor_area(&self, area: Rect);
    fn set_cursor(&self, cursor: crate::element::Cursor);
}

struct WindowPlatform<'a>(&'a Window);

impl Platform for WindowPlatform<'_> {
    fn set_ime_allowed(&self, allowed: bool) {
        self.0.set_ime_allowed(allowed);
    }

    fn set_ime_cursor_area(&self, area: Rect) {
        self.0.set_ime_cursor_area(LogicalPosition::new(area.x, area.y), LogicalSize::new(area.w, area.h));
    }

    fn set_cursor(&self, cursor: crate::element::Cursor) {
        self.0.set_cursor(winit::window::CursorIcon::from(cursor));
    }
}

/// What an [`App`] can see and ask for while it handles an event or paints a frame.
pub struct Cx<'a> {
    /// Measures text with the same fonts the renderer draws with.
    pub shaper: &'a mut Shaper,
    /// The modifier keys held right now.
    pub modifiers: Modifiers,
    size: (f32, f32),
    scale_factor: f32,
    platform: &'a dyn Platform,
    wake_at: &'a mut Option<Instant>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(
        shaper: &'a mut Shaper,
        modifiers: Modifiers,
        size: (f32, f32),
        scale_factor: f32,
        platform: &'a dyn Platform,
        wake_at: &'a mut Option<Instant>,
    ) -> Self {
        Cx { shaper, modifiers, size, scale_factor, platform, wake_at }
    }

    /// The drawable area in logical pixels.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Asks for another frame after `after` (for blinking carets, animations).
    pub fn request_redraw_after(&mut self, after: Duration) {
        let at = Instant::now() + after;
        *self.wake_at = Some(self.wake_at.map_or(at, |current| current.min(at)));
    }

    /// Turns input-method (IME) support on or off. Text fields want it on.
    pub fn set_ime_allowed(&self, allowed: bool) {
        self.platform.set_ime_allowed(allowed);
    }

    /// Sets the mouse cursor shown over the window.
    pub fn set_cursor(&self, cursor: crate::element::Cursor) {
        self.platform.set_cursor(cursor);
    }

    /// Tells the input method where the caret is, so its candidate window appears next to it.
    pub fn set_ime_cursor_area(&self, area: Rect) {
        self.platform.set_ime_cursor_area(area);
    }
}

/// An application: reacts to input and describes each frame as a [`Scene`].
pub trait App {
    /// Called for every input event. A new frame is drawn afterwards.
    fn event(&mut self, _event: Event, _cx: &mut Cx) {}

    /// Called when a frame is needed.
    fn scene(&mut self, cx: &mut Cx) -> Scene;
}

/// Opens a window titled `title` and runs `app` until the window is closed.
pub fn run(title: &str, app: impl App) -> Result<()> {
    let event_loop = EventLoop::new().map_err(err("create event loop"))?;
    let mut shell = Shell {
        title: title.to_string(),
        app,
        window: None,
        renderer: None,
        shaper: Shaper::new(),
        modifiers: Modifiers::default(),
        cursor: (0., 0.),
        last_click: None,
        wake_at: None,
        failure: None,
    };
    event_loop.run_app(&mut shell).map_err(err("run event loop"))?;
    match shell.failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Clicks closer together than this in time and space count as a multi-click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f32 = 5.;

struct Shell<A: App> {
    title: String,
    app: A,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    shaper: Shaper,
    modifiers: Modifiers,
    cursor: (f32, f32),
    last_click: Option<(Instant, (f32, f32), MouseButton, u32)>,
    wake_at: Option<Instant>,
    failure: Option<crate::Error>,
}

impl<A: App> Shell<A> {
    /// Hands `event` to the app, then draws a new frame.
    fn dispatch(&mut self, event: Event) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_ref()) else { return };
        let platform = WindowPlatform(window);
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, renderer.logical_size(), window.scale_factor() as f32, &platform, &mut self.wake_at);
        self.app.event(event, &mut cx);
        window.request_redraw();
    }

    fn click_count(&mut self, button: MouseButton) -> u32 {
        let now = Instant::now();
        let count = match self.last_click {
            Some((at, pos, b, count))
                if b == button
                    && now.duration_since(at) < MULTI_CLICK_TIME
                    && (pos.0 - self.cursor.0).abs() < MULTI_CLICK_DISTANCE
                    && (pos.1 - self.cursor.1).abs() < MULTI_CLICK_DISTANCE =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, self.cursor, button, count));
        count
    }
}

impl<A: App> ApplicationHandler for Shell<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title(self.title.as_str()).with_inner_size(LogicalSize::new(900., 600.));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                self.failure = Some(crate::Error(format!("create window: {e}")));
                event_loop.exit();
                return;
            }
        };
        match Renderer::new(window.clone()) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(e) => {
                self.failure = Some(e);
                event_loop.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(window), Some(renderer)) = (self.window.clone(), self.renderer.as_mut()) else { return };
        let scale = window.scale_factor() as f32;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                renderer.resize(size, scale);
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                renderer.resize(window.inner_size(), scale);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.shaper.begin_frame();
                let platform = WindowPlatform(&window);
                let mut cx = Cx::new(&mut self.shaper, self.modifiers, renderer.logical_size(), scale, &platform, &mut self.wake_at);
                let scene = self.app.scene(&mut cx);
                renderer.render(&scene, &mut self.shaper);
                self.shaper.end_frame();
            }
            WindowEvent::ModifiersChanged(m) => {
                let state = m.state();
                self.modifiers = Modifiers {
                    shift: state.shift_key(),
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    logo: state.super_key(),
                };
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                self.dispatch(Event::MouseMoved { pos: self.cursor });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = mouse_button(button);
                let pos = self.cursor;
                match state {
                    ElementState::Pressed => {
                        let click_count = self.click_count(button);
                        self.dispatch(Event::MousePressed { button, pos, click_count });
                    }
                    ElementState::Released => self.dispatch(Event::MouseReleased { button, pos }),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(x, y),
                    MouseScrollDelta::PixelDelta(p) => ScrollDelta::Pixels(p.x as f32 / scale, p.y as f32 / scale),
                };
                let pos = self.cursor;
                self.dispatch(Event::Scroll { delta, pos });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let key = key(&event.logical_key);
                match event.state {
                    ElementState::Pressed => {
                        let text = event.text.as_deref().filter(|t| !t.is_empty() && !t.chars().any(char::is_control)).map(str::to_string);
                        self.dispatch(Event::KeyDown { key, text, repeat: event.repeat });
                    }
                    ElementState::Released => self.dispatch(Event::KeyUp { key }),
                }
            }
            WindowEvent::Ime(ime) => match ime {
                WinitIme::Preedit(text, cursor) => self.dispatch(Event::Ime(Ime::Preedit { text, cursor })),
                WinitIme::Commit(text) => self.dispatch(Event::Ime(Ime::Commit(text))),
                WinitIme::Enabled | WinitIme::Disabled => {}
            },
            WindowEvent::Focused(focused) => self.dispatch(Event::FocusChanged(focused)),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.wake_at {
            Some(at) if Instant::now() >= at => {
                self.wake_at = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

fn mouse_button(button: winit::event::MouseButton) -> MouseButton {
    use winit::event::MouseButton as W;
    match button {
        W::Left => MouseButton::Left,
        W::Right => MouseButton::Right,
        W::Middle => MouseButton::Middle,
        W::Back => MouseButton::Other(3),
        W::Forward => MouseButton::Other(4),
        W::Other(n) => MouseButton::Other(n),
    }
}

fn key(key: &WinitKey) -> Key {
    match key {
        WinitKey::Character(s) => Key::Char(s.to_string()),
        WinitKey::Named(named) => match named {
            WinitNamedKey::Space => Key::Char(" ".into()),
            WinitNamedKey::Enter => Key::Named(NamedKey::Enter),
            WinitNamedKey::Escape => Key::Named(NamedKey::Escape),
            WinitNamedKey::Backspace => Key::Named(NamedKey::Backspace),
            WinitNamedKey::Delete => Key::Named(NamedKey::Delete),
            WinitNamedKey::Tab => Key::Named(NamedKey::Tab),
            WinitNamedKey::ArrowLeft => Key::Named(NamedKey::ArrowLeft),
            WinitNamedKey::ArrowRight => Key::Named(NamedKey::ArrowRight),
            WinitNamedKey::ArrowUp => Key::Named(NamedKey::ArrowUp),
            WinitNamedKey::ArrowDown => Key::Named(NamedKey::ArrowDown),
            WinitNamedKey::Home => Key::Named(NamedKey::Home),
            WinitNamedKey::End => Key::Named(NamedKey::End),
            WinitNamedKey::PageUp => Key::Named(NamedKey::PageUp),
            WinitNamedKey::PageDown => Key::Named(NamedKey::PageDown),
            _ => Key::Other,
        },
        _ => Key::Other,
    }
}
