//! Guards against closing with unsaved changes: while "Unsaved changes" is checked, the close button
//! and Cmd+Q are refused and a confirmation appears; its button closes for real.
//!
//!     cargo run --example close_guard
use gyeol::{div, run_view_with, text, Color, Cx, Element, View, WindowOptions};

const INK: Color = Color::hex(0x14202b);
const PAPER: Color = Color::hex(0xfbfaf7);
const WARN: Color = Color::hex(0xb3261e);

#[derive(Default)]
struct Guard {
    unsaved: bool,
    confirming: bool,
}

fn button(label: &str, fill: Color) -> Element<Guard> {
    div().id(label.to_string()).px(14.).py(8.).rounded(8.).bg(fill).child(text(label.to_string()).text_size(14.).text_color(PAPER))
}

impl View for Guard {
    fn view(&self, _cx: &mut Cx) -> Element<Guard> {
        let mark = if self.unsaved { "[x] Unsaved changes" } else { "[ ] Unsaved changes" };
        let mut root = div().p(24.).gap(14.).child(div().id("unsaved").on_click(|s: &mut Guard, _| s.unsaved = !s.unsaved).child(text(mark).text_size(18.).text_color(INK)));
        if self.confirming {
            root = root
                .child(text("Close without saving?").text_size(18.).text_color(WARN))
                .child(
                    div()
                        .gap(10.)
                        .child(button("Close anyway", WARN).on_click(|_: &mut Guard, cx| cx.request_close()))
                        .child(button("Keep editing", INK).on_click(|s: &mut Guard, _| s.confirming = false)),
                );
        }
        root
    }

    fn close_requested(&mut self, _cx: &mut Cx) -> bool {
        self.confirming = self.unsaved;
        !self.unsaved
    }

    fn background(&self) -> Color {
        PAPER
    }
}

fn main() {
    if let Err(e) = run_view_with(WindowOptions::new("gyeol close guard").size(420., 220.), Guard::default()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;

    #[test]
    fn closing_is_refused_while_unsaved_until_confirmed() {
        let mut h = TestHost::new(Guard::default(), (420., 220.));
        h.click_text("[ ] Unsaved changes");
        assert!(!h.request_close_for_test());
        assert!(h.state().confirming);
        h.click_text("Keep editing");
        assert!(!h.exited() && !h.state().confirming);
        h.request_close_for_test();
        h.click_text("Close anyway");
        assert!(h.exited());
    }

    #[test]
    fn a_clean_document_closes_at_once() {
        let mut h = TestHost::new(Guard::default(), (420., 220.));
        assert!(h.request_close_for_test());
    }
}
