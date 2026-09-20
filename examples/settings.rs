//! Phase 2: the declarative API. A settings dialog like Madi's, built from `div()` / `text()` with
//! flexbox layout, hover styles and click handlers that just change state.
use gyeol::{div, run_view, text, Color, Cx, Element, View};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    General,
    Editor,
}

#[derive(Clone, Copy, PartialEq)]
enum Theme {
    Light,
    Dark,
}

struct Settings {
    open: bool,
    tab: Tab,
    theme: Theme,
    font_size: u32,
}

/// The colors of one theme.
#[derive(Clone, Copy)]
struct Palette {
    page: Color,
    modal: Color,
    chrome: Color,
    panel: Color,
    border: Color,
    soft: Color,
    selected: Color,
    text: Color,
    strong: Color,
    dim: Color,
}

impl Theme {
    fn palette(self) -> Palette {
        match self {
            Theme::Light => Palette {
                page: Color::hex(0xe9ecef),
                modal: Color::hex(0xf7f7f6),
                chrome: Color::hex(0xffffff),
                panel: Color::hex(0xfbfbfa),
                border: Color::hex(0xe4e4e2),
                soft: Color::hex(0xececea),
                selected: Color::hex(0xeeeeec),
                text: Color::hex(0x3a3b3c),
                strong: Color::hex(0x1a1b1c),
                dim: Color::hex(0x8a8b8c),
            },
            Theme::Dark => Palette {
                page: Color::hex(0x0a0d10),
                modal: Color::hex(0x101112),
                chrome: Color::hex(0x151617),
                panel: Color::hex(0x131415),
                border: Color::hex(0x232425),
                soft: Color::hex(0x1c1d1e),
                selected: Color::hex(0x1d1e1f),
                text: Color::hex(0xc6c7c9),
                strong: Color::hex(0xe2e3e4),
                dim: Color::hex(0x8a8b8c),
            },
        }
    }
}

type El = Element<Settings>;

/// A 1px rule that stretches across its parent.
fn rule(p: Palette) -> El {
    div().bg(p.border).w(1.).min_h(1.).min_w(1.)
}

fn hrule(color: Color) -> El {
    div().h(1.).bg(color).w_full()
}

/// One setting: title and description on the left, the control on the right.
fn setting(p: Palette, title: &str, description: &str, control: El) -> El {
    div()
        .col()
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .gap(24.)
                .py(16.)
                .child(
                    div()
                        .gap(4.)
                        .child(text(title).text_size(14.).text_color(p.strong))
                        .child(text(description).text_size(12.).text_color(p.dim)),
                )
                .child(control),
        )
        .child(hrule(p.soft))
}

/// A bordered strip of cells: the one control shape for choices and steppers.
fn strip(p: Palette, cells: Vec<El>) -> El {
    let mut row = div().row().border(1., p.border).rounded(6.);
    for (i, cell) in cells.into_iter().enumerate() {
        if i > 0 {
            row = row.child(rule(p));
        }
        row = row.child(cell);
    }
    row
}

fn cell(label: String, color: Color) -> El {
    div().min_w(28.).h(26.).px(12.).items_center().justify_center().child(text(label).text_size(12.).text_color(color))
}

impl Settings {
    fn general(&self, p: Palette) -> El {
        let choice = |theme: Theme, label: &str| {
            let on = self.theme == theme;
            cell(label.to_string(), if on { p.strong } else { p.dim })
                .bg(if on { p.selected } else { Color::TRANSPARENT })
                .hover_bg(p.selected)
                .on_click(move |s: &mut Settings, _| s.theme = theme)
        };
        setting(p, "Theme", "Always use light or dark.", strip(p, vec![choice(Theme::Light, "Light"), choice(Theme::Dark, "Dark")]))
    }

    fn editor(&self, p: Palette) -> El {
        let size = self.font_size;
        let step = |label: &str, delta: i32, enabled: bool| {
            let cell = cell(label.to_string(), if enabled { p.strong } else { p.soft });
            if enabled {
                cell.hover_bg(p.selected).on_click(move |s: &mut Settings, _| s.font_size = (s.font_size as i32 + delta).clamp(10, 24) as u32)
            } else {
                cell
            }
        };
        let control = strip(p, vec![step("−", -1, size > 10), cell(size.to_string(), p.strong).w(44.), step("+", 1, size < 24)]);
        div()
            .child(setting(p, "Font size", "Size of the editor's text, in pixels.", control))
            .child(
                div()
                    .mt(16.)
                    .p(12.)
                    .rounded(6.)
                    .border(1., p.border)
                    .bg(p.panel)
                    .text_size(size as f32)
                    .text_color(p.text)
                    .child(text("fn main() {"))
                    .child(text("    println!(\"Hello, 결\");"))
                    .child(text("}")),
            )
    }

    fn nav_item(&self, p: Palette, tab: Tab, label: &str) -> El {
        let on = self.tab == tab;
        div()
            .row()
            .items_center()
            .h(28.)
            .px(12.)
            .rounded(6.)
            .bg(if on { p.selected } else { Color::TRANSPARENT })
            .hover_bg(p.selected)
            .on_click(move |s: &mut Settings, _| s.tab = tab)
            .child(text(label).text_size(12.).text_color(if on { p.strong } else { p.dim }))
    }

    fn modal(&self, p: Palette) -> El {
        let title = if self.tab == Tab::General { "General" } else { "Editor" };
        let content = if self.tab == Tab::General { self.general(p) } else { self.editor(p) };
        div()
            .w(760.)
            .h(460.)
            .rounded(10.)
            .border(1., p.border)
            .bg(p.modal)
            .text_color(p.text)
            .child(
                div()
                    .row()
                    .items_center()
                    .justify_between()
                    .h(40.)
                    .px(16.)
                    .bg(p.chrome)
                    .rounded_top(9.)
                    .child(text("Settings").text_size(14.).text_color(p.strong))
                    .child(
                        div()
                            .size(24.)
                            .items_center()
                            .justify_center()
                            .rounded(6.)
                            .hover_bg(p.selected)
                            .on_click(|s: &mut Settings, _| s.open = false)
                            .child(text("×").text_size(14.).text_color(p.dim)),
                    ),
            )
            .child(hrule(p.border))
            .child(
                div()
                    .row()
                    .grow()
                    .child(
                        div()
                            .w(176.)
                            .p(8.)
                            .gap(4.)
                            .bg(p.panel)
                            .child(self.nav_item(p, Tab::General, "General"))
                            .child(self.nav_item(p, Tab::Editor, "Editor")),
                    )
                    .child(rule(p))
                    .child(div().grow().px(24.).py(16.).child(text(title).text_size(15.).text_color(p.strong).mb(8.)).child(content)),
            )
    }
}

impl View for Settings {
    fn view(&self, _cx: &mut Cx) -> Element<Self> {
        let p = self.theme.palette();
        let root = div().items_center().justify_center().text_color(p.text);
        if self.open {
            root.child(self.modal(p))
        } else {
            root.child(
                div()
                    .px(16.)
                    .py(8.)
                    .rounded(8.)
                    .border(1., p.border)
                    .bg(p.chrome)
                    .hover_bg(p.selected)
                    .on_click(|s: &mut Settings, _| s.open = true)
                    .child(text("Open settings").text_size(13.).text_color(p.strong)),
            )
        }
    }

    fn background(&self) -> Color {
        self.theme.palette().page
    }
}

fn main() {
    let dark = std::env::args().any(|a| a == "--dark");
    let editor = std::env::args().any(|a| a == "--editor");
    let state = Settings {
        open: true,
        tab: if editor { Tab::Editor } else { Tab::General },
        theme: if dark { Theme::Dark } else { Theme::Light },
        font_size: 15,
    };
    if let Err(e) = run_view("gyeol settings", state) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
