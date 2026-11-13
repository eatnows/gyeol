//! Gradients and shadows, CSS-style: multi-stop linear, radial and elliptical gradients, stacked layers,
//! color-to-transparent fades, several shadows, spread and inset shadows. Run it in a window, or write a
//! PNG without one: `cargo run --example gradients --features snapshot -- out.png [scale]`.
use gyeol::{div, text, Color, Cx, Element, Gradient, RadialSize, Shadow, View};

const INK: Color = Color::hex(0x14202b);

struct Gradients;

fn swatch(label: &str) -> Element<Gradients> {
    div().w(180.).h(110.).rounded(16.).p(10.).child(text(label).text_size(12.).text_color(Color::hex(0xffffff)))
}

impl View for Gradients {
    fn view(&self, _cx: &mut Cx) -> Element<Self> {
        let white = Color::hex(0xffffff);
        let rainbow = Gradient::linear(90.)
            .stop(Color::hex(0xe63946))
            .stop(Color::hex(0xf4a261))
            .stop(Color::hex(0xe9c46a))
            .stop(Color::hex(0x2a9d8f))
            .stop(Color::hex(0x457b9d));
        // A dark linear base with two radial glows over it: the common "glass wallpaper" stack.
        let base = Gradient::linear(160.).stop(Color::hex(0x1b1b3a)).stop(Color::hex(0x0d1b2a));
        let violet = Gradient::radial((0.15, 0.1), RadialSize::Ellipse(300., 200.)).stop(Color::hex(0x8b5cf6).with_alpha(0.7)).stop_at(Color::TRANSPARENT, 0.7);
        let teal = Gradient::radial((0.9, 0.9), RadialSize::FarthestCorner).stop(Color::hex(0x14b8a6).with_alpha(0.6)).stop_at(Color::TRANSPARENT, 0.6);

        let row1 = div()
            .row()
            .gap(16.)
            .child(swatch("3 stops, 90deg").bg_layer(Gradient::linear(90.).stop(Color::hex(0xff0000)).stop(Color::hex(0x00ff00)).stop(Color::hex(0x0000ff))))
            .child(swatch("5 stops").bg_layer(rainbow))
            .child(swatch("positions 0 / 20% / 100%").bg_layer(Gradient::linear(135.).stop(Color::hex(0x111111)).stop_at(Color::hex(0xff5500), 0.2).stop_at(Color::hex(0xffffff), 1.)))
            .child(swatch("radial closest-side").bg_layer(Gradient::radial((0.5, 0.5), RadialSize::ClosestSide).stop(white).stop(Color::hex(0x6d28d9))));

        let checker = div().w(180.).h(110.).rounded(16.).p(10.).bg(Color::hex(0xdddddd)); // a flat color for comparison
        let row2 = div()
            .row()
            .gap(16.)
            .child(swatch("ellipse 90px 30px").bg(Color::hex(0x111827)).bg_layer(Gradient::radial((0.5, 0.5), RadialSize::Ellipse(90., 30.)).stop(Color::hex(0xfbbf24)).stop(Color::hex(0x111827))))
            .child(swatch("red -> transparent").bg(Color::hex(0x2563eb)).bg_layer(Gradient::linear(90.).stop(Color::hex(0xff0000)).stop(Color::TRANSPARENT)))
            .child(
                div().w(180.).h(110.).rounded(16.).p(10.).bg(Color::hex(0x111827))
                    .bg_layers([base, violet, teal])
                    .child(text("linear + 2 glows").text_size(12.).text_color(white)),
            )
            .child(checker.child(text("flat").text_size(12.).text_color(INK)));

        let card = |label: &str| div().w(180.).h(90.).rounded(18.).bg(Color::hex(0xffffff)).p(10.).child(text(label).text_size(12.).text_color(INK));
        let row3 = div()
            .row()
            .gap(30.)
            .child(card("two outer shadows").shadows([
                Shadow::new(0., 2., 4., 0., Color::hex(0).with_alpha(0.25)),
                Shadow::new(0., 18., 30., 0., Color::hex(0x4f46e5).with_alpha(0.35)),
            ]))
            .child(card("spread 8, blur 0").shadow_layer(Shadow::new(0., 0., 0., 8., Color::hex(0x4f46e5).with_alpha(0.35))))
            .child(card("glow: spread -6, blur 24").shadow_layer(Shadow::new(0., 20., 24., -6., Color::hex(0xe11d48).with_alpha(0.55))))
            .child(
                card("inset top + bottom, outer")
                    .bg(Color::hex(0xe2e8f0))
                    .inset_shadows([
                        Shadow::new(0., 3., 0., 0., white),
                        Shadow::new(0., -3., 6., 0., Color::hex(0x64748b).with_alpha(0.5)),
                    ])
                    .shadow_layer(Shadow::new(0., 8., 16., 0., Color::hex(0).with_alpha(0.2))),
            );

        div().w_full().h_full().p(30.).col().gap(28.).bg(Color::hex(0xf1f5f9)).child(row1).child(row2).child(row3)
    }

    fn background(&self) -> Color {
        Color::hex(0xf1f5f9)
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Some(path) = args.first() {
            let scale = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.);
            let image = gyeol::snapshot::view(Gradients, (900., 520.), scale).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    if let Err(e) = gyeol::run_view("gyeol gradients", Gradients) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
