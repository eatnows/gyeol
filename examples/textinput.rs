//! Phase 1: input. Two text fields with mouse selection, keyboard editing, clipboard and IME
//! composition (try typing Korean), plus a live readout of mouse, scroll and modifier state.
//! The fields are the library's `TextInput` / `text_input`.
//!
//!     cargo run --example textinput
//!     cargo run --example textinput -- --demo      # pre-filled, with a selection and a composition
//!     GYEOL_LOG_EVENTS=1 cargo run --example textinput    # print every event to stderr
//!     cargo run --example textinput --features snapshot -- --snapshot out.png [scale]
//!         # headless: out.png (focused field with a selection) and out-ime.png (a Korean composition)
use gyeol::{
    cycle_focus, div, run_view, text, text_input, Color, Cx, Element, Event, Ime, InputChange, InputStyle, Key, NamedKey, ScrollDelta, TextInput, View,
};

const BG: Color = Color::hex(0xf3f5f7);
const SURFACE: Color = Color::hex(0xffffff);
const LINE: Color = Color::hex(0xdce2e8);
const INK: Color = Color::hex(0x14202b);
const DIM: Color = Color::hex(0x4a5866);
const ACCENT: Color = Color::hex(0xb45f06);

struct Demo {
    name: TextInput,
    note: TextInput,
    submitted: Option<String>,
    mouse: (f32, f32),
    clicks: u32,
    scroll: (f32, f32),
    window_focused: bool,
    log_events: bool,
    blink: bool,
}

impl Demo {
    fn new(demo: bool) -> Demo {
        let mut d = Demo { name: TextInput::new(), note: TextInput::new(), submitted: None, mouse: (0., 0.), clicks: 0, scroll: (0., 0.), window_focused: true, log_events: std::env::var_os("GYEOL_LOG_EVENTS").is_some(), blink: true };
        if demo {
            d.name.set_text("결이 만든 UI toolkit");
            d.note.set_text("한글 조합 ");
            d.note.ime(Ime::Preedit { text: "테".into(), cursor: None });
            d.note.focus();
        }
        d
    }

    #[cfg(feature = "snapshot")]
    /// For snapshots: a steady caret, the name focused with "만든" selected, the note still empty.
    fn selection_scene() -> Demo {
        let mut d = Demo::new(false);
        d.blink = false;
        d.name.set_text("결이 만든 UI toolkit");
        d.name.move_to("결이 ".len(), false);
        d.name.move_to("결이 만든".len(), true);
        d.name.focus();
        d
    }

    #[cfg(feature = "snapshot")]
    /// For snapshots: the note focused while a Korean syllable is being composed.
    fn composing_scene() -> Demo {
        let mut d = Demo::new(true);
        d.blink = false;
        d.name.set_text("");
        d
    }
}

fn field(label: &'static str, input: &TextInput, placeholder: &'static str, blink: bool, cx: &mut Cx, get: fn(&mut Demo) -> &mut TextInput) -> Element<Demo> {
    let style = InputStyle::new(18.).color(INK).blink(blink).placeholder(placeholder).selection(ACCENT.with_alpha(0.22)).caret(INK);
    let (border, width) = if input.is_focused() { (ACCENT, 2.) } else { (LINE, 1.) };
    div()
        .gap(6.)
        .child(text(label).text_size(13.).text_color(DIM))
        .child(div().w(420.).px(12.).py(10.).bg(SURFACE).rounded(8.).border(width, border).child(text_input(input, &style, cx, get)))
}

impl View for Demo {
    fn view(&self, cx: &mut Cx) -> Element<Demo> {
        let m = cx.modifiers;
        let mods = [(m.shift, "shift"), (m.ctrl, "ctrl"), (m.alt, "alt"), (m.logo, "cmd/win")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" + ");
        let focus = if self.name.is_focused() { "Name" } else if self.note.is_focused() { "Note" } else { "none" };
        let info = [
            format!("mouse   {:.0}, {:.0}   clicks {}", self.mouse.0, self.mouse.1, self.clicks),
            format!("scroll  {:.1}, {:.1}", self.scroll.0, self.scroll.1),
            format!("keys    {}", if mods.is_empty() { "-".to_string() } else { mods }),
            format!("focus   {}   window {}", focus, if self.window_focused { "active" } else { "inactive" }),
            format!("enter   {}", self.submitted.as_deref().unwrap_or("-")),
        ];
        div()
            .p(40.)
            .gap(28.)
            .child(div().gap(10.).child(text("gyeol  결").text_size(30.).text_color(INK)).child(
                text("Click a field and type. Korean composes with the system input method.").text_size(14.).text_color(DIM),
            ))
            .child(field("Name", &self.name, "Your name", self.blink, cx, |s| &mut s.name))
            .child(field("Note", &self.note, "Anything", self.blink, cx, |s| &mut s.note))
            .child(
                div()
                    .p(16.)
                    .gap(4.)
                    .bg(SURFACE)
                    .rounded(8.)
                    .border(1., LINE)
                    .text_size(13.)
                    .text_color(DIM)
                    .children(info.into_iter().map(|l| text(l))),
            )
    }

    fn event(&mut self, event: &Event, cx: &mut Cx) {
        if self.log_events {
            eprintln!("{event:?}  mods={:?}", cx.modifiers);
        }
        match event {
            Event::MouseMoved { pos } => self.mouse = *pos,
            Event::MousePressed { click_count, .. } => self.clicks = *click_count,
            Event::Scroll { delta, .. } => {
                let (dx, dy) = match *delta {
                    ScrollDelta::Lines(x, y) | ScrollDelta::Pixels(x, y) => (x, y),
                };
                self.scroll.0 += dx;
                self.scroll.1 += dy;
            }
            Event::FocusChanged(active) => self.window_focused = *active,
            _ => {}
        }
        let changes = [self.name.event(event, cx), self.note.event(event, cx)];
        for change in changes {
            match change {
                InputChange::Tab { back } => cycle_focus(&mut [&mut self.name, &mut self.note], back),
                InputChange::Escape => {
                    self.name.blur();
                    self.note.blur();
                }
                InputChange::Submitted => {
                    let focused = if self.name.is_focused() { &self.name } else { &self.note };
                    self.submitted = Some(focused.text().to_string());
                }
                _ => {}
            }
        }
        // Tab with nothing focused starts at the first field.
        if let Event::KeyDown { key: Key::Named(NamedKey::Tab), .. } = event {
            if !self.name.is_focused() && !self.note.is_focused() {
                cycle_focus(&mut [&mut self.name, &mut self.note], cx.modifiers.shift);
            }
        }
    }

    fn background(&self) -> Color {
        BG
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let scale = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.);
            let ime_path = path.strip_suffix(".png").map_or(format!("{path}-ime"), |p| format!("{p}-ime.png"));
            for (state, out) in [(Demo::selection_scene(), path.clone()), (Demo::composing_scene(), ime_path)] {
                let image = gyeol::snapshot::view(state, (520., 560.), scale).expect("render the snapshot");
                image.save_png(&out).expect("save the png");
                println!("wrote {out} ({}x{})", image.width, image.height);
            }
            return;
        }
    }
    let demo = std::env::args().any(|a| a == "--demo");
    if let Err(e) = run_view("gyeol textinput", Demo::new(demo)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;

    fn host() -> TestHost<Demo> {
        let mut h = TestHost::new(Demo::new(false), (600., 600.));
        h.frame();
        h
    }

    #[test]
    fn korean_composes_in_the_focused_field_and_enter_reports_it() {
        let mut h = host();
        h.click_text("Your name");
        for step in ["ㅎ", "하", "한"] {
            h.ime(Ime::Preedit { text: step.into(), cursor: None });
            assert_eq!(h.state().name.text(), "", "composing must not change the committed text");
        }
        h.ime(Ime::Commit("한".into()));
        h.ime(Ime::Preedit { text: "글".into(), cursor: None });
        h.ime(Ime::Commit("글".into()));
        assert_eq!(h.state().name.text(), "한글");
        h.key(Key::Named(NamedKey::Enter), None);
        assert_eq!(h.state().submitted.as_deref(), Some("한글"));
    }

    #[test]
    fn tab_moves_between_fields_and_escape_leaves() {
        let mut h = host();
        h.key(Key::Named(NamedKey::Tab), None);
        assert!(h.state().name.is_focused());
        h.key(Key::Named(NamedKey::Tab), None);
        assert!(h.state().note.is_focused() && !h.state().name.is_focused());
        h.key(Key::Char("x".into()), Some("x"));
        assert_eq!((h.state().name.text(), h.state().note.text()), ("", "x"));
        h.key(Key::Named(NamedKey::Escape), None);
        assert!(!h.state().note.is_focused());
    }
}
