//! A translucent glass panel over a high-contrast striped background, so the backdrop blur is easy to
//! check by eye. Run with `--sharp` to draw the same scene without the blur for comparison.
use gyeol::{Color, Cx, Quad, Rect, Scene, Text};

struct Blur {
    sharp: bool,
}

impl gyeol::App for Blur {
    fn scene(&mut self, cx: &mut Cx) -> Scene {
        let (w, h) = cx.size();
        let mut scene = Scene { background: Some(Color::hex(0xffffff)), ..Default::default() };
        // Hard black stripes, a few saturated shapes and some text: lots of edges to blur.
        let mut x = 0.;
        while x < w {
            scene.push_quad(Quad::new(Rect::new(x, 0., 12., h), Color::hex(0x101820)));
            x += 24.;
        }
        let colors = [0xe63946, 0x2a9d8f, 0xf4a261, 0x457b9d];
        for (i, c) in colors.into_iter().enumerate() {
            let x = 40. + i as f32 * 150.;
            scene.push_quad(Quad::new(Rect::new(x, 60., 110., 110.), Color::hex(c)).rounded(55.));
            scene.push_quad(Quad::new(Rect::new(x + 20., h - 170., 110., 110.), Color::hex(c)).rounded(10.));
        }
        scene.push_text(Text::new((40., 200.), "Sharp text behind the glass: 결 gyeol 0123456789", 28., Color::hex(0xffffff)));
        scene.push_text(Text::new((40., 250.), "Sharp text behind the glass: 결 gyeol 0123456789", 28., Color::hex(0x101820)));

        let panel = |bounds, radius| {
            let quad = Quad::new(bounds, Color::hex(0xffffff).with_alpha(0.22))
                .rounded(radius)
                .bordered(1., Color::hex(0xffffff).with_alpha(0.6))
                .inset_shadowed(0., 1., 0., Color::hex(0xffffff).with_alpha(0.6))
                .shadowed(0., 12., 30., Color::hex(0x101820).with_alpha(0.3));
            if self.sharp { quad } else { quad.backdrop_blurred(24., 1.9) }
        };
        scene.push_quad(panel(Rect::new(120., 110., 360., 220.), 28.));
        scene.push_text(Text::new((150., 140.), "Glass panel", 28., Color::hex(0x101820)));
        scene.push_text(Text::new((150., 186.), "blur 24, saturate 1.9", 16., Color::hex(0x101820)));
        // A small panel with a tight blur, and one clipped by a parent rectangle.
        scene.push_quad(panel(Rect::new(w - 220., 60., 160., 100.), 16.).backdrop_blurred(6., 1.));
        scene.push_quad(panel(Rect::new(w - 260., h - 200., 200., 140.), 20.).clipped(Some(Rect::new(w - 200., 0., 200., h))));
        scene
    }
}

fn main() {
    let sharp = std::env::args().any(|a| a == "--sharp");
    if let Err(e) = gyeol::run("gyeol backdrop blur", Blur { sharp }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
