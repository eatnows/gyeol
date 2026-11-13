//! Multi-line text: chat bubbles whose text wraps at the bubble's max width. Plain `text()` stays on
//! one line; `.wrap_text()` wraps at spaces (and between characters inside a word too wide for a line).
//!
//!     cargo run --example text_wrap
//!     cargo run --example text_wrap --features snapshot -- --snapshot out.png [scale]   # headless
use gyeol::{div, run_view_with, text, Align, Color, Cx, Element, View, WindowOptions};

struct Chat;

fn bubble(message: &'static str, mine: bool) -> Element<Chat> {
    let (fill, ink) = if mine { (Color::hex(0x2f6fed), Color::hex(0xffffff)) } else { (Color::hex(0xe9ebef), Color::hex(0x14202b)) };
    div()
        .align_self(if mine { Align::End } else { Align::Start })
        .max_w(260.)
        .px(12.)
        .py(8.)
        .rounded(14.)
        .bg(fill)
        .child(text(message).text_size(15.).text_color(ink).wrap_text())
}

impl View for Chat {
    fn view(&self, _cx: &mut Cx) -> Element<Chat> {
        div()
            .p(20.)
            .gap(8.)
            .child(bubble("Hi!", false))
            .child(bubble("Did you see the new layout engine? Long messages now wrap inside the bubble instead of running off the window.", false))
            .child(bubble("네, 줄바꿈이 잘 됩니다.\n두 번째 줄은 명시적인 줄바꿈입니다.", true))
            .child(bubble("가나다라마바사아자차카타파하가나다라마바사아자차카타파하가나다라마바사", true))
            .child(div().child(text("Without wrap_text this line is a single long line that overflows its parent.").text_size(13.).text_color(Color::hex(0x8a94a0))))
    }

    fn background(&self) -> Color {
        Color::hex(0xfbfaf7)
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let scale = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.);
            let image = gyeol::snapshot::view(Chat, (400., 420.), scale).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path}");
            return;
        }
    }
    if let Err(e) = run_view_with(WindowOptions::new("gyeol text wrap").size(400., 420.), Chat) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
