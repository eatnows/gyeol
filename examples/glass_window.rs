//! A transparent window with a hidden titlebar, repositioned traffic lights and (on macOS) a blurred
//! backdrop, showing a gradient wallpaper and a rounded gradient card.
use gyeol::{Color, Cx, Quad, Rect, Scene, Text, WindowOptions};

struct Glass;

impl gyeol::App for Glass {
    fn scene(&mut self, cx: &mut Cx) -> Scene {
        let (w, h) = cx.size();
        let white = Color::hex(0xffffff);
        // No background: the wallpaper below is translucent, so the blurred desktop shows through it.
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., w, h), Color::hex(0x7aa7ff)).gradient(150., Color::hex(0x7aa7ff).with_alpha(0.55), Color::hex(0xff9ecb).with_alpha(0.55)));
        let card = Rect::new(60., 90., (w - 120.).max(200.), 180.);
        scene.push_quad(
            Quad::new(card, white.with_alpha(0.3))
                .rounded(28.)
                .gradient(140., white.with_alpha(0.55), white.with_alpha(0.12))
                .bordered(1., white.with_alpha(0.55))
                .inset_shadowed(0., 1., 0., white.with_alpha(0.6))
                .shadowed(0., 16., 40., Color::hex(0x1b2a6b).with_alpha(0.3)),
        );
        scene.push_text(Text::new((card.x + 28., card.y + 28.), "Liquid glass", 30., Color::hex(0x14202b)));
        scene.push_text(Text::new((card.x + 28., card.y + 78.), "transparent window, hidden titlebar, vibrancy", 15., Color::hex(0x33414f)));
        scene
    }
}

fn main() {
    let options = WindowOptions::new("glass").size(720., 480.).transparent(true).transparent_titlebar(true).traffic_lights(22., 22.).vibrancy(true);
    if let Err(e) = gyeol::run_with(options, Glass) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
