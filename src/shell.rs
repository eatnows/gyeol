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
    event::{Event, Ime, Key, Modifiers, MouseButton, NamedKey, ScrollDelta, SystemTheme},
    renderer::{Acquired, Renderer},
    scene::{Rect, Scene},
    shaper::Shaper,
};

/// What the window offers an app. The shell implements it on a real window; tests use a recorder.
pub trait Platform {
    fn set_ime_allowed(&self, allowed: bool);
    fn set_ime_cursor_area(&self, area: Rect);
    fn set_cursor(&self, cursor: crate::element::Cursor);
    /// The system's current light/dark preference, if it has one.
    fn theme(&self) -> Option<SystemTheme>;
    fn clipboard_text(&self) -> Option<String>;
    fn set_clipboard_text(&self, text: &str);
    /// Whether the user asked the system to reduce transparency (macOS: Accessibility > Display).
    fn reduce_transparency(&self) -> bool {
        false
    }
    /// Shows a modal "open" dialog; `None` when cancelled or not implemented on this platform.
    fn pick_file(&self, _filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        None
    }
    /// Shows a modal "save" dialog; `None` when cancelled or not implemented on this platform.
    fn pick_save_path(&self, _default_name: &str, _filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        None
    }
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

    fn theme(&self) -> Option<SystemTheme> {
        self.0.theme().map(system_theme)
    }

    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn set_clipboard_text(&self, text: &str) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text.to_string());
        }
    }

    fn reduce_transparency(&self) -> bool {
        #[cfg(target_os = "macos")]
        return crate::macos::display::reduce_transparency();
        #[cfg(not(target_os = "macos"))]
        false
    }

    fn pick_file(&self, filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "macos")]
        return crate::macos::dialog::pick_file(filters);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = filters;
            None
        }
    }

    fn pick_save_path(&self, default_name: &str, filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "macos")]
        return crate::macos::dialog::pick_save_path(default_name, filters);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (default_name, filters);
            None
        }
    }
}

fn system_theme(theme: winit::window::Theme) -> SystemTheme {
    match theme {
        winit::window::Theme::Light => SystemTheme::Light,
        winit::window::Theme::Dark => SystemTheme::Dark,
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
    /// The time of this frame or event; [`Cx::request_redraw_after`] counts from it.
    now: Instant,
    /// Scroll positions and viewport sizes of the previous frame, for building only visible rows.
    pub(crate) scroll_info: std::collections::HashMap<crate::element::ElementId, crate::list::ScrollInfo>,
    /// Regions asked to be scrolled into view; applied once the event has been handled.
    pub(crate) reveals: Vec<(crate::element::ElementId, Rect)>,
    /// Pointer readings since the previous pointer event, oldest first.
    pub(crate) pointer_samples: Vec<crate::pointer::PointerSample>,
    /// The reading that belongs to the event being handled, when the platform gave one.
    pub(crate) pointer: Option<crate::pointer::PointerSample>,
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
        Cx { shaper, modifiers, size, scale_factor, platform, wake_at, now: Instant::now(), scroll_info: Default::default(), reveals: Vec::new(), pointer_samples: Vec::new(), pointer: None }
    }

    /// The drawable area in logical pixels.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// The time this frame or event is being handled at. [`View::tick`](crate::View::tick) gets the same
    /// value; headless hosts ([`TestHost`](crate::testing::TestHost)) run on their own clock.
    pub fn now(&self) -> Instant {
        self.now
    }

    pub(crate) fn with_now(mut self, now: Instant) -> Self {
        self.now = now;
        self
    }

    /// Asks for another frame after `after` (for blinking carets, animations). Each frame calls
    /// [`View::tick`](crate::View::tick) first, so an animation advances its state there and asks
    /// again; the earliest request wins when several are made.
    pub fn request_redraw_after(&mut self, after: Duration) {
        let at = self.now + after;
        *self.wake_at = Some(self.wake_at.map_or(at, |current| current.min(at)));
    }

    /// Turns input-method (IME) support on or off. Text fields want it on.
    pub fn set_ime_allowed(&self, allowed: bool) {
        self.platform.set_ime_allowed(allowed);
    }

    /// The text on the system clipboard, if any.
    pub fn clipboard_text(&self) -> Option<String> {
        self.platform.clipboard_text()
    }

    pub fn set_clipboard_text(&self, text: &str) {
        self.platform.set_clipboard_text(text);
    }

    /// How far the scroll container `id` was scrolled in the latest frame.
    pub fn scroll_offset(&self, id: impl std::hash::Hash) -> (f32, f32) {
        self.scroll_info.get(&crate::element::ElementId::new(id)).map_or((0., 0.), |i| i.offset)
    }

    /// The visible size of the scroll container `id` in the latest frame.
    pub fn viewport(&self, id: impl std::hash::Hash) -> Option<(f32, f32)> {
        self.scroll_info.get(&crate::element::ElementId::new(id)).map(|i| i.viewport)
    }

    /// Where the scroll container `id` was in the window in the latest frame (position and size).
    pub fn bounds(&self, id: impl std::hash::Hash) -> Option<Rect> {
        self.scroll_info.get(&crate::element::ElementId::new(id)).map(|i| Rect::new(i.origin.0, i.origin.1, i.viewport.0, i.viewport.1))
    }

    /// Scrolls the container `id` just enough that `region` (in the container's unscrolled content
    /// coordinates) is visible. Takes effect for the next frame.
    pub fn scroll_to_reveal(&mut self, id: impl std::hash::Hash, region: Rect) {
        self.reveals.push((crate::element::ElementId::new(id), region));
    }

    /// Every pointer reading since the previous pointer event, oldest first; the last one is where
    /// the pointer is now. A frame can contain many pen samples, so draw strokes through all of them
    /// rather than only the event's position. Empty when the platform reports none (see
    /// [`crate::pointer`]); then use the event's own `pos`.
    pub fn pointer_samples(&self) -> &[crate::pointer::PointerSample] {
        &self.pointer_samples
    }

    pub(crate) fn with_pointer(mut self, samples: Vec<crate::pointer::PointerSample>, current: Option<crate::pointer::PointerSample>) -> Self {
        self.pointer_samples = samples;
        self.pointer = current;
        self
    }

    /// Kind, pressure and tilt for a mouse event; `pressed` is whether a button is down, for a pointer
    /// the platform said nothing about.
    pub(crate) fn pointer_state(&self, pressed: bool) -> (crate::pointer::PointerKind, f32, Option<(f32, f32)>) {
        match self.pointer {
            Some(s) => (s.kind, s.pressure, s.tilt),
            None => (crate::pointer::PointerKind::Mouse, if pressed { 1. } else { 0. }, None),
        }
    }

    /// Shows the system's modal "open file" dialog and returns the chosen path, or `None` if the user
    /// cancelled. `filters` are `(description, extensions)` pairs such as `("Images", &["png", "jpg"])`;
    /// empty allows any file. The dialog blocks the app until it closes. Implemented on macOS
    /// (NSOpenPanel; the panel takes one combined extension list, so descriptions are not shown); on
    /// other platforms this is **not implemented** and always returns `None`.
    pub fn pick_file(&self, filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        self.platform.pick_file(filters)
    }

    /// Shows the system's modal "save" dialog pre-filled with `default_name` and returns the chosen
    /// path (the file is not created), or `None` if cancelled. `filters` as for [`Cx::pick_file`].
    /// macOS only (NSSavePanel); **not implemented** elsewhere, where it returns `None`.
    pub fn pick_save_path(&self, default_name: &str, filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
        self.platform.pick_save_path(default_name, filters)
    }

    /// Whether the user turned on the system's "Reduce transparency" accessibility option, so translucent
    /// glass should give way to solid colors. macOS only; false elsewhere. [`Event::DisplayOptionsChanged`]
    /// is sent when it changes.
    pub fn reduce_transparency(&self) -> bool {
        self.platform.reduce_transparency()
    }

    /// Whether the system prefers dark or light (light when it does not say).
    pub fn system_theme(&self) -> SystemTheme {
        self.platform.theme().unwrap_or(SystemTheme::Light)
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

/// How a window looks regardless of the system's light or dark setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the system (the default).
    #[default]
    System,
    Dark,
    Light,
}

/// The blur-and-tint recipe of the window's vibrancy (macOS `NSVisualEffectMaterial`); see
/// [`WindowOptions::vibrancy_material`]. Materials other than the first two adapt to the window's
/// [`Appearance`]: with `Dark` they give a dark translucent glass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Material {
    /// The default: the light, nearly opaque backdrop used behind a window's content.
    #[default]
    UnderWindowBackground,
    /// A dark translucent heads-up-display panel, dark whatever the appearance.
    HudWindow,
    Sidebar,
    Popover,
    Menu,
    FullScreenUI,
    UnderPageBackground,
    Titlebar,
}

/// How the window is opened.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    /// Initial size in logical pixels.
    pub size: (f32, f32),
    /// The smallest size the user can resize to.
    pub min_size: Option<(f32, f32)>,
    /// A window whose unpainted (clear) parts show what is behind it; pair with a `Scene` background of
    /// `None` or a translucent color.
    pub transparent: bool,
    /// macOS: hide the title and make the titlebar transparent, letting the content fill the whole window
    /// (the traffic-light buttons float over it). Ignored elsewhere.
    pub transparent_titlebar: bool,
    /// macOS: where to put the traffic-light buttons, as (x from the left, y from the top) in logical
    /// pixels. Ignored elsewhere.
    pub traffic_lights: Option<(f32, f32)>,
    /// macOS: blur what is behind the window (NSVisualEffectView); needs `transparent`. Ignored elsewhere.
    pub vibrancy: bool,
    /// macOS: the vibrancy recipe; see [`Material`]. Needs `vibrancy`. Ignored elsewhere.
    pub vibrancy_material: Material,
    /// macOS: force a light or dark window, whatever the system setting. This decides how the vibrancy
    /// material is tinted and what [`Cx::system_theme`] reports. Ignored elsewhere.
    pub appearance: Appearance,
    /// macOS: deliver every mouse and pen movement instead of letting AppKit merge them to one per
    /// frame, so [`Cx::pointer_samples`] holds the device's full-rate samples. Costs more events.
    /// Applies to the whole app. Ignored elsewhere (pen input is not implemented there).
    pub raw_pointer_samples: bool,
}

impl WindowOptions {
    pub fn new(title: impl Into<String>) -> Self {
        WindowOptions { title: title.into(), size: (900., 600.), min_size: None, transparent: false, transparent_titlebar: false, traffic_lights: None, vibrancy: false, vibrancy_material: Material::UnderWindowBackground, appearance: Appearance::System, raw_pointer_samples: false }
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.min_size = Some((width, height));
        self
    }

    pub fn transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    pub fn transparent_titlebar(mut self, on: bool) -> Self {
        self.transparent_titlebar = on;
        self
    }

    pub fn traffic_lights(mut self, x: f32, y: f32) -> Self {
        self.traffic_lights = Some((x, y));
        self
    }

    pub fn vibrancy(mut self, on: bool) -> Self {
        self.vibrancy = on;
        self
    }

    pub fn vibrancy_material(mut self, material: Material) -> Self {
        self.vibrancy_material = material;
        self
    }

    pub fn appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn raw_pointer_samples(mut self, on: bool) -> Self {
        self.raw_pointer_samples = on;
        self
    }
}

/// Opens a window titled `title` and runs `app` until the window is closed.
pub fn run(title: &str, app: impl App) -> Result<()> {
    run_with(WindowOptions::new(title), app)
}

/// Opens a window as described by `options` and runs `app` until the window is closed.
pub fn run_with(options: WindowOptions, app: impl App) -> Result<()> {
    let event_loop = EventLoop::new().map_err(err("create event loop"))?;
    let proxy = event_loop.create_proxy();
    let mut shell = Shell {
        options,
        app,
        window: None,
        renderer: None,
        shaper: Shaper::new(),
        modifiers: Modifiers::default(),
        cursor: (0., 0.),
        last_click: None,
        wake_at: None,
        failure: None,
        proxy,
        #[cfg(target_os = "macos")]
        display_observer: None,
        trace: std::env::var_os("GYEOL_TRACE_REDRAW").map(|_| RedrawTrace { since: Instant::now(), asked: [0; 5], drawn: 0, skipped: 0 }),
        pen: None,
        dropped: Vec::new(),
        files_hovered: false,
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
/// How long to wait before trying again when no frame could be acquired (surface reconfigured or timed out).
const RETRY_AFTER: Duration = Duration::from_millis(50);

/// Why a redraw was asked for, for [`RedrawTrace`].
#[derive(Clone, Copy)]
enum Why {
    Event,
    Resize,
    Timer,
    Retry,
    Start,
}

/// `GYEOL_TRACE_REDRAW=1` prints once a second how many frames were drawn or skipped and who asked
/// for them. Frames nobody here asked for ("unasked") come from the window system itself.
struct RedrawTrace {
    since: Instant,
    asked: [u32; 5],
    drawn: u32,
    skipped: u32,
}

impl RedrawTrace {
    fn report(&mut self) {
        if self.since.elapsed() < Duration::from_secs(1) {
            return;
        }
        let asked: u32 = self.asked.iter().sum();
        let a = self.asked;
        eprintln!(
            "[gyeol] frames/s: drawn {} skipped {} | asked: event {} resize {} timer {} retry {} start {} | unasked {}",
            self.drawn,
            self.skipped,
            a[0], a[1], a[2], a[3], a[4],
            (self.drawn + self.skipped).saturating_sub(asked)
        );
        *self = RedrawTrace { since: Instant::now(), asked: [0; 5], drawn: 0, skipped: 0 };
    }
}

/// Asks the window for a redraw.
fn ask(trace: &mut Option<RedrawTrace>, window: &Window, why: Why) {
    if let Some(t) = trace {
        t.asked[why as usize] += 1;
    }
    window.request_redraw();
}

struct Shell<A: App> {
    options: WindowOptions,
    app: A,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    shaper: Shaper,
    modifiers: Modifiers,
    cursor: (f32, f32),
    last_click: Option<(Instant, (f32, f32), MouseButton, u32)>,
    wake_at: Option<Instant>,
    failure: Option<crate::Error>,
    /// Wakes the event loop from outside it (a system notification) so it can deliver an event.
    proxy: winit::event_loop::EventLoopProxy<()>,
    /// Keeps the Reduce Transparency subscription alive for as long as the shell runs.
    #[cfg(target_os = "macos")]
    display_observer: Option<crate::macos::display::DisplayOptionsObserver>,
    trace: Option<RedrawTrace>,
    pen: Option<crate::pointer::PenInput>,
    /// Files of the drop in progress; winit reports them one by one, delivered together in `about_to_wait`.
    dropped: Vec<std::path::PathBuf>,
    files_hovered: bool,
}

impl<A: App> Shell<A> {
    /// Hands `event` to the app, then draws a new frame.
    fn dispatch(&mut self, event: Event) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_ref()) else { return };
        let platform = WindowPlatform(window);
        // Pen data the platform saw for this pointer event.
        let (samples, current) = match (&event, self.pen.as_mut()) {
            (Event::MouseMoved { pos } | Event::MousePressed { pos, .. } | Event::MouseReleased { pos, .. }, Some(pen)) => pen.take(*pos),
            _ => (Vec::new(), None),
        };
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, renderer.logical_size(), window.scale_factor() as f32, &platform, &mut self.wake_at).with_pointer(samples, current);
        self.app.event(event, &mut cx);
        ask(&mut self.trace, window, Why::Event);
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
        let mut attributes = Window::default_attributes()
            .with_title(self.options.title.as_str())
            .with_transparent(self.options.transparent)
            .with_inner_size(LogicalSize::new(self.options.size.0, self.options.size.1));
        if let Some((w, h)) = self.options.min_size {
            attributes = attributes.with_min_inner_size(LogicalSize::new(w, h));
        }
        #[cfg(target_os = "macos")]
        if self.options.transparent_titlebar {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attributes = attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true);
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                self.failure = Some(crate::Error(format!("create window: {e}")));
                event_loop.exit();
                return;
            }
        };
        match Renderer::new(window.clone(), self.options.transparent) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(e) => {
                self.failure = Some(e);
                event_loop.exit();
                return;
            }
        }
        #[cfg(target_os = "macos")]
        {
            // Appearance first: the vibrancy material is tinted by the window's appearance.
            crate::macos::set_appearance(&window, self.options.appearance);
            if self.options.vibrancy {
                crate::macos::add_vibrancy(&window, self.options.vibrancy_material);
            }
            if let Some(at) = self.options.traffic_lights {
                crate::macos::place_traffic_lights(&window, at);
            }
        }
        self.pen = Some(crate::pointer::PenInput::install(&window, self.options.raw_pointer_samples));
        #[cfg(target_os = "macos")]
        {
            let proxy = self.proxy.clone();
            self.display_observer = Some(crate::macos::display::DisplayOptionsObserver::new(move || {
                let _ = proxy.send_event(());
            }));
        }
        ask(&mut self.trace, &window, Why::Start);
        self.window = Some(window);
    }

    /// The only user event is "a display option changed" (sent from the system notification).
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        self.dispatch(Event::DisplayOptionsChanged);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(window), Some(renderer)) = (self.window.clone(), self.renderer.as_mut()) else { return };
        let scale = window.scale_factor() as f32;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                #[cfg(target_os = "macos")]
                if let Some(at) = self.options.traffic_lights {
                    crate::macos::place_traffic_lights(&window, at); // AppKit resets them on resize
                }
                renderer.resize(size, scale);
                ask(&mut self.trace, &window, Why::Resize);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                renderer.resize(window.inner_size(), scale);
                ask(&mut self.trace, &window, Why::Resize);
            }
            WindowEvent::RedrawRequested => {
                // Get the frame first: a window that cannot show one (hidden, minimized, screen locked)
                // costs nothing, instead of a full layout per redraw request that is thrown away.
                let frame = match renderer.acquire() {
                    Acquired::Ready(frame) => frame,
                    Acquired::Hidden => {
                        if let Some(t) = &mut self.trace {
                            t.skipped += 1;
                            t.report();
                        }
                        return;
                    }
                    Acquired::Retry => {
                        // Not a redraw request of its own, or a surface that keeps failing would spin.
                        let at = Instant::now() + RETRY_AFTER;
                        self.wake_at = Some(self.wake_at.map_or(at, |w| w.min(at)));
                        if let Some(t) = &mut self.trace {
                            t.skipped += 1;
                            t.asked[Why::Retry as usize] += 1;
                            t.report();
                        }
                        return;
                    }
                };
                self.shaper.begin_frame();
                let platform = WindowPlatform(&window);
                let mut cx = Cx::new(&mut self.shaper, self.modifiers, renderer.logical_size(), scale, &platform, &mut self.wake_at);
                let scene = self.app.scene(&mut cx);
                renderer.render_acquired(frame, &scene, &mut self.shaper);
                self.shaper.end_frame();
                if let Some(t) = &mut self.trace {
                    t.drawn += 1;
                    t.report();
                }
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
            // Frames are skipped while the window is hidden; draw as soon as it is visible again.
            WindowEvent::Occluded(false) => ask(&mut self.trace, &window, Why::Event),
            WindowEvent::HoveredFile(_) => {
                if !self.files_hovered {
                    self.files_hovered = true;
                    self.dispatch(Event::FilesHovered(true));
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if std::mem::take(&mut self.files_hovered) {
                    self.dispatch(Event::FilesHovered(false));
                }
            }
            WindowEvent::DroppedFile(path) => {
                if std::mem::take(&mut self.files_hovered) {
                    self.dispatch(Event::FilesHovered(false));
                }
                self.dropped.push(path);
            }
            WindowEvent::Focused(focused) => self.dispatch(Event::FocusChanged(focused)),
            WindowEvent::ThemeChanged(theme) => self.dispatch(Event::ThemeChanged(system_theme(theme))),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.dropped.is_empty() {
            let files = std::mem::take(&mut self.dropped);
            self.dispatch(Event::FilesDropped(files));
        }
        match self.wake_at {
            Some(at) if Instant::now() >= at => {
                self.wake_at = None;
                if let Some(window) = &self.window {
                    ask(&mut self.trace, window, Why::Timer);
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
