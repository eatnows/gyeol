//! Phase 3: a list of 100,000 rows that scrolls smoothly because only the visible rows are built.
//! Scroll with the wheel or trackpad; click a row to select it.
use gyeol::{div, run_view, text, uniform_list, Color, Cx, Element, View};

const ROWS: usize = 100_000;
const ROW_H: f32 = 26.;

struct Files {
    selected: Option<usize>,
}

fn name(i: usize) -> String {
    let dirs = ["core", "ui", "editor", "git", "project", "text"];
    format!("crates/{}/src/module_{}/file_{i}.rs", dirs[i % dirs.len()], i / 50)
}

impl View for Files {
    fn view(&self, cx: &mut Cx) -> Element<Self> {
        let ink = Color::hex(0x1a1b1c);
        let dim = Color::hex(0x8a8b8c);
        let line = Color::hex(0xe4e4e2);
        let hot = Color::hex(0xeeeeec);
        let accent = Color::hex(0xfbeedc);

        let list = uniform_list(cx, "files", ROWS, ROW_H, |i| {
            let selected = self.selected == Some(i);
            div()
                .row()
                .items_center()
                .px(12.)
                .bg(if selected { accent } else { Color::TRANSPARENT })
                .hover_bg(if selected { accent } else { hot })
                .on_click(move |s: &mut Files, _| s.selected = Some(i))
                .child(text(name(i)).text_size(13.).text_color(ink))
        })
        .grow()
        .scrollbar(Color::hex(0x000000).with_alpha(0.28));

        let detail = match self.selected {
            Some(i) => div()
                .gap(6.)
                .child(text(name(i)).text_size(16.).text_color(ink))
                .child(text(format!("row {i} of {ROWS}")).text_size(13.).text_color(dim)),
            None => div().child(text("Select a file").text_size(14.).text_color(dim)),
        };

        div()
            .row()
            .child(div().w(360.).bg(Color::hex(0xfbfbfa)).child(list))
            .child(div().w(1.).bg(line))
            .child(div().grow().p(24.).child(detail))
    }

    fn background(&self) -> Color {
        Color::hex(0xffffff)
    }
}

fn main() {
    if let Err(e) = run_view("gyeol filelist", Files { selected: Some(3) }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
