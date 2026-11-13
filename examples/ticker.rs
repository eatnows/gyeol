//! `View::tick`: a counter that goes up 12 times a second, driven by asking for the next frame
//! from inside `tick`. Nothing redraws while idle except what `tick` schedules.
//!
//!     cargo run --example ticker
//!     cargo run --example ticker --features snapshot -- --snapshot out.png [seconds]   # headless
use std::time::{Duration, Instant};

use gyeol::{div, run_view, text, Color, Cx, Element, View};

const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 12);

struct Ticker {
    count: u64,
    /// When the next count is due; counting by deadlines keeps 12 per second even if frames come late.
    next: Option<Instant>,
}

impl View for Ticker {
    fn tick(&mut self, now: Instant, cx: &mut Cx) {
        let due = *self.next.get_or_insert(now + FRAME);
        if now >= due {
            self.count += 1;
            self.next = Some(due + FRAME);
        }
        let next = self.next.unwrap_or(now + FRAME);
        cx.request_redraw_after(next.saturating_duration_since(now));
    }

    fn view(&self, _cx: &mut Cx) -> Element<Ticker> {
        div()
            .p(32.)
            .gap(8.)
            .child(text(self.count.to_string()).text_size(64.).text_color(Color::hex(0x14202b)))
            .child(text("12 per second, advanced in View::tick").text_size(14.).text_color(Color::hex(0x4a5866)))
    }

    fn background(&self) -> Color {
        Color::hex(0xf3f5f7)
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let seconds: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.5);
            let options = gyeol::snapshot::Options { advance: Duration::from_secs_f32(seconds), ..Default::default() };
            let image = gyeol::snapshot::view_with(Ticker { count: 0, next: None }, (360., 160.), 2., &options).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    if let Err(e) = run_view("gyeol ticker", Ticker { count: 0, next: None }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;

    #[test]
    fn counts_twelve_per_second() {
        let mut h = TestHost::new(Ticker { count: 0, next: None }, (360., 160.));
        h.advance(Duration::from_secs(1));
        assert_eq!(h.state().count, 12);
        h.advance(Duration::from_secs(2));
        assert_eq!(h.state().count, 36);
        assert!(h.scene().texts().any(|t| t.content == "36"));
    }
}
