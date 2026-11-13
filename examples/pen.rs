//! Pen pressure: drag to draw dots whose size follows the pressure (a pen on macOS; a mouse draws a
//! constant size). The readout shows the pointer kind, pressure and tilt of the latest sample. The
//! eraser end of a stylus paints in the paper color.
//!
//!     cargo run --example pen
//!     cargo run --example pen --features snapshot -- --snapshot out.png [scale]   # headless, synthetic stroke
use gyeol::{div, run_view_with, text, Color, Cx, Element, Event, PointerKind, PointerSample, View, WindowOptions};

const PAPER: Color = Color::hex(0xfbfaf7);
const INK: Color = Color::hex(0x14202b);
const DIM: Color = Color::hex(0x4a5866);
const MIN_DOT: f32 = 1.5;
const MAX_DOT: f32 = 22.;

#[derive(Clone, Copy)]
struct Dot {
    pos: (f32, f32),
    radius: f32,
    eraser: bool,
}

#[derive(Default)]
struct Pen {
    dots: Vec<Dot>,
    last: Option<Dot>,
    latest: Option<PointerSample>,
    samples_in_event: usize,
    down: bool,
}

impl Pen {
    /// Adds a dot for `sample` (canvas coordinates), filling the gap to the previous one so a fast
    /// stroke stays continuous.
    fn add(&mut self, pos: (f32, f32), pressure: f32, kind: PointerKind) {
        let radius = (MIN_DOT + pressure * (MAX_DOT - MIN_DOT)) / 2.;
        let dot = Dot { pos, radius, eraser: kind == PointerKind::Eraser };
        if let Some(prev) = self.last.filter(|p| p.eraser == dot.eraser) {
            let (dx, dy) = (pos.0 - prev.pos.0, pos.1 - prev.pos.1);
            let steps = (dx.hypot(dy) / (radius.min(prev.radius).max(1.) * 0.6)).floor() as usize;
            for i in 1..steps {
                let t = i as f32 / steps as f32;
                self.dots.push(Dot { pos: (prev.pos.0 + dx * t, prev.pos.1 + dy * t), radius: prev.radius + (radius - prev.radius) * t, eraser: dot.eraser });
            }
        }
        self.dots.push(dot);
        self.last = Some(dot);
    }
}

impl View for Pen {
    fn view(&self, _cx: &mut Cx) -> Element<Pen> {
        // An id keeps the drag attached to the canvas while dots are added (captures without one are
        // found again by position, which a changing child count invalidates).
        let mut canvas = div()
            .id("canvas")
            .w_full()
            .h_full()
            .overflow_hidden()
            .cursor(gyeol::Cursor::Grab)
            .on_mouse_down(|s: &mut Pen, cx, e| {
                s.down = true;
                s.last = None;
                s.add(e.local, e.pressure, e.pointer);
                s.latest = cx.pointer_samples().last().copied().or(Some(sample_of(&e)));
            })
            .on_drag(|s: &mut Pen, cx, e| {
                // Every reading since the last event, not just the final position.
                let origin = (e.pos.0 - e.local.0, e.pos.1 - e.local.1);
                s.samples_in_event = cx.pointer_samples().len();
                if cx.pointer_samples().is_empty() {
                    s.add(e.local, e.pressure, e.pointer);
                } else {
                    for p in cx.pointer_samples().to_vec() {
                        s.add((p.pos.0 - origin.0, p.pos.1 - origin.1), p.pressure, p.kind);
                    }
                }
                s.latest = cx.pointer_samples().last().copied().or(Some(sample_of(&e)));
            })
            .on_mouse_up(|s: &mut Pen, _, _| {
                s.down = false;
                s.last = None;
            });
        for d in &self.dots {
            canvas = canvas.child(
                div()
                    .absolute()
                    .left(d.pos.0 - d.radius)
                    .top(d.pos.1 - d.radius)
                    .size(d.radius * 2.)
                    .rounded(d.radius)
                    .bg(if d.eraser { PAPER } else { INK }),
            );
        }
        let readout = match self.latest {
            Some(s) => format!(
                "{:?}   pressure {:.2}   tilt {}   samples/event {}",
                s.kind,
                s.pressure,
                s.tilt.map_or("-".to_string(), |(x, y)| format!("{x:+.2}, {y:+.2}")),
                self.samples_in_event
            ),
            None => "draw here with a pen or the mouse".to_string(),
        };
        canvas.child(div().absolute().left(16.).top(12.).child(text(readout).text_size(14.).text_color(DIM)))
    }

    fn event(&mut self, event: &Event, cx: &mut Cx) {
        // Hovering reports the pen too (pressure 0), so the readout follows it before it touches down.
        if let (Event::MouseMoved { .. }, false) = (event, self.down) {
            self.latest = cx.pointer_samples().last().copied();
        }
    }

    fn background(&self) -> Color {
        PAPER
    }
}

fn sample_of(e: &gyeol::MouseEvent) -> PointerSample {
    PointerSample { pos: e.pos, pressure: e.pressure, tilt: e.tilt, rotation: None, kind: e.pointer, time: 0. }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let scale = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.);
            let image = gyeol::snapshot::view(synthetic_stroke(), (520., 260.), scale).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    let options = WindowOptions::new("gyeol pen").size(900., 600.).raw_pointer_samples(true);
    if let Err(e) = run_view_with(options, Pen::default()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// A made-up stroke whose pressure swells and fades, then an eraser sweep through it.
#[cfg(feature = "snapshot")]
fn synthetic_stroke() -> Pen {
    let mut pen = Pen::default();
    for i in 0..=120 {
        let t = i as f32 / 120.;
        pen.add((40. + t * 440., 130. + (t * std::f32::consts::TAU).sin() * 50.), (t * std::f32::consts::PI).sin(), PointerKind::Pen);
    }
    pen.last = None;
    for i in 0..=20 {
        pen.add((260. + i as f32 * 1.2, 70. + i as f32 * 6.), 0.9, PointerKind::Eraser);
    }
    pen.latest = Some(PointerSample::pen((480., 130.), 0.).tilt(0.35, -0.2));
    pen.samples_in_event = 6;
    pen
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::testing::TestHost;

    #[test]
    fn dot_size_follows_pressure_and_every_sample_counts() {
        let mut h = TestHost::new(Pen::default(), (400., 300.));
        h.pen_down(PointerSample::pen((50., 50.), 0.1));
        let burst: Vec<_> = (1..=4).map(|i| PointerSample::pen((50. + i as f32 * 10., 50.), 0.1 + i as f32 * 0.2)).collect();
        h.pen_move(burst);
        h.pen_up((90., 50.));
        let dots = &h.state().dots;
        assert!(dots.len() >= 5, "one dot per sample at least: {}", dots.len());
        let first = dots.first().unwrap().radius;
        let last = dots.last().unwrap().radius;
        assert!(last > first * 3., "pressure 0.9 draws a much bigger dot than 0.1: {first} vs {last}");
        assert_eq!(h.state().samples_in_event, 4);
        assert_eq!(h.state().latest.map(|s| s.kind), Some(PointerKind::Pen));
    }

    #[test]
    fn the_eraser_paints_paper_and_a_mouse_still_draws() {
        let mut h = TestHost::new(Pen::default(), (400., 300.));
        h.pen_down(PointerSample::pen((20., 20.), 0.5).eraser());
        h.pen_up((20., 20.));
        assert!(h.state().dots.iter().all(|d| d.eraser));
        let mut m = TestHost::new(Pen::default(), (400., 300.));
        m.mouse_down((20., 20.));
        m.mouse_move((60., 20.));
        m.mouse_up((60., 20.));
        assert!(!m.state().dots.is_empty() && m.state().dots.iter().all(|d| !d.eraser && d.radius > 5.), "a mouse is pressure 1: big dots");
    }
}
