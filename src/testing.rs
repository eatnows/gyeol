//! Running a [`View`] without a window or GPU, for tests: feed it input, look at the scene it paints.
use std::{
    cell::{Cell, RefCell},
    time::Instant,
};

use crate::{
    element::Cursor,
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

    fn set_clipboard_text(&self, text: &str) {
        *self.clipboard.borrow_mut() = Some(text.to_string());
    }
}

/// A [`View`] running headlessly at a fixed window size (scale factor 1).
pub struct TestHost<S: View> {
    host: Host<S>,
    shaper: Shaper,
    platform: Recorder,
    size: (f32, f32),
    modifiers: Modifiers,
    wake_at: Option<Instant>,
    scene: Scene,
    mouse: Option<(f32, f32)>,
}

impl<S: View> TestHost<S> {
    /// Starts `state` in a window of `size` logical pixels and paints the first frame.
    pub fn new(state: S, size: (f32, f32)) -> Self {
        let mut host = TestHost {
            host: Host::new(state),
            shaper: Shaper::new(),
            platform: Recorder::default(),
            size,
            modifiers: Modifiers::default(),
            wake_at: None,
            scene: Scene::default(),
            mouse: None,
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

    /// Paints a frame, as the shell does when the window asks for a redraw.
    pub fn frame(&mut self) -> &Scene {
        self.shaper.begin_frame();
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, self.size, 1., &self.platform, &mut self.wake_at);
        self.scene = self.host.scene(&mut cx);
        self.shaper.end_frame();
        &self.scene
    }

    /// Delivers `event`, then paints a frame (the shell redraws after every event too).
    pub fn event(&mut self, event: Event) -> &Scene {
        let mut cx = Cx::new(&mut self.shaper, self.modifiers, self.size, 1., &self.platform, &mut self.wake_at);
        self.host.event(event, &mut cx);
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
}
