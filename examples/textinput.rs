//! Phase 1: input. Two text fields with mouse selection, keyboard editing and IME composition
//! (try typing Korean), plus a live readout of mouse, scroll and modifier state.
//!
//!     cargo run --example textinput
//!     cargo run --example textinput -- --demo      # pre-filled, with a selection and a composition
//!     GYEOL_LOG_EVENTS=1 cargo run --example textinput    # print every event to stderr
use std::time::{Duration, Instant};

use gyeol::{Color, Cx, Event, Ime, Key, Modifiers, MouseButton, NamedKey, Quad, Rect, Scene, ScrollDelta, Shaper, Text};

const SIZE: f32 = 18.;
const PAD: f32 = 12.;
const BLINK: Duration = Duration::from_millis(530);

const BG: Color = Color::hex(0xf3f5f7);
const SURFACE: Color = Color::hex(0xffffff);
const LINE: Color = Color::hex(0xdce2e8);
const INK: Color = Color::hex(0x14202b);
const DIM: Color = Color::hex(0x4a5866);
const ACCENT: Color = Color::hex(0xb45f06);

/// One line of editable text: the committed text, a caret with an optional selection, and the
/// provisional text an input method is composing.
struct Field {
    label: &'static str,
    text: String,
    caret: usize,
    anchor: usize,
    preedit: String,
    rect: Rect,
}

impl Field {
    fn new(label: &'static str, rect: Rect) -> Field {
        Field { label, text: String::new(), caret: 0, anchor: 0, preedit: String::new(), rect }
    }

    fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn insert(&mut self, s: &str) {
        let (a, b) = self.selection();
        self.text.replace_range(a..b, s);
        self.caret = a + s.len();
        self.anchor = self.caret;
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        self.text.replace_range(a..b, "");
        self.caret = a;
        self.anchor = a;
        true
    }

    fn move_to(&mut self, index: usize, extend: bool) {
        self.caret = index;
        if !extend {
            self.anchor = index;
        }
    }

    fn prev(&self, from: usize) -> usize {
        self.text[..from].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    fn next(&self, from: usize) -> usize {
        self.text[from..].chars().next().map_or(from, |c| from + c.len_utf8())
    }

    fn word_start(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && !is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        while i > 0 && is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        i
    }

    fn word_end(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.text.len() && !is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        while i < self.text.len() && is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        i
    }

    /// The word around `index`, for double-click selection.
    fn word_at(&self, index: usize) -> (usize, usize) {
        let on_word = |i: usize| self.text[i..].chars().next().is_some_and(is_word);
        let start = if index > 0 && !on_word(index) { self.prev(index) } else { index };
        if !on_word(start) {
            return (index, index);
        }
        let mut a = start;
        while a > 0 && is_word(self.text[..a].chars().next_back().unwrap()) {
            a = self.prev(a);
        }
        let mut b = start;
        while b < self.text.len() && is_word(self.text[b..].chars().next().unwrap()) {
            b = self.next(b);
        }
        (a, b)
    }

    /// Input-method events: composition shows provisional text, a commit inserts the final text.
    fn ime(&mut self, ime: Ime) {
        match ime {
            Ime::Preedit { text, .. } => self.preedit = text,
            Ime::Commit(text) => {
                self.preedit.clear();
                self.insert(&text);
            }
        }
    }

    /// Keyboard editing. While an input method is composing it owns the keys, so they are ignored here.
    fn key(&mut self, key: &Key, text: Option<&str>, mods: Modifiers) {
        if !self.preedit.is_empty() {
            return;
        }
        let word = if cfg!(target_os = "macos") { mods.alt } else { mods.ctrl };
        match key {
            Key::Named(NamedKey::Backspace) => {
                if !self.delete_selection() && self.caret > 0 {
                    let from = if word { self.word_start(self.caret) } else { self.prev(self.caret) };
                    self.text.replace_range(from..self.caret, "");
                    self.caret = from;
                    self.anchor = from;
                }
            }
            Key::Named(NamedKey::Delete) => {
                if !self.delete_selection() && self.caret < self.text.len() {
                    let to = if word { self.word_end(self.caret) } else { self.next(self.caret) };
                    self.text.replace_range(self.caret..to, "");
                }
            }
            Key::Named(NamedKey::ArrowLeft) if mods.command() => self.move_to(0, mods.shift),
            Key::Named(NamedKey::ArrowRight) if mods.command() => self.move_to(self.text.len(), mods.shift),
            Key::Named(NamedKey::ArrowLeft) => {
                let (a, _) = self.selection();
                let to = if word { self.word_start(self.caret) } else if self.caret != self.anchor && !mods.shift { a } else { self.prev(self.caret) };
                self.move_to(to, mods.shift);
            }
            Key::Named(NamedKey::ArrowRight) => {
                let (_, b) = self.selection();
                let to = if word { self.word_end(self.caret) } else if self.caret != self.anchor && !mods.shift { b } else { self.next(self.caret) };
                self.move_to(to, mods.shift);
            }
            Key::Named(NamedKey::Home) => self.move_to(0, mods.shift),
            Key::Named(NamedKey::End) => self.move_to(self.text.len(), mods.shift),
            Key::Char(c) if mods.command() && c.eq_ignore_ascii_case("a") => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            _ if mods.command() => {}
            _ => {
                if let Some(t) = text {
                    self.insert(t);
                }
            }
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Demo {
    fields: Vec<Field>,
    focus: Option<usize>,
    dragging: bool,
    mouse: (f32, f32),
    clicks: u32,
    scroll: (f32, f32),
    window_focused: bool,
    ime_ready: bool,
    blink_epoch: Instant,
    log_events: bool,
}

impl Demo {
    fn new(demo: bool) -> Demo {
        let mut name = Field::new("Name", Rect::new(40., 150., 420., 44.));
        let mut note = Field::new("Note", Rect::new(40., 240., 420., 44.));
        let mut focus = None;
        if demo {
            name.text = "결이 만든 UI toolkit".into();
            name.anchor = "결이 ".len();
            name.caret = "결이 만든".len();
            note.text = "한글 조합 ".into();
            note.caret = note.text.len();
            note.anchor = note.caret;
            note.preedit = "테".into();
            focus = Some(1);
        }
        Demo {
            fields: vec![name, note],
            focus,
            dragging: false,
            mouse: (0., 0.),
            clicks: 0,
            scroll: (0., 0.),
            window_focused: true,
            ime_ready: false,
            blink_epoch: Instant::now(),
            log_events: std::env::var_os("GYEOL_LOG_EVENTS").is_some(),
        }
    }

    fn text_x(field: &Field) -> f32 {
        field.rect.x + PAD
    }
}

impl gyeol::App for Demo {
    fn event(&mut self, event: Event, cx: &mut Cx) {
        if self.log_events {
            eprintln!("{event:?}  mods={:?}", cx.modifiers);
        }
        match event {
            Event::MouseMoved { pos } => {
                self.mouse = pos;
                if let (true, Some(i)) = (self.dragging, self.focus) {
                    let field = &mut self.fields[i];
                    let index = cx.shaper.hit(&field.text, SIZE, pos.0 - Self::text_x(field));
                    field.caret = index;
                }
            }
            Event::MousePressed { button: MouseButton::Left, pos, click_count } => {
                self.clicks = click_count;
                self.focus = self.fields.iter().position(|f| f.rect.contains(pos));
                self.blink_epoch = Instant::now();
                if let Some(i) = self.focus {
                    let field = &mut self.fields[i];
                    field.preedit.clear();
                    let index = cx.shaper.hit(&field.text, SIZE, pos.0 - Self::text_x(field));
                    if click_count >= 2 {
                        let (a, b) = field.word_at(index);
                        field.anchor = a;
                        field.caret = b;
                    } else {
                        field.move_to(index, cx.modifiers.shift);
                        self.dragging = true;
                    }
                }
            }
            Event::MouseReleased { button: MouseButton::Left, .. } => self.dragging = false,
            Event::Scroll { delta, .. } => {
                let (dx, dy) = match delta {
                    ScrollDelta::Lines(x, y) => (x, y),
                    ScrollDelta::Pixels(x, y) => (x, y),
                };
                self.scroll.0 += dx;
                self.scroll.1 += dy;
            }
            Event::KeyDown { key, text, .. } => {
                self.blink_epoch = Instant::now();
                match (&key, self.focus) {
                    (Key::Named(NamedKey::Tab), _) => {
                        let step = if cx.modifiers.shift { self.fields.len() - 1 } else { 1 };
                        self.focus = Some(self.focus.map_or(0, |i| (i + step) % self.fields.len()));
                    }
                    (Key::Named(NamedKey::Escape), _) => self.focus = None,
                    (_, Some(i)) => self.fields[i].key(&key, text.as_deref(), cx.modifiers),
                    _ => {}
                }
            }
            Event::Ime(ime) => {
                self.blink_epoch = Instant::now();
                if let Some(i) = self.focus {
                    self.fields[i].ime(ime);
                }
            }
            Event::FocusChanged(focused) => self.window_focused = focused,
            _ => {}
        }
    }

    fn scene(&mut self, cx: &mut Cx) -> Scene {
        if !self.ime_ready {
            cx.set_ime_allowed(true);
            self.ime_ready = true;
        }
        let (w, h) = cx.size();
        let line_h = Shaper::line_height(SIZE);
        let mut scene = Scene { background: Some(BG), ..Default::default() };

        scene.texts.push(Text::new((40., 40.), "gyeol  결", 30., INK));
        scene.texts.push(Text::new((40., 84.), "Click a field and type. Korean composes with the system input method.", 14., DIM));

        let blink_on = (self.blink_epoch.elapsed().as_millis() / BLINK.as_millis()) % 2 == 0;
        let to_next_blink = BLINK.as_millis() - self.blink_epoch.elapsed().as_millis() % BLINK.as_millis();

        for (i, field) in self.fields.iter().enumerate() {
            let focused = self.focus == Some(i);
            let r = field.rect;
            scene.texts.push(Text::new((r.x, r.y - 22.), field.label, 13., DIM));
            let border = if focused { ACCENT } else { LINE };
            scene.quads.push(Quad::new(r, SURFACE).rounded(8.).bordered(if focused { 2. } else { 1. }, border));

            let tx = Self::text_x(field);
            let ty = r.y + (r.h - line_h) / 2.;
            let (sel_a, sel_b) = field.selection();
            let composing = !field.preedit.is_empty();

            // What is drawn: the text with the composition spliced in at the caret.
            let mut shown = field.text.clone();
            shown.insert_str(field.caret, &field.preedit);

            if focused && !composing && sel_a != sel_b {
                let x0 = cx.shaper.caret_x(&field.text, SIZE, sel_a);
                let x1 = cx.shaper.caret_x(&field.text, SIZE, sel_b);
                scene.quads.push(Quad::new(Rect::new(tx + x0, ty, x1 - x0, line_h), ACCENT.with_alpha(0.22)).rounded(2.));
            }
            scene.texts.push(Text::new((tx, ty), shown.clone(), SIZE, INK));

            let caret_byte = field.caret + field.preedit.len();
            let caret_x = tx + cx.shaper.caret_x(&shown, SIZE, caret_byte);
            if composing && focused {
                let x0 = tx + cx.shaper.caret_x(&shown, SIZE, field.caret);
                scene.quads.push(Quad::new(Rect::new(x0, ty + line_h - 3., caret_x - x0, 1.5), INK));
            }
            if focused {
                cx.set_ime_cursor_area(Rect::new(caret_x, ty, 1., line_h));
                if blink_on && self.window_focused && (composing || sel_a == sel_b) {
                    scene.quads.push(Quad::new(Rect::new(caret_x, ty + 2., 1.5, line_h - 4.), INK));
                }
            }
        }

        let m = cx.modifiers;
        let mods = [(m.shift, "shift"), (m.ctrl, "ctrl"), (m.alt, "alt"), (m.logo, "cmd/win")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" + ");
        let info = [
            format!("mouse   {:.0}, {:.0}   clicks {}", self.mouse.0, self.mouse.1, self.clicks),
            format!("scroll  {:.1}, {:.1}", self.scroll.0, self.scroll.1),
            format!("keys    {}", if mods.is_empty() { "-".to_string() } else { mods }),
            format!("focus   {}   window {}", self.focus.map_or("none", |i| self.fields[i].label), if self.window_focused { "active" } else { "inactive" }),
        ];
        scene.quads.push(Quad::new(Rect::new(40., h - 140., w - 80., 100.), SURFACE).rounded(8.).bordered(1., LINE));
        for (i, line) in info.into_iter().enumerate() {
            scene.texts.push(Text::new((56., h - 130. + i as f32 * 22.), line, 13., DIM));
        }

        if self.focus.is_some() && self.window_focused {
            cx.request_redraw_after(Duration::from_millis(to_next_blink as u64));
        }
        scene
    }
}

fn main() {
    let demo = std::env::args().any(|a| a == "--demo");
    if let Err(e) = gyeol::run("gyeol textinput", Demo::new(demo)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> Field {
        let mut f = Field::new("t", Rect::default());
        f.text = text.into();
        f.caret = text.len();
        f.anchor = f.caret;
        f
    }

    fn press(f: &mut Field, key: Key) {
        f.key(&key, None, Modifiers::default());
    }

    fn type_text(f: &mut Field, s: &str) {
        f.key(&Key::Char(s.into()), Some(s), Modifiers::default());
    }

    #[test]
    fn korean_composition_becomes_text_only_on_commit() {
        let mut f = field("");
        // ㅎ → 하 → 한 → (next key starts a new syllable) commit 한, compose ㄱ → 그 → 글 → commit 글
        for step in [
            Ime::Preedit { text: "ㅎ".into(), cursor: None },
            Ime::Preedit { text: "하".into(), cursor: None },
            Ime::Preedit { text: "한".into(), cursor: None },
        ] {
            f.ime(step);
            assert_eq!(f.text, "", "composing must not change the committed text");
        }
        f.ime(Ime::Commit("한".into()));
        f.ime(Ime::Preedit { text: "ㄱ".into(), cursor: None });
        f.ime(Ime::Preedit { text: "글".into(), cursor: None });
        assert_eq!((f.text.as_str(), f.preedit.as_str()), ("한", "글"));
        f.ime(Ime::Commit("글".into()));
        assert_eq!((f.text.as_str(), f.preedit.as_str(), f.caret), ("한글", "", "한글".len()));
    }

    #[test]
    fn keys_are_ignored_while_composing() {
        let mut f = field("가");
        f.ime(Ime::Preedit { text: "ㄴ".into(), cursor: None });
        press(&mut f, Key::Named(NamedKey::Backspace));
        type_text(&mut f, "x");
        assert_eq!(f.text, "가", "the input method owns the keyboard until it commits");
    }

    #[test]
    fn a_commit_replaces_the_selection() {
        let mut f = field("hello world");
        f.anchor = 0;
        f.caret = 5;
        f.ime(Ime::Commit("안녕".into()));
        assert_eq!(f.text, "안녕 world");
        assert_eq!(f.caret, "안녕".len());
    }

    #[test]
    fn backspace_and_arrows_step_over_whole_korean_characters() {
        let mut f = field("결의");
        press(&mut f, Key::Named(NamedKey::ArrowLeft));
        assert_eq!(f.caret, "결".len());
        press(&mut f, Key::Named(NamedKey::Backspace));
        assert_eq!(f.text, "의");
        assert_eq!(f.caret, 0);
        press(&mut f, Key::Named(NamedKey::Delete));
        assert_eq!(f.text, "");
        press(&mut f, Key::Named(NamedKey::Backspace)); // nothing left: must not panic
    }

    #[test]
    fn shift_arrows_select_and_typing_replaces() {
        let mut f = field("abc");
        f.move_to(0, false);
        let shift = Modifiers { shift: true, ..Default::default() };
        f.key(&Key::Named(NamedKey::ArrowRight), None, shift);
        f.key(&Key::Named(NamedKey::ArrowRight), None, shift);
        assert_eq!(f.selection(), (0, 2));
        type_text(&mut f, "Z");
        assert_eq!(f.text, "Zc");
    }

    #[test]
    fn select_all_and_word_movement() {
        let mut f = field("foo bar baz");
        let word = if cfg!(target_os = "macos") { Modifiers { alt: true, ..Default::default() } } else { Modifiers { ctrl: true, ..Default::default() } };
        f.key(&Key::Named(NamedKey::ArrowLeft), None, word);
        assert_eq!(f.caret, "foo bar ".len());
        f.key(&Key::Named(NamedKey::Backspace), None, word);
        assert_eq!(f.text, "foo baz");
        let cmd = if cfg!(target_os = "macos") { Modifiers { logo: true, ..Default::default() } } else { Modifiers { ctrl: true, ..Default::default() } };
        f.key(&Key::Char("a".into()), Some("a"), cmd);
        assert_eq!(f.selection(), (0, f.text.len()));
        assert_eq!(f.text, "foo baz", "the shortcut must not type its letter");
    }

    #[test]
    fn double_click_selects_a_word() {
        let f = field("hello 한글 world");
        let i = "hello 한".len();
        assert_eq!(f.word_at(i), ("hello ".len(), "hello 한글".len()));
        assert_eq!(f.word_at("hello".len()), (0, "hello".len()), "just after a word still picks that word");
    }
}
