//! Renders a View headlessly to a PNG, no window needed:
//! `cargo run --example snapshot --features snapshot -- out.png [scale]`
use gyeol::{div, snapshot, text, Color, Cx, Element, View};

/// Stripes and shapes with a glass card (gradient, borders, shadows, backdrop blur) over them.
struct Demo;

impl View for Demo {
    fn view(&self, _cx: &mut Cx) -> Element<Self> {
        let white = Color::hex(0xffffff);
        let mut root = div().w_full().h_full().bg(Color::hex(0xffffff));
        for i in 0..40 {
            root = root.child(div().absolute().left(i as f32 * 24.).top(0.).w(12.).h(600.).bg(Color::hex(0x101820)));
        }
        for (i, c) in [0xe63946, 0x2a9d8f, 0xf4a261, 0x457b9d].into_iter().enumerate() {
            root = root.child(div().absolute().left(60. + i as f32 * 140.).top(70.).size(100.).rounded(50.).bg(Color::hex(c)));
        }
        let card = div()
            .absolute()
            .left(110.)
            .top(110.)
            .w(420.)
            .h(220.)
            .p(28.)
            .rounded(28.)
            .bg(white.with_alpha(0.22))
            .bg_gradient(140., white.with_alpha(0.5), white.with_alpha(0.1))
            .border(1., white.with_alpha(0.6))
            .inset_shadow(0., 1., 0., white.with_alpha(0.7))
            .shadow(0., 14., 34., Color::hex(0x101820).with_alpha(0.3))
            .backdrop_blur(24., 1.9)
            .child(text("Liquid glass").text_size(30.).text_color(Color::hex(0x101820)))
            .child(text("headless snapshot · 결 gyeol").text_size(16.).text_color(Color::hex(0x101820)));
        root.child(card)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "snapshot.png".into());
    let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.);
    let image = snapshot::view(Demo, (640., 420.), scale).expect("render the snapshot");
    image.save_png(&path).expect("save the png");
    println!("wrote {path} ({}x{})", image.width, image.height);
}
