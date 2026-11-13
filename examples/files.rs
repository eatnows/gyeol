//! Files: drop files on the window to list their paths, or use the buttons for the native dialogs
//! (macOS; elsewhere they return nothing): open a file, choose a file or a folder, choose a folder,
//! save.
//!
//!     cargo run --example files
use gyeol::{div, run_view_with, text, Color, Cx, Element, Event, View, WindowOptions};

const INK: Color = Color::hex(0x14202b);
const PAPER: Color = Color::hex(0xfbfaf7);
const FILTERS: &[(&str, &[&str])] = &[("Images", &["png", "jpg", "jpeg"]), ("Text", &["txt", "md"])];

#[derive(Default)]
struct Files {
    dropped: Vec<String>,
    hovering: bool,
    picked: Option<String>,
}

fn button(label: &str) -> Element<Files> {
    div().id(label.to_string()).px(14.).py(8.).rounded(8.).bg(INK).child(text(label.to_string()).text_size(14.).text_color(PAPER))
}

impl View for Files {
    fn view(&self, _cx: &mut Cx) -> Element<Files> {
        let mut list = div().gap(4.);
        for p in &self.dropped {
            list = list.child(text(p.clone()).text_size(13.).text_color(INK));
        }
        let hint = if self.hovering { "Release to drop" } else { "Drop files here" };
        div()
            .p(24.)
            .gap(14.)
            .bg(if self.hovering { Color::hex(0xdfe9f5) } else { PAPER })
            .child(text(hint).text_size(20.).text_color(INK))
            .child(
                div()
                    .gap(10.)
                    .child(button("Open...").on_click(|s: &mut Files, cx| {
                        s.picked = Some(format!("open: {:?}", cx.pick_file(FILTERS)));
                    }))
                    .child(button("File or folder...").on_click(|s: &mut Files, cx| {
                        s.picked = Some(format!("path: {:?}", cx.pick_path(FILTERS)));
                    }))
                    .child(button("Folder...").on_click(|s: &mut Files, cx| {
                        s.picked = Some(format!("folder: {:?}", cx.pick_folder()));
                    }))
                    .child(button("Save...").on_click(|s: &mut Files, cx| {
                        s.picked = Some(format!("save: {:?}", cx.pick_save_path("untitled.png", FILTERS)));
                    })),
            )
            .child(text(self.picked.clone().unwrap_or_else(|| "no dialog yet".into())).text_size(14.).text_color(INK))
            .child(list)
    }

    fn event(&mut self, event: &Event, _cx: &mut Cx) {
        match event {
            Event::FilesHovered(over) => self.hovering = *over,
            Event::FilesDropped(files) => self.dropped = files.iter().map(|p| p.display().to_string()).collect(),
            _ => {}
        }
    }

    fn background(&self) -> Color {
        PAPER
    }
}

fn main() {
    if let Err(e) = run_view_with(WindowOptions::new("gyeol files").size(760., 420.), Files::default()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;
    use std::path::PathBuf;

    #[test]
    fn dropped_files_arrive_together_and_hover_is_tracked() {
        let mut h = TestHost::new(Files::default(), (640., 420.));
        h.hover_files(true);
        assert!(h.state().hovering);
        h.drop_files(vec![PathBuf::from("/a/one.png"), PathBuf::from("/b/two.txt")]);
        assert!(!h.state().hovering);
        assert_eq!(h.state().dropped, ["/a/one.png", "/b/two.txt"]);
    }

    #[test]
    fn dialogs_return_the_preset_result_once() {
        let mut h = TestHost::new(Files::default(), (640., 420.));
        h.set_next_pick(Some(PathBuf::from("/x/y.png")));
        h.click_text("Open...");
        assert_eq!(h.state().picked.as_deref(), Some("open: Some(\"/x/y.png\")"));
        h.click_text("Save...");
        assert_eq!(h.state().picked.as_deref(), Some("save: None"), "the preset is consumed");
        assert_eq!(h.picks(), ["open Images:png,jpg,jpeg;Text:txt,md", "save untitled.png Images:png,jpg,jpeg;Text:txt,md"]);
    }

    #[test]
    fn the_path_and_folder_buttons_ask_for_a_path_or_a_folder() {
        let mut h = TestHost::new(Files::default(), (900., 420.));
        h.set_next_pick(Some(PathBuf::from("/some/dir")));
        h.click_text("File or folder...");
        assert_eq!(h.state().picked.as_deref(), Some("path: Some(\"/some/dir\")"));
        h.set_next_pick(Some(PathBuf::from("/other")));
        h.click_text("Folder...");
        assert_eq!(h.state().picked.as_deref(), Some("folder: Some(\"/other\")"));
        assert_eq!(h.picks(), ["path Images:png,jpg,jpeg;Text:txt,md", "folder"]);
    }
}
