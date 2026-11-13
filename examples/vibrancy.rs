//! Compare window vibrancy looks: a transparent window over a macOS blur, with white panels at 6, 20
//! and 40% opacity and white text, to judge contrast.
//!
//!     GYEOL_MATERIAL=hud GYEOL_APPEARANCE=dark cargo run --example vibrancy
//!
//! `GYEOL_MATERIAL`: under-window (default), hud, sidebar, popover, menu, fullscreen, under-page, titlebar.
//! `GYEOL_APPEARANCE`: system (default), dark, light. Both are ignored outside macOS.
use gyeol::{div, run_view_with, text, Appearance, Color, Cx, Element, Material, View, WindowOptions};

struct Look {
    label: String,
}

fn material(name: &str) -> Option<Material> {
    Some(match name.to_ascii_lowercase().as_str() {
        "under-window" | "underwindow" | "default" => Material::UnderWindowBackground,
        "hud" => Material::HudWindow,
        "sidebar" => Material::Sidebar,
        "popover" => Material::Popover,
        "menu" => Material::Menu,
        "fullscreen" | "full-screen-ui" => Material::FullScreenUI,
        "under-page" | "underpage" => Material::UnderPageBackground,
        "titlebar" => Material::Titlebar,
        _ => return None,
    })
}

fn appearance(name: &str) -> Option<Appearance> {
    Some(match name.to_ascii_lowercase().as_str() {
        "system" => Appearance::System,
        "dark" => Appearance::Dark,
        "light" => Appearance::Light,
        _ => return None,
    })
}

impl View for Look {
    fn view(&self, _cx: &mut Cx) -> Element<Look> {
        let white = Color::hex(0xffffff);
        let panel = |percent: u32| {
            div()
                .w(190.)
                .h(130.)
                .p(14.)
                .rounded(18.)
                .bg(white.with_alpha(percent as f32 / 100.))
                .border(1., white.with_alpha(0.25))
                .child(text(format!("{percent}% white")).text_size(16.).text_color(white))
        };
        div()
            .p(40.)
            .gap(24.)
            .text_color(white)
            .child(text(self.label.clone()).text_size(22.).text_color(white))
            .child(text("White text on the blurred desktop").text_size(14.).text_color(white))
            .child(div().row().gap(16.).child(panel(6)).child(panel(20)).child(panel(40)))
    }

    fn background(&self) -> Color {
        Color::TRANSPARENT
    }
}

fn main() {
    let m = std::env::var("GYEOL_MATERIAL").ok();
    let a = std::env::var("GYEOL_APPEARANCE").ok();
    let material = m.as_deref().map_or(Material::default(), |n| material(n).unwrap_or_else(|| panic!("unknown GYEOL_MATERIAL {n:?}")));
    let appearance = a.as_deref().map_or(Appearance::default(), |n| self::appearance(n).unwrap_or_else(|| panic!("unknown GYEOL_APPEARANCE {n:?}")));
    let label = format!("material {material:?} / appearance {appearance:?}");
    let options = WindowOptions::new("gyeol vibrancy")
        .size(680., 320.)
        .transparent(true)
        .transparent_titlebar(true)
        .traffic_lights(22., 22.)
        .vibrancy(true)
        .vibrancy_material(material)
        .appearance(appearance);
    if let Err(e) = run_view_with(options, Look { label }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
