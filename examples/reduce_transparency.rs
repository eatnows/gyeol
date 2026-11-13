//! Follows the system's "Reduce transparency" option: translucent glass normally, a solid panel when
//! the user turns the option on (System Settings > Accessibility > Display), updating live.
//!
//!     cargo run --example reduce_transparency
//!     cargo run --example reduce_transparency --features snapshot -- --snapshot out.png [reduce]   # headless
use gyeol::{div, run_view_with, text, Color, Cx, Element, Event, Material, View, WindowOptions};

struct Demo;

impl View for Demo {
    fn view(&self, cx: &mut Cx) -> Element<Demo> {
        let white = Color::hex(0xffffff);
        let reduced = cx.reduce_transparency();
        // Reduced: an opaque panel with solid text colors. Normal: see-through glass over the blur.
        let (panel, note) = if reduced { (Color::hex(0x2b3036), "Reduce transparency is ON: solid panel") } else { (white.with_alpha(0.14), "Reduce transparency is off: glass") };
        div()
            .p(40.)
            .child(
                div()
                    .p(24.)
                    .gap(10.)
                    .rounded(20.)
                    .bg(panel)
                    .border(1., white.with_alpha(if reduced { 0.12 } else { 0.3 }))
                    .child(text("Display options").text_size(22.).text_color(white))
                    .child(text(note).text_size(15.).text_color(white)),
            )
    }

    fn event(&mut self, event: &Event, _cx: &mut Cx) {
        if matches!(event, Event::DisplayOptionsChanged) {
            eprintln!("display options changed");
        }
    }

    fn background(&self) -> Color {
        Color::TRANSPARENT
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let options = gyeol::snapshot::Options {
                reduce_transparency: args.get(2).is_some_and(|a| a == "reduce"),
                background: gyeol::snapshot::Background::Color(Color::hex(0x6b8fb8)),
                ..Default::default()
            };
            let image = gyeol::snapshot::view_with(Demo, (520., 220.), 2., &options).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path}");
            return;
        }
    }
    let options = WindowOptions::new("gyeol reduce transparency")
        .size(520., 220.)
        .transparent(true)
        .transparent_titlebar(true)
        .traffic_lights(22., 22.)
        .vibrancy(true)
        .vibrancy_material(Material::UnderPageBackground)
        .appearance(gyeol::Appearance::Dark);
    if let Err(e) = run_view_with(options, Demo) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;

    #[test]
    fn the_panel_turns_solid_when_the_option_turns_on() {
        let mut h = TestHost::new(Demo, (520., 220.));
        let alpha = |h: &TestHost<Demo>| h.scene().quads().find(|q| q.border_width > 0.).map(|q| q.background.a).unwrap();
        assert!(alpha(&h) < 0.5, "glass at first");
        h.set_reduce_transparency(true);
        assert_eq!(alpha(&h), 1., "solid once reduced");
        h.set_reduce_transparency(false);
        assert!(alpha(&h) < 0.5);
    }
}
