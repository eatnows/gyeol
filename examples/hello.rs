//! Phase 0: a window with rounded rectangles.
use gyeol::{Color, Cx, Quad, Rect, Scene, Text};

struct Hello;

impl gyeol::App for Hello {
    fn scene(&mut self, cx: &mut Cx) -> Scene {
        let (w, h) = cx.size();
        let mut scene = Scene { background: Some(Color::hex(0xf3f5f7)), ..Default::default() };
        scene.push_quad(Quad::new(Rect::new(40., 40., w - 80., 120.), Color::hex(0xffffff)).rounded(12.).bordered(1., Color::hex(0xdce2e8)));
        scene.push_quad(Quad::new(Rect::new(40., 190., 200., 64.), Color::hex(0xb45f06)).rounded(8.));
        scene.push_quad(Quad::new(Rect::new(260., 190., 64., 64.), Color::hex(0x14202b)).rounded(32.));
        scene.push_quad(Quad::new(Rect::new(344., 190., (w - 384.).max(40.), 64.), Color::hex(0x2e7d4e).with_alpha(0.35)).rounded(8.).bordered(2., Color::hex(0x2e7d4e)));
        scene.push_quad(Quad::new(Rect::new(40., h - 90., w - 80., 50.), Color::hex(0x0f1720)).rounded(6.));
        // A glass-like card: translucent gradient over a colorful backdrop.
        scene.push_quad(Quad::new(Rect::new(40., 290., 260., 100.), Color::hex(0x7aa7ff)).rounded(20.).gradient(150., Color::hex(0x7aa7ff), Color::hex(0xff9ecb)).shadowed(0., 12., 28., Color::hex(0x1b2a6b).with_alpha(0.35)));
        scene.push_quad(
            Quad::new(Rect::new(60., 310., 220., 60.), Color::hex(0xffffff).with_alpha(0.5))
                .rounded(14.)
                .gradient(140., Color::hex(0xffffff).with_alpha(0.55), Color::hex(0xffffff).with_alpha(0.15))
                .bordered(1., Color::hex(0xffffff).with_alpha(0.6))
                .inset_shadowed(0., 1., 0., Color::hex(0xffffff).with_alpha(0.6))
                .shadowed(0., 6., 16., Color::hex(0x1b2a6b).with_alpha(0.18)),
        );
        let ink = Color::hex(0x14202b);
        scene.push_text(Text::new((60., 58.), "gyeol 결", 34., ink));
        scene.push_text(Text::new((60., 108.), "한글 조합, Hello world, fn main() { 0123456789 }", 16., Color::hex(0x4a5866)));
        scene.push_text(Text::new((60., 206.), "Build", 22., Color::hex(0xffffff)));
        scene.push_text(Text::new((60., h - 78.), "cargo run --example hello   // 안녕하세요", 14., Color::hex(0xd5dee6)));
        scene
    }
}

fn main() {
    if let Err(e) = gyeol::run("gyeol hello", Hello) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
