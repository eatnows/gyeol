//! A slider driven by `MouseEvent::size`, dividers that glow on hover (`Element::hover`), and faded
//! cards (`Element::opacity`).
//!
//! `cargo run --example interaction` opens a window (drag the slider, hover the dividers);
//! `cargo run --example interaction --features snapshot -- out.png [scale] [pointer-x pointer-y]` writes a
//! PNG, with the pointer placed where given so a hover state can be captured without a window.
use gyeol::{div, text, Color, Cx, Element, Gradient, MouseEvent, Shadow, View};

const INK: Color = Color::hex(0xe6e9ef);
const DIM: Color = Color::hex(0x8a93a6);
const ACCENT: Color = Color::hex(0xff8a1f);
const TRACK_W: f32 = 320.;

struct Interaction {
    value: f32,
}

impl Interaction {
    /// Sets the value to how far across the track the pointer is, from the event's own size.
    fn scrub(&mut self, e: MouseEvent) {
        self.value = (e.local.0 / e.size.0).clamp(0., 1.);
    }
}

fn section(title: &str) -> Element<Interaction> {
    div().col().gap(10.).child(text(title).text_size(12.).text_color(DIM))
}

fn divider(label: &str) -> Element<Interaction> {
    let glow = Gradient::linear(90.).stop(Color::TRANSPARENT).stop(ACCENT).stop(Color::TRANSPARENT);
    div().col().gap(6.).child(
        div()
            .w(TRACK_W)
            .h(2.)
            .bg(Color::hex(0x2a2f3a))
            .hover(|h| h.bg(Color::TRANSPARENT).bg_layers([glow]).shadows([Shadow::new(0., 0., 12., 0., ACCENT.with_alpha(0.7)), Shadow::new(0., 0., 32., 4., ACCENT.with_alpha(0.3))])),
    )
    .child(text(label).text_size(11.).text_color(DIM))
}

fn card(opacity: f32) -> Element<Interaction> {
    div()
        .w(120.)
        .h(70.)
        .p(10.)
        .rounded(14.)
        .col()
        .gap(4.)
        .bg(Color::hex(0x2c3242))
        .bg_layer(Gradient::linear(150.).stop(ACCENT.with_alpha(0.5)).stop(Color::TRANSPARENT))
        .border(1., Color::hex(0xffffff).with_alpha(0.4))
        .shadow_layer(Shadow::new(0., 8., 16., 0., Color::hex(0).with_alpha(0.5)))
        .opacity(opacity)
        .child(text(format!("opacity {opacity}")).text_size(12.).text_color(INK))
        .child(text("child text").text_size(11.).text_color(DIM))
}

impl View for Interaction {
    fn view(&self, _cx: &mut Cx) -> Element<Self> {
        let fill = self.value * TRACK_W;
        let slider = div()
            .id("track")
            .w(TRACK_W)
            .h(8.)
            .rounded(4.)
            .bg(Color::hex(0x2a2f3a))
            .on_mouse_down(|s: &mut Interaction, _, e| s.scrub(e))
            .on_drag(|s: &mut Interaction, _, e| s.scrub(e))
            .child(div().absolute().left(0.).top(0.).w(fill).h(8.).rounded(4.).bg(ACCENT))
            .child(div().absolute().left(fill - 8.).top(-4.).size(16.).rounded(8.).bg(INK).shadow_layer(Shadow::new(0., 2., 6., 0., Color::hex(0).with_alpha(0.5))));

        let chip = div()
            .px(14.)
            .py(8.)
            .rounded(999.)
            .bg(Color::hex(0x2a2f3a))
            .text_color(DIM)
            .hover(|h| h.bg(ACCENT.with_alpha(0.25)).text_color(INK).border_color(ACCENT))
            .border(1., Color::TRANSPARENT)
            .child(text("hover: text color + border").text_size(12.));

        div()
            .p(30.)
            .col()
            .gap(26.)
            .w_full()
            .h_full()
            .child(section(&format!("slider: local.0 / size.0 = {:.2}", self.value)).child(slider))
            .child(section("hover: replaces gradient layers and shadows").child(divider("divider one")).child(divider("divider two")).child(div().row().child(chip)))
            .child(section("opacity: fades background, gradient, border, shadow and text").child(div().row().gap(18.).child(card(1.)).child(card(0.7)).child(card(0.4)).child(card(0.15))))
    }

    fn background(&self) -> Color {
        Color::hex(0x14161c)
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Some(path) = args.first() {
            let number = |i: usize| args.get(i).and_then(|s| s.parse::<f32>().ok());
            let scale = number(1).unwrap_or(1.);
            let pointer = number(2).zip(number(3));
            let options = gyeol::snapshot::Options { pointer, ..Default::default() };
            let image = gyeol::snapshot::view_with(Interaction { value: 0.6 }, (600., 420.), scale, &options).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    if let Err(e) = gyeol::run_view("gyeol interaction", Interaction { value: 0.35 }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
