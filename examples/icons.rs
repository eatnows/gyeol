//! Vector icons from inline SVG (Lucide-style: 24x24, stroke=currentColor, round caps and joins), at
//! 14/16/20/24 px, tinted white on dark and with an accent color on light. Plus a filled glyph.
//!
//! `cargo run --example icons --features svg` opens a window;
//! `cargo run --example icons --features svg,snapshot -- out.png [scale]` writes a PNG instead.
use gyeol::{div, icon, text, Color, Cx, Element, View};

/// Lucide-style icons: the shapes inside the shared `<svg>` wrapper.
const ICONS: &[(&str, &str)] = &[
    ("arrow-right", r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#),
    ("arrow-left", r#"<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>"#),
    ("arrow-up", r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#),
    ("arrow-down", r#"<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>"#),
    ("chevron-right", r#"<path d="m9 18 6-6-6-6"/>"#),
    ("chevron-down", r#"<path d="m6 9 6 6 6-6"/>"#),
    ("play", r#"<polygon points="6 3 20 12 6 21 6 3"/>"#),
    ("pause", r#"<rect x="14" y="4" width="4" height="16" rx="1"/><rect x="6" y="4" width="4" height="16" rx="1"/>"#),
    ("skip-back", r#"<polygon points="19 20 9 12 19 4 19 20"/><line x1="5" x2="5" y1="19" y2="5"/>"#),
    ("skip-forward", r#"<polygon points="5 4 15 12 5 20 5 4"/><line x1="19" x2="19" y1="5" y2="19"/>"#),
    ("eye", r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"#),
    ("lock", r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#),
    ("pen", r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#),
    ("eraser", r#"<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>"#),
    ("square-dashed", r#"<path d="M5 3a2 2 0 0 0-2 2"/><path d="M19 3a2 2 0 0 1 2 2"/><path d="M21 19a2 2 0 0 1-2 2"/><path d="M5 21a2 2 0 0 1-2-2"/><path d="M9 3h1"/><path d="M9 21h1"/><path d="M14 3h1"/><path d="M14 21h1"/><path d="M3 9v1"/><path d="M21 9v1"/><path d="M3 14v1"/><path d="M21 14v1"/>"#),
    ("lasso", r#"<path d="M7 22a5 5 0 0 1-2-4"/><path d="M3.3 14A6.8 6.8 0 0 1 2 10c0-4.4 4.5-8 10-8s10 3.6 10 8-4.5 8-10 8a12 12 0 0 1-5-1"/><path d="M5 18a2 2 0 1 0 0-4 2 2 0 0 0 0 4"/>"#),
    ("hand", r#"<path d="M18 11V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2"/><path d="M14 10V4a2 2 0 0 0-2-2a2 2 0 0 0-2 2v2"/><path d="M10 10.5V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2v8"/><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15"/>"#),
    ("search", r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#),
    ("type", r#"<polyline points="4 7 4 4 20 4 20 7"/><line x1="9" x2="15" y1="20" y2="20"/><line x1="12" x2="12" y1="4" y2="20"/>"#),
    ("folder", r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#),
    ("layers", r#"<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>"#),
    ("undo", r#"<path d="M3 7v6h6"/><path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13"/>"#),
    ("redo", r#"<path d="M21 7v6h-6"/><path d="M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3l3 2.7"/>"#),
    ("send", r#"<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/>"#),
    ("sparkles", r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/><path d="M4 17v2"/><path d="M5 18H3"/>"#),
    ("maximize", r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#),
];

fn lucide(shapes: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{shapes}</svg>"#
    )
}

/// A filled glyph (Material style: `fill` defaults to the tint, with an evenodd-free single path).
const CHECK_CIRCLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"/></svg>"#;

struct Icons;

fn panel(bg: u32, tint: Color, label: &str) -> Element<Icons> {
    let mut tiles = div().row().wrap().gap(10.);
    for (name, shapes) in ICONS {
        let svg = lucide(shapes);
        let mut sizes = div().row().gap(12.).items_center();
        for size in [14., 16., 20., 24.] {
            sizes = sizes.child(icon(&svg).size(size));
        }
        tiles = tiles.child(div().w(150.).col().gap(4.).child(sizes).child(text(*name).text_size(10.).text_color(tint.with_alpha(0.6))));
    }
    let glyphs = div().row().gap(12.).items_center().child(text("filled").text_size(10.).text_color(tint.with_alpha(0.6))).children([14., 20., 24., 32.].map(|s| icon(CHECK_CIRCLE).size(s)));
    div().w(680.).p(20.).col().gap(14.).rounded(16.).bg(Color::hex(bg)).text_color(tint).child(text(label).text_size(13.)).child(tiles).child(glyphs)
}

impl View for Icons {
    fn view(&self, _cx: &mut Cx) -> Element<Self> {
        div()
            .row()
            .gap(20.)
            .p(20.)
            .w_full()
            .h_full()
            .child(panel(0x14202b, Color::hex(0xffffff), "white on dark"))
            .child(panel(0xf4f6f8, Color::hex(0xc2410c), "accent on light"))
    }

    fn background(&self) -> Color {
        Color::hex(0x2b2f36)
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Some(path) = args.first() {
            let scale = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.);
            let image = gyeol::snapshot::view(Icons, (1440., 520.), scale).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    if let Err(e) = gyeol::run_view("gyeol icons", Icons) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
