//! Running a [`View`] without a window or GPU, for tests: feed it input, look at the scene it paints.
use std::{cell::Cell, time::Instant};

use crate::{
    element::Cursor,
    event::{Event, Ime, Key, Modifiers, MouseButton, ScrollDelta},
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

    pub fn mouse_move(&mut self, pos: (f32, f32)) -> &Scene {
        self.event(Event::MouseMoved { pos })
    }

    pub fn mouse_down(&mut self, pos: (f32, f32)) {
        self.mouse_move(pos);
        self.event(Event::MousePressed { button: MouseButton::Left, pos, click_count: 1 });
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
        let (w, h) = (self.shaper.width(&t.content, t.size), Shaper::line_height(t.size));
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
}
