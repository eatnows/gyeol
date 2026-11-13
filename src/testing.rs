//! Running a [`View`] without a window or GPU, for tests: feed it input, look at the scene it paints.
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    element::Cursor,
    pointer::PointerSample,
    event::{Event, Ime, Key, Modifiers, MouseButton, ScrollDelta, SystemTheme},
    scene::{Rect, Scene},
    shaper::Shaper,
    shell::{App, Cx, Platform},
    view::{Host, View},
};

/// Remembers what the app asked the window for.
#[derive(Default)]
struct Recorder {
    cursor: Cell<Cursor>,
    ime_allowed: Cell<bool>,
    ime_area: Cell<Option<Rect>>,
    theme: Cell<Option<SystemTheme>>,
    reduce_transparency: Cell<bool>,
    next_pick: RefCell<Option<PathBuf>>,
    picks: RefCell<Vec<String>>,
    clipboard: RefCell<Option<String>>,
}

impl Platform for Recorder {
    fn set_ime_allowed(&self, allowed: bool) {
        self.ime_allowed.set(allowed);
    }

    fn set_ime_cursor_area(&self, area: Rect) {
        self.ime_area.set(Some(area));
    }

    fn set_cursor(&self, cursor: Cursor) {
        self.cursor.set(cursor);
    }

    fn theme(&self) -> Option<SystemTheme> {
        self.theme.get()
    }

    fn clipboard_text(&self) -> Option<String> {
        self.clipboard.borrow().clone()
    }

    fn reduce_transparency(&self) -> bool {
        self.reduce_transparency.get()
    }

    fn pick_file(&self, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        self.picks.borrow_mut().push(format!("open {}", describe(filters)));
        self.next_pick.borrow_mut().take()
    }

    fn pick_path(&self, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        self.picks.borrow_mut().push(format!("path {}", describe(filters)));
        self.next_pick.borrow_mut().take()
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        self.picks.borrow_mut().push("folder".to_string());
        self.next_pick.borrow_mut().take()
    }

    fn pick_save_path(&self, default_name: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        self.picks.borrow_mut().push(format!("save {default_name} {}", describe(filters)));
        self.next_pick.borrow_mut().take()
    }

    fn set_clipboard_text(&self, text: &str) {
        *self.clipboard.borrow_mut() = Some(text.to_string());
    }
}

fn describe(filters: &[(&str, &[&str])]) -> String {
    filters.iter().map(|(name, exts)| format!("{name}:{}", exts.join(","))).collect::<Vec<_>>().join(";")
}

/// A [`View`] running headlessly at a fixed window size (scale factor 1).
pub struct TestHost<S: View> {
    host: Host<S>,
    shaper: Shaper,
    platform: Recorder,
    size: (f32, f32),
    scale: f32,
    modifiers: Modifiers,
    wake_at: Option<Instant>,
    exit: bool,
    /// The host's own time: it moves only through [`TestHost::advance`].
    clock: Instant,
    scene: Scene,
    mouse: Option<(f32, f32)>,
    /// Pen samples to attach to the next pointer event.
    pointer: Vec<PointerSample>,
}

impl<S: View> TestHost<S> {
    /// Starts `state` in a window of `size` logical pixels and paints the first frame.
    pub fn new(state: S, size: (f32, f32)) -> Self {
        Self::with_scale(state, size, 1.)
    }

    /// Like [`TestHost::new`], on a display with `scale` device pixels per logical pixel (what the app
    /// sees as `cx.scale_factor()`; sizes and the scene stay in logical pixels).
    pub fn with_scale(state: S, size: (f32, f32), scale: f32) -> Self {
        let mut host = TestHost {
            host: Host::new(state),
            shaper: Shaper::new(),
            platform: Recorder::default(),
            size,
            scale,
            modifiers: Modifiers::default(),
            wake_at: None,
            exit: false,
            clock: Instant::now(),
            scene: Scene::default(),
            mouse: None,
            pointer: Vec::new(),
        };
        host.frame();
        host
    }

    pub fn state(&self) -> &S {
        &self.host.state
    }

    pub fn state_mut(&mut self) -> &mut S {
        &mut self.host.state
    }

    /// The scene of the latest frame.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The cursor the app last asked for.
    pub fn cursor(&self) -> Cursor {
        self.platform.cursor.get()
    }

    pub fn ime_allowed(&self) -> bool {
        self.platform.ime_allowed.get()
    }

    pub fn ime_cursor_area(&self) -> Option<Rect> {
        self.platform.ime_area.get()
    }

    /// Pretends the operating system switched to `theme` and delivers the change to the app.
    pub fn set_system_theme(&mut self, theme: SystemTheme) -> &Scene {
        self.platform.theme.set(Some(theme));
        self.event(Event::ThemeChanged(theme))
    }

    /// Pretends the user switched the system's Reduce Transparency option and delivers
    /// [`Event::DisplayOptionsChanged`]; `Cx::reduce_transparency` reports `reduce` from now on.
    pub fn set_reduce_transparency(&mut self, reduce: bool) -> &Scene {
        self.platform.reduce_transparency.set(reduce);
        self.event(Event::DisplayOptionsChanged)
    }

    /// Simulates the user closing the window (close button, Cmd+Q). Asks the app through
    /// [`View::close_requested`]; returns whether the app would exit, which [`TestHost::exited`] also
    /// reports afterwards.
    pub fn request_close_for_test(&mut self) -> bool {
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, self.size, self.scale, &self.platform, &mut self.wake_at).with_now(self.clock);
        let allowed = self.host.close_requested(&mut cx);
        self.exit |= allowed || cx.close;
        self.frame();
        self.exit
    }

    /// Whether the app has closed the window: it allowed a close request or called `Cx::request_close`.
    pub fn exited(&self) -> bool {
        self.exit
    }

    /// Delivers a drop of `files` (as one [`Event::FilesDropped`], after a hover that ends).
    pub fn drop_files(&mut self, files: Vec<PathBuf>) -> &Scene {
        self.event(Event::FilesHovered(false));
        self.event(Event::FilesDropped(files))
    }

    /// Delivers [`Event::FilesHovered`] as if files were dragged over (or off) the window.
    pub fn hover_files(&mut self, over: bool) -> &Scene {
        self.event(Event::FilesHovered(over))
    }

    /// What the next `Cx::pick_file` / `pick_path` / `pick_folder` / `pick_save_path` returns (once; afterwards `None`, as if
    /// cancelled). `None` here means the user cancels.
    pub fn set_next_pick(&mut self, path: Option<PathBuf>) {
        *self.platform.next_pick.borrow_mut() = path;
    }

    /// The dialogs the app opened so far, as `"open Images:png,jpg"`, `"path ..."`, `"folder"` / `"save name.png Images:png"`.
    pub fn picks(&self) -> Vec<String> {
        self.platform.picks.borrow().clone()
    }

    /// The text on the (fake) clipboard.
    pub fn clipboard(&self) -> Option<String> {
        self.platform.clipboard.borrow().clone()
    }

    pub fn set_clipboard(&mut self, text: &str) {
        *self.platform.clipboard.borrow_mut() = Some(text.to_string());
    }

    /// How far the scroll container `id` is scrolled.
    pub fn scroll_offset(&self, id: impl std::hash::Hash) -> (f32, f32) {
        self.host.scroll_offset(crate::element::ElementId::new(id))
    }

    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Resizes the window and paints a frame.
    pub fn resize(&mut self, size: (f32, f32)) {
        self.size = size;
        self.frame();
    }

    /// Lets `duration` of virtual time pass, running the frames that `Cx::request_redraw_after` asked for
    /// along the way: each runs at the moment it was due (so [`View::tick`] sees that time) and may ask
    /// for the next. Returns the latest scene.
    pub fn advance(&mut self, duration: Duration) -> &Scene {
        let target = self.clock + duration;
        // A guard against an app that asks to be redrawn immediately and forever.
        for _ in 0..100_000 {
            match self.wake_at {
                Some(at) if at <= target => {
                    self.clock = self.clock.max(at);
                    self.wake_at = None;
                    self.frame();
                }
                _ => break,
            }
        }
        self.clock = target;
        &self.scene
    }

    /// The host's current virtual time (what `View::tick` and `Cx::now` report).
    pub fn now(&self) -> Instant {
        self.clock
    }

    /// When the app asked to be redrawn next, if it did.
    pub fn next_wake(&self) -> Option<Instant> {
        self.wake_at
    }

    /// Paints a frame, as the shell does when the window asks for a redraw.
    pub fn frame(&mut self) -> &Scene {
        self.shaper.begin_frame();
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, self.size, self.scale, &self.platform, &mut self.wake_at).with_now(self.clock);
        self.scene = self.host.scene(&mut cx);
        self.shaper.end_frame();
        &self.scene
    }

    /// Draws the latest scene with `renderer`, using this host's fonts so text looks as it was measured.
    pub(crate) fn render(&mut self, renderer: &mut crate::Renderer) -> Vec<u8> {
        self.shaper.begin_frame();
        let pixels = renderer.render_to_image(&self.scene, &mut self.shaper);
        self.shaper.end_frame();
        pixels
    }

    /// Delivers `event`, then paints a frame (the shell redraws after every event too).
    pub fn event(&mut self, event: Event) -> &Scene {
        let samples = std::mem::take(&mut self.pointer);
        let current = samples.last().copied();
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, self.size, self.scale, &self.platform, &mut self.wake_at).with_pointer(samples, current).with_now(self.clock);
        self.host.event(event, &mut cx);
        self.exit |= cx.close;
        self.frame()
    }

    /// Moves the pointer to `pos`. Like a real window system, nothing is sent when it is already there.
    pub fn mouse_move(&mut self, pos: (f32, f32)) -> &Scene {
        if self.mouse == Some(pos) {
            return &self.scene;
        }
        self.mouse = Some(pos);
        self.event(Event::MouseMoved { pos })
    }

    pub fn mouse_down(&mut self, pos: (f32, f32)) {
        self.mouse_down_n(pos, 1);
    }

    /// Presses the primary button as the `click_count`-th click in a row (2 = double click).
    pub fn mouse_down_n(&mut self, pos: (f32, f32), click_count: u32) {
        self.mouse_move(pos);
        self.event(Event::MousePressed { button: MouseButton::Left, pos, click_count });
    }

    pub fn mouse_up(&mut self, pos: (f32, f32)) {
        self.mouse_move(pos);
        self.event(Event::MouseReleased { button: MouseButton::Left, pos });
    }

    /// Presses and releases the primary button at `pos`.
    pub fn click(&mut self, pos: (f32, f32)) {
        self.mouse_down(pos);
        self.mouse_up(pos);
    }

    /// Clicks the middle of the first text in the latest frame that reads `content`.
    pub fn click_text(&mut self, content: &str) {
        let pos = self.text_center(content);
        self.click(pos);
    }

    /// The middle of the first text in the latest frame that reads `content`.
    pub fn text_center(&mut self, content: &str) -> (f32, f32) {
        let Some(t) = self.scene.texts().find(|t| t.content == content) else {
            let all: Vec<&str> = self.scene.texts().map(|t| t.content.as_str()).collect();
            panic!("no text {content:?} on screen; there is {all:?}");
        };
        let (w, h) = (self.shaper.width(&t.content, t.style), Shaper::line_height(t.style.size));
        (t.origin.0 + w / 2., t.origin.1 + h / 2.)
    }

    pub fn scroll(&mut self, pos: (f32, f32), delta: ScrollDelta) -> &Scene {
        self.mouse_move(pos);
        self.event(Event::Scroll { delta, pos })
    }

    /// Delivers a pointer `event` as the platform would when a pen is in use: `samples` (oldest first,
    /// the last being the event's own reading) are what the app sees through `MouseEvent::pressure`,
    /// `pointer`, `tilt` and `Cx::pointer_samples`.
    pub fn pointer_event(&mut self, event: Event, samples: Vec<PointerSample>) -> &Scene {
        if let Event::MouseMoved { pos } = &event {
            self.mouse = Some(*pos);
        }
        self.pointer = samples;
        self.event(event)
    }

    /// A pen touching down at the position of `sample`.
    pub fn pen_down(&mut self, sample: PointerSample) {
        let pos = sample.pos;
        self.pointer_event(Event::MouseMoved { pos }, vec![sample.with_pressure(0.)]);
        self.pointer_event(Event::MousePressed { button: MouseButton::Left, pos, click_count: 1 }, vec![sample]);
    }

    /// A pen moving (dragging when it is down) through `samples`; the event is at the last one.
    pub fn pen_move(&mut self, samples: Vec<PointerSample>) {
        let pos = samples.last().expect("at least one sample").pos;
        self.pointer_event(Event::MouseMoved { pos }, samples);
    }

    /// The pen lifting at `pos`.
    pub fn pen_up(&mut self, pos: (f32, f32)) {
        self.pointer_event(Event::MouseReleased { button: MouseButton::Left, pos }, vec![PointerSample::pen(pos, 0.)]);
    }

    pub fn key(&mut self, key: Key, text: Option<&str>) -> &Scene {
        self.event(Event::KeyDown { key, text: text.map(str::to_string), repeat: false })
    }

    pub fn ime(&mut self, ime: Ime) -> &Scene {
        self.event(Event::Ime(ime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        element::{div, text, Element},
        scene::Color,
    };

    struct Counter {
        count: u32,
    }

    const IDLE: Color = Color::hex(0xdddddd);
    const HOT: Color = Color::hex(0xff8800);

    impl View for Counter {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            div()
                .row()
                .gap(20.)
                .child(div().w(100.).h(40.).bg(IDLE).hover_bg(HOT).on_click(|s: &mut Counter, _| s.count += 1).child(text("inc")))
                .child(div().w(100.).h(40.).bg(IDLE).on_click(|s: &mut Counter, _| s.count = 0).child(text("reset")))
                .child(div().w(100.).h(40.).bg(IDLE).child(text(format!("count {}", self.count))))
        }
    }

    fn app() -> TestHost<Counter> {
        TestHost::new(Counter { count: 0 }, (500., 200.))
    }

    #[test]
    fn clicking_runs_the_handler_and_the_next_frame_shows_the_new_state() {
        let mut host = app();
        host.click_text("inc");
        host.click_text("inc");
        assert_eq!(host.state().count, 2);
        assert!(host.scene().texts().any(|t| t.content == "count 2"));
        host.click_text("reset");
        assert_eq!(host.state().count, 0);
    }

    #[test]
    fn a_click_needs_press_and_release_on_the_same_element() {
        let mut host = app();
        let inc = host.text_center("inc");
        let reset = host.text_center("reset");
        host.mouse_down(inc);
        host.mouse_up(reset);
        assert_eq!(host.state().count, 0, "dragged off the button: not a click");
        host.mouse_down(inc);
        host.mouse_up(inc);
        assert_eq!(host.state().count, 1);
    }

    #[test]
    fn clicking_where_nothing_listens_does_nothing() {
        let mut host = app();
        host.click((450., 150.));
        host.click_text("count 0");
        assert_eq!(host.state().count, 0);
    }

    #[test]
    fn hovering_changes_the_cursor_and_the_painted_color() {
        let mut host = app();
        let inc = host.text_center("inc");
        let scene = host.mouse_move(inc);
        assert_eq!(scene.quads().next().unwrap().background, HOT);
        assert_eq!(host.cursor(), Cursor::Pointer);
        host.mouse_move((450., 150.));
        assert_eq!(host.cursor(), Cursor::Default);
        assert_eq!(host.scene().quads().next().unwrap().background, IDLE);
    }

    struct Rows;

    impl View for Rows {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            let mut list = div().id("rows").h(90.).overflow_y_scroll();
            for i in 0..20 {
                list = list.child(div().h(30.).child(text(format!("row {i}"))));
            }
            div().child(list)
        }
    }

    fn y_of(host: &TestHost<Rows>, content: &str) -> f32 {
        host.scene().texts().find(|t| t.content == content).unwrap().origin.1
    }

    #[test]
    fn the_wheel_scrolls_a_list_and_stops_at_both_ends() {
        let mut host = TestHost::new(Rows, (300., 300.));
        assert_eq!(y_of(&host, "row 1"), 30.);

        host.scroll((10., 10.), ScrollDelta::Pixels(0., -50.));
        assert_eq!(y_of(&host, "row 1"), -20., "content moved up by 50");

        host.scroll((10., 10.), ScrollDelta::Lines(0., -1.));
        assert_eq!(y_of(&host, "row 1"), -60., "one wheel notch is 40 logical pixels");

        host.scroll((10., 10.), ScrollDelta::Pixels(0., -100_000.));
        assert_eq!(y_of(&host, "row 19"), 60., "at the end the last row sits at the bottom of the 90px viewport");

        host.scroll((10., 10.), ScrollDelta::Pixels(0., 100_000.));
        assert_eq!(y_of(&host, "row 0"), 0., "back at the start");
    }

    struct ScrollObserver {
        observed_offset: f32,
    }

    impl View for ScrollObserver {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            let mut list = div().id("observed-list").h(90.).overflow_y_scroll();
            for _ in 0..20 {
                list = list.child(div().h(30.));
            }
            div().child(list)
        }

        fn event(&mut self, event: &Event, cx: &mut Cx) {
            if matches!(event, Event::Scroll { .. }) {
                self.observed_offset = cx.scroll_offset("observed-list").1;
            }
        }
    }

    #[test]
    fn an_app_hook_sees_the_scroll_offset_after_element_routing() {
        let mut host = TestHost::new(ScrollObserver { observed_offset: 0. }, (300., 300.));
        host.scroll((10., 10.), ScrollDelta::Pixels(0., -50.));
        assert_eq!(host.state().observed_offset, 50.);
    }

    #[test]
    fn horizontal_wheel_movement_does_not_move_a_vertical_list() {
        let mut host = TestHost::new(Rows, (300., 300.));
        host.scroll((10., 10.), ScrollDelta::Pixels(-80., 0.));
        assert_eq!(y_of(&host, "row 0"), 0.);
        host.scroll((10., 10.), ScrollDelta::Pixels(-80., -30.));
        assert_eq!(y_of(&host, "row 0"), -30., "only the vertical part counts");
    }

    #[test]
    fn scrolling_outside_the_list_does_nothing() {
        let mut host = TestHost::new(Rows, (300., 300.));
        host.scroll((10., 200.), ScrollDelta::Pixels(0., -50.));
        assert_eq!(y_of(&host, "row 0"), 0.);
    }

    struct Big {
        selected: Option<usize>,
    }

    impl View for Big {
        fn view(&self, cx: &mut Cx) -> Element<Self> {
            let list = crate::list::uniform_list(cx, "big", 100_000, 20., |i| {
                div().on_click(move |s: &mut Big, _| s.selected = Some(i)).child(text(format!("item {i}")))
            });
            div().child(list.h(100.))
        }
    }

    fn texts(host: &TestHost<Big>) -> Vec<String> {
        host.scene().texts().map(|t| t.content.clone()).collect()
    }

    #[test]
    fn a_huge_list_builds_only_the_rows_on_screen() {
        let mut host = TestHost::new(Big { selected: None }, (300., 300.));
        host.frame(); // the second frame knows the real viewport (100px), the first used the window's height
        let shown = texts(&host);
        assert!(shown.len() <= 100 / 20 + 2 * 2 + 1, "5 visible rows plus a little overscan, got {}", shown.len());
        assert_eq!(shown[0], "item 0");

        host.scroll((10., 10.), ScrollDelta::Pixels(0., -10_000.));
        let shown = texts(&host);
        assert!(shown.len() <= 10, "still only a handful of rows, got {}", shown.len());
        let row500 = host.scene().texts().find(|t| t.content == "item 500").expect("row 500 is on screen");
        assert_eq!(row500.origin.1, 0., "500 rows of 20px scrolled off: row 500 is at the top");

        host.click_text("item 502");
        assert_eq!(host.state().selected, Some(502), "clicks reach the right row");
    }

    #[test]
    fn scrolling_to_the_very_end_shows_the_last_rows() {
        let mut host = TestHost::new(Big { selected: None }, (300., 300.));
        host.frame();
        host.scroll((10., 10.), ScrollDelta::Pixels(0., -100_000_000.));
        let shown = texts(&host);
        assert!(shown.contains(&"item 99999".to_string()), "{shown:?}");
        assert!(shown.len() <= 10);
        let last = host.scene().texts().find(|t| t.content == "item 99999").unwrap();
        assert_eq!(last.origin.1, 80., "the last 20px row ends at the bottom of the 100px viewport");
    }

    #[derive(Default)]
    struct Pad {
        log: Vec<String>,
    }

    impl View for Pad {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            let pad = div()
                .id("pad")
                .w(100.)
                .h(100.)
                .bg(Color::hex(0xdddddd))
                .on_mouse_down(|s: &mut Pad, _, e| s.log.push(format!("down {:?} n={}", e.local, e.click_count)))
                .on_drag(|s: &mut Pad, _, e| s.log.push(format!("drag {:?}", e.local)))
                .on_mouse_up(|s: &mut Pad, _, e| s.log.push(format!("up {:?}", e.local)));
            let plain = div().w(100.).h(100.).on_mouse_down(|s: &mut Pad, _, _| s.log.push("plain down".into()));
            div().row().gap(20.).child(div().w(20.).h(20.)).child(pad).child(plain)
        }
    }

    #[test]
    fn a_press_is_captured_by_its_element_until_release_with_local_coordinates() {
        let mut host = TestHost::new(Pad::default(), (400., 200.));
        // The pad starts at x = 20 + 20 = 40.
        host.mouse_down_n((50., 30.), 2);
        host.mouse_move((90., 40.));
        host.mouse_move((300., 150.));
        host.mouse_up((300., 150.));
        host.mouse_move((60., 60.));
        assert_eq!(
            host.state().log,
            ["down (10.0, 30.0) n=2", "drag (50.0, 40.0)", "drag (260.0, 150.0)", "up (260.0, 150.0)"],
            "moves outside the pad still reach it while pressed; nothing after the release"
        );
    }

    #[test]
    fn an_element_with_only_a_press_handler_does_not_capture() {
        let mut host = TestHost::new(Pad::default(), (400., 200.));
        host.mouse_down((200., 50.));
        host.mouse_move((50., 50.));
        host.mouse_up((50., 50.));
        assert_eq!(host.state().log, ["plain down"], "no drag or release events for it");
    }

    /// Keys go to the app's `event` hook; it can use the clipboard and ask for regions to be revealed.
    #[derive(Default)]
    struct Keys {
        typed: String,
        pasted: Option<String>,
    }

    impl View for Keys {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            let mut list = div().id("rows").h(90.).overflow_y_scroll();
            for i in 0..20 {
                list = list.child(div().h(30.).child(text(format!("row {i}"))));
            }
            div().child(list)
        }

        fn event(&mut self, event: &Event, cx: &mut Cx) {
            match event {
                Event::KeyDown { key: Key::Char(c), .. } if c == "c" && cx.modifiers.command() => {
                    cx.set_clipboard_text(&format!("copied:{}", self.typed));
                }
                Event::KeyDown { key: Key::Char(c), .. } if c == "v" && cx.modifiers.command() => self.pasted = cx.clipboard_text(),
                Event::KeyDown { key: Key::Char(c), text: Some(t), .. } if c == "j" => {
                    self.typed.push_str(t);
                    cx.scroll_to_reveal("rows", Rect::new(0., 15. * 30., 10., 30.));
                }
                Event::KeyDown { key: Key::Char(c), .. } if c == "k" => cx.scroll_to_reveal("rows", Rect::new(0., 0., 10., 30.)),
                _ => {}
            }
        }
    }

    #[test]
    fn the_app_hook_gets_keys_and_can_use_the_clipboard() {
        let mut host = TestHost::new(Keys::default(), (300., 300.));
        host.key(Key::Char("j".into()), Some("j"));
        host.set_modifiers(Modifiers { logo: true, ctrl: true, ..Default::default() });
        host.key(Key::Char("c".into()), Some("c"));
        assert_eq!(host.clipboard().as_deref(), Some("copied:j"));
        host.set_clipboard("from elsewhere");
        host.key(Key::Char("v".into()), Some("v"));
        assert_eq!(host.state().pasted.as_deref(), Some("from elsewhere"));
    }

    #[test]
    fn scroll_to_reveal_moves_the_least_needed() {
        let mut host = TestHost::new(Keys::default(), (300., 300.));
        host.key(Key::Char("j".into()), Some("j"));
        assert_eq!(host.scroll_offset("rows").1, 15. * 30. + 30. - 90., "row 15 is brought to the bottom edge");
        host.key(Key::Char("j".into()), Some("j"));
        assert_eq!(host.scroll_offset("rows").1, 15. * 30. + 30. - 90., "already visible: no movement");
        host.key(Key::Char("k".into()), Some("k"));
        assert_eq!(host.scroll_offset("rows").1, 0., "row 0 is brought back to the top");
    }

    struct Slider {
        events: Vec<(&'static str, crate::element::MouseEvent)>,
    }

    impl View for Slider {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            // A 200x20 scrub bar under a 100px spacer, inside a scroll container that is shorter than its content.
            let bar = div()
                .id("bar")
                .w(200.)
                .h(20.)
                .bg(IDLE)
                .on_mouse_down(|s: &mut Slider, _, e| s.events.push(("down", e)))
                .on_drag(|s: &mut Slider, _, e| s.events.push(("drag", e)))
                .on_mouse_up(|s: &mut Slider, _, e| s.events.push(("up", e)));
            div().child(div().id("list").h(150.).overflow_y_scroll().child(div().h(100.)).child(bar).child(div().h(300.)))
        }
    }

    #[test]
    fn mouse_events_carry_the_size_of_the_element_for_fractions_also_while_dragging_and_scrolled() {
        let mut host = TestHost::new(Slider { events: Vec::new() }, (400., 300.));
        // The bar is at y 100..120. Press 50px across it.
        host.mouse_down((50., 110.));
        let (kind, down) = host.state().events[0];
        assert_eq!(kind, "down");
        assert_eq!((down.size, down.local), ((200., 20.), (50., 10.)));
        assert_eq!(down.local.0 / down.size.0, 0.25, "a quarter of the way across");

        // Drag far outside the bar (to the right and below it): still the bar's size, local runs past it.
        host.mouse_move((390., 290.));
        let (kind, drag) = *host.state().events.last().unwrap();
        assert_eq!(kind, "drag");
        assert_eq!(drag.size, (200., 20.), "the captured element's size, not what is under the pointer");
        assert_eq!(drag.local, (390., 190.));
        host.mouse_up((10., 5.));
        let (kind, up) = *host.state().events.last().unwrap();
        assert_eq!((kind, up.size, up.local), ("up", (200., 20.), (10., -95.)));

        // Scroll the list by 40: the bar is at y 60..80 now, and `local` is measured from where it is.
        host.scroll((10., 10.), ScrollDelta::Pixels(0., -40.));
        host.mouse_down((100., 70.));
        let (_, down) = *host.state().events.last().unwrap();
        assert_eq!((down.size, down.local), ((200., 20.), (100., 10.)), "scrolling included");
    }

    struct Divider;

    impl View for Divider {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            let glow = Color::hex(0xff8800);
            div().child(
                div()
                    .w(300.)
                    .h(2.)
                    .bg(Color::hex(0x444444))
                    .hover(|h| h.bg_layers([crate::scene::Gradient::linear(90.).stop(Color::TRANSPARENT).stop(glow).stop(Color::TRANSPARENT)]).shadows([crate::scene::Shadow::new(0., 0., 10., 0., glow)])),
            )
        }
    }

    #[test]
    fn hover_style_appears_when_the_pointer_enters_and_goes_when_it_leaves() {
        let mut host = TestHost::new(Divider, (400., 100.));
        let quad = |host: &TestHost<Divider>| *host.scene().quads().next().unwrap();
        host.mouse_move((350., 50.)); // the host starts with the pointer at (0, 0), which is over the divider
        assert!(quad(&host).gradients.is_empty() && quad(&host).shadows.is_empty());
        host.mouse_move((100., 1.));
        assert_eq!((quad(&host).gradients.len(), quad(&host).shadows.len()), (1, 1), "entered");
        host.mouse_move((350., 50.));
        assert!(quad(&host).gradients.is_empty() && quad(&host).shadows.is_empty(), "left");
    }

    struct Cursors;

    impl View for Cursors {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            div()
                .row()
                .child(div().w(50.).h(50.).cursor(Cursor::ResizeHorizontal))
                .child(div().w(50.).h(50.).cursor(Cursor::ResizeVertical))
                .child(div().w(50.).h(50.).cursor(Cursor::Grab))
                .child(div().w(50.).h(50.).cursor(Cursor::Grabbing))
                .child(div().w(50.).h(50.).cursor(Cursor::NotAllowed))
        }
    }

    #[test]
    fn the_new_cursors_are_reported_over_their_elements_and_map_to_window_cursors() {
        let mut host = TestHost::new(Cursors, (400., 100.));
        for (x, cursor) in [(25., Cursor::ResizeHorizontal), (75., Cursor::ResizeVertical), (125., Cursor::Grab), (175., Cursor::Grabbing), (225., Cursor::NotAllowed), (350., Cursor::Default)] {
            host.mouse_move((x, 25.));
            assert_eq!(host.cursor(), cursor);
        }
        use winit::window::CursorIcon;
        assert_eq!(CursorIcon::from(Cursor::ResizeHorizontal), CursorIcon::ColResize);
        assert_eq!(CursorIcon::from(Cursor::ResizeVertical), CursorIcon::RowResize);
        assert_eq!(CursorIcon::from(Cursor::Grab), CursorIcon::Grab);
        assert_eq!(CursorIcon::from(Cursor::Grabbing), CursorIcon::Grabbing);
        assert_eq!(CursorIcon::from(Cursor::NotAllowed), CursorIcon::NotAllowed);
    }

    struct Menu {
        outside: u32,
        inside_clicks: u32,
        opened: bool,
    }

    impl View for Menu {
        fn view(&self, _: &mut Cx) -> Element<Self> {
            // A 100x40 panel (at 20,20) with a descendant sticking out below it, a banner overlay that
            // covers part of the panel, and a bystander.
            let panel = div()
                .id("panel")
                .absolute()
                .left(20.)
                .top(20.)
                .w(100.)
                .h(40.)
                .bg(IDLE)
                .on_mouse_down_outside(|s: &mut Menu, _| s.outside += 1)
                .on_click(|s: &mut Menu, _| s.inside_clicks += 1)
                .child(div().absolute().left(10.).top(30.).w(30.).h(40.).bg(HOT).on_click(|s: &mut Menu, _| s.inside_clicks += 10));
            let banner = div().overlay().left(100.).top(20.).w(60.).h(40.).bg(HOT).on_click(|_, _| {});
            div().child(panel).child(if self.opened { banner } else { div() }).child(div().absolute().left(200.).top(100.).w(50.).h(50.).bg(IDLE))
        }
    }

    #[test]
    fn an_outside_press_fires_only_outside_the_element_its_descendants_and_not_under_an_overlay() {
        let mut host = TestHost::new(Menu { outside: 0, inside_clicks: 0, opened: false }, (400., 300.));
        host.mouse_down((50., 40.)); // inside the panel
        host.mouse_down((40., 80.)); // on the descendant, below the panel's box
        assert_eq!(host.state().outside, 0, "presses on the element and on its descendants are not outside");
        host.mouse_down((300., 200.)); // far away
        host.mouse_down((110., 40.)); // inside the panel's box (20..120), no overlay yet
        assert_eq!(host.state().outside, 1);
        host.state_mut().opened = true;
        host.frame();
        host.mouse_down((110., 40.)); // still inside the panel's box, but the banner overlay (100..160) covers it
        assert_eq!(host.state().outside, 2, "a press that lands on an overlay above counts as outside");
        host.mouse_down((60., 40.)); // inside the panel, away from the banner
        assert_eq!(host.state().outside, 2);
        host.click((110., 40.));
        assert_eq!(host.state().inside_clicks, 0, "and the overlay blocked the click on the panel under it");
    }

    #[derive(Default)]
    struct PenPad {
        log: Vec<(&'static str, crate::PointerKind, f32, Option<(f32, f32)>)>,
        samples_seen: Vec<usize>,
    }

    impl View for PenPad {
        fn view(&self, _cx: &mut Cx) -> Element<PenPad> {
            div().child(div()
                .w(200.)
                .h(200.)
                .on_mouse_down(|s: &mut PenPad, _, e| s.log.push(("down", e.pointer, e.pressure, e.tilt)))
                .on_drag(|s: &mut PenPad, _, e| s.log.push(("drag", e.pointer, e.pressure, e.tilt)))
                .on_mouse_up(|s: &mut PenPad, _, e| s.log.push(("up", e.pointer, e.pressure, e.tilt))))
        }

        fn event(&mut self, event: &Event, cx: &mut Cx) {
            if matches!(event, Event::MouseMoved { .. }) {
                self.samples_seen.push(cx.pointer_samples().len());
            }
        }
    }

    #[test]
    fn handlers_receive_pen_pressure_tilt_and_kind() {
        use crate::{PointerKind, PointerSample};
        let mut h = TestHost::new(PenPad::default(), (300., 300.));
        h.pen_down(PointerSample::pen((20., 20.), 0.4).tilt(0.5, -0.25));
        h.pen_move(vec![PointerSample::pen((30., 20.), 0.6).tilt(0.5, -0.25)]);
        h.pen_up((30., 20.));
        let log = &h.state().log;
        assert_eq!(log[0], ("down", PointerKind::Pen, 0.4, Some((0.5, -0.25))));
        assert_eq!(log[1], ("drag", PointerKind::Pen, 0.6, Some((0.5, -0.25))));
        assert_eq!((log[2].0, log[2].1, log[2].2), ("up", PointerKind::Pen, 0.), "lifting reports zero pressure");
    }

    #[test]
    fn the_eraser_is_told_apart_and_coalesced_samples_reach_cx() {
        use crate::{PointerKind, PointerSample};
        let mut h = TestHost::new(PenPad::default(), (300., 300.));
        h.pen_down(PointerSample::pen((10., 10.), 0.5).eraser());
        let burst = (1..=5).map(|i| PointerSample::pen((10. + i as f32, 10.), i as f32 / 10.).eraser()).collect::<Vec<_>>();
        h.pen_move(burst);
        let log = &h.state().log;
        assert_eq!(log[1].1, PointerKind::Eraser);
        assert_eq!(log[1].2, 0.5, "the event reports its own (last) sample");
        assert_eq!(*h.state().samples_seen.last().unwrap(), 5, "all five samples since the last event");
    }

    #[test]
    fn without_pen_data_a_mouse_has_full_pressure_only_while_pressed() {
        use crate::PointerKind;
        let mut h = TestHost::new(PenPad::default(), (300., 300.));
        h.mouse_down((20., 20.));
        h.mouse_move((40., 20.));
        h.mouse_up((40., 20.));
        let log = &h.state().log;
        assert_eq!(log[0], ("down", PointerKind::Mouse, 1., None));
        assert_eq!(log[1], ("drag", PointerKind::Mouse, 1., None));
        assert_eq!(log[2], ("up", PointerKind::Mouse, 0., None));
        assert!(h.state().samples_seen.iter().all(|n| *n == 0), "no platform samples: the list is empty");
    }

    /// Counts ticks.
    struct Clock {
        ticks: Vec<Instant>,
        interval: Option<Duration>,
    }

    impl View for Clock {
        fn tick(&mut self, now: Instant, cx: &mut Cx) {
            self.ticks.push(now);
            assert_eq!(now, cx.now());
            if let Some(every) = self.interval {
                cx.request_redraw_after(every);
            }
        }

        fn view(&self, _cx: &mut Cx) -> Element<Clock> {
            div().child(text(format!("{} ticks", self.ticks.len())))
        }
    }

    fn clock(interval: Option<Duration>) -> TestHost<Clock> {
        TestHost::new(Clock { ticks: Vec::new(), interval }, (100., 50.))
    }

    #[test]
    fn tick_runs_before_view_on_every_frame_so_the_scene_shows_its_changes() {
        let mut h = clock(None);
        assert_eq!(h.state().ticks.len(), 1, "the first frame ticks");
        assert!(h.scene().texts().any(|t| t.content == "1 ticks"), "view saw the state tick had just changed");
        h.frame();
        h.mouse_move((5., 5.)); // an event also paints a frame
        assert_eq!(h.state().ticks.len(), 3);
        assert!(h.scene().texts().any(|t| t.content == "3 ticks"));
    }

    #[test]
    fn request_redraw_after_in_tick_repeats_on_the_hosts_clock() {
        let every = Duration::from_millis(100);
        let mut h = clock(Some(every));
        let start = h.now();
        assert_eq!(h.state().ticks.len(), 1);
        assert_eq!(h.next_wake(), Some(start + every));
        h.advance(Duration::from_millis(99));
        assert_eq!(h.state().ticks.len(), 1, "not due yet");
        h.advance(Duration::from_millis(1));
        assert_eq!(h.state().ticks.len(), 2);
        h.advance(Duration::from_millis(350));
        let ticks = &h.state().ticks;
        assert_eq!(ticks.len(), 5, "three more fell due within 350ms");
        // Each tick saw the moment it was due, not the end of the advance.
        let offsets: Vec<u128> = ticks.iter().map(|t| t.duration_since(start).as_millis()).collect();
        assert_eq!(offsets, [0, 100, 200, 300, 400]);
        assert_eq!(h.now().duration_since(start), Duration::from_millis(450));
        assert!(h.scene().texts().any(|t| t.content == "5 ticks"));
    }

    #[test]
    fn an_app_that_never_asks_is_not_redrawn_by_time() {
        let mut h = clock(None);
        h.advance(Duration::from_secs(10));
        assert_eq!(h.state().ticks.len(), 1);
        assert_eq!(h.next_wake(), None);
    }

    #[derive(Default)]
    struct Opts {
        events: u32,
        seen: Vec<bool>,
    }

    impl View for Opts {
        fn view(&self, _cx: &mut Cx) -> Element<Opts> {
            div()
        }

        fn event(&mut self, event: &Event, cx: &mut Cx) {
            if matches!(event, Event::DisplayOptionsChanged) {
                self.events += 1;
                self.seen.push(cx.reduce_transparency());
            }
        }
    }

    #[test]
    fn changing_reduce_transparency_updates_cx_and_notifies_the_app() {
        let mut h = TestHost::new(Opts::default(), (50., 50.));
        h.frame();
        assert_eq!(h.state().events, 0);
        h.set_reduce_transparency(true);
        h.set_reduce_transparency(false);
        assert_eq!((h.state().events, h.state().seen.as_slice()), (2, &[true, false][..]), "the app sees each change, with the new value already in effect");
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    use crate::{div, Color, Element};

    struct Guard {
        dirty: bool,
        asked: u32,
    }

    impl View for Guard {
        fn view(&self, _cx: &mut Cx) -> Element<Guard> {
            div().child(div().id("btn").w(50.).h(20.).bg(Color::hex(0x888888)).on_click(|_: &mut Guard, cx| cx.request_close()))
        }
        fn close_requested(&mut self, _cx: &mut Cx) -> bool {
            self.asked += 1;
            !self.dirty
        }
    }

    #[test]
    fn a_refused_close_keeps_the_window_until_the_app_closes_it() {
        let mut h = TestHost::new(Guard { dirty: true, asked: 0 }, (200., 100.));
        assert!(!h.request_close_for_test());
        assert!(!h.exited());
        h.click((10., 10.));
        assert!(h.exited(), "Cx::request_close closes without asking again");
        assert_eq!(h.state().asked, 1);
    }

    #[test]
    fn an_allowed_close_exits_and_the_default_allows() {
        let mut h = TestHost::new(Guard { dirty: false, asked: 0 }, (200., 100.));
        assert!(h.request_close_for_test());
        assert!(h.exited());
        struct Plain;
        impl View for Plain {
            fn view(&self, _: &mut Cx) -> Element<Plain> {
                div()
            }
        }
        assert!(TestHost::new(Plain, (50., 50.)).request_close_for_test());
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use crate::{div, text, Color, Element};

    const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running far past the edge of the bubble";

    struct Bubble(&'static str, bool);

    impl View for Bubble {
        fn view(&self, _cx: &mut Cx) -> Element<Bubble> {
            let t = text(self.0).text_size(14.);
            let t = if self.1 { t.wrap_text() } else { t };
            div().p(10.).child(div().max_w(200.).bg(Color::hex(0xdddddd)).child(t))
        }
    }

    fn bubble(h: &TestHost<Bubble>) -> Rect {
        let q = h.scene().quads().find(|q| q.background.a > 0.9).expect("the bubble quad");
        q.bounds
    }

    #[test]
    fn a_long_sentence_wraps_inside_the_width_and_grows_taller() {
        let one_line = Shaper::line_height(14.);
        let h = TestHost::new(Bubble(LONG, true), (400., 400.));
        let b = bubble(&h);
        assert!(b.w <= 200.01, "stays inside max_width: {}", b.w);
        assert!(b.h >= one_line * 3., "several lines: {}", b.h);
        let t = h.scene().texts().next().unwrap();
        assert!(t.wrap_width.is_some_and(|w| w <= 200.6));
    }

    #[test]
    fn a_short_sentence_and_the_default_stay_on_one_line() {
        let one_line = Shaper::line_height(14.);
        let short = TestHost::new(Bubble("Hi there", true), (400., 400.));
        assert_eq!(bubble(&short).h, one_line);
        let unwrapped = TestHost::new(Bubble(LONG, false), (400., 400.));
        assert_eq!(bubble(&unwrapped).h, one_line, "wrapping is opt-in");
        assert!(unwrapped.scene().texts().next().unwrap().wrap_width.is_none());
    }

    #[test]
    fn explicit_newlines_and_unspaced_korean_break() {
        let one_line = Shaper::line_height(14.);
        let nl = TestHost::new(Bubble("one\ntwo\nthree", true), (400., 400.));
        assert_eq!(bubble(&nl).h, 3. * one_line);
        let ko = TestHost::new(Bubble("가나다라마바사아자차카타파하가나다라마바사아자차카타파하가나다라마바사아자차카타파하", true), (400., 400.));
        let b = bubble(&ko);
        assert!(b.h >= 2. * one_line && b.w <= 200.01, "{b:?}");
    }
}
