//! A tiny paint program on `image_surface`: draw into a 64x64 pixel buffer (pen pressure sets the
//! brush size; a mouse draws a steady one), zoom with the wheel around the cursor, pan with the right
//! (or middle) button, and watch only the touched rectangle being uploaded.
//!
//!     cargo run --example image_surface
//!     cargo run --example image_surface --features snapshot -- --snapshot out.png [scale]   # headless
//!
//! Keys: 0 fit the window, 1 actual size (1:1 pixels), E toggle erasing, C clear.
use gyeol::{
    div, image_surface, run_view_with, text, Color, Cx, Element, Event, ImageBuffer, ImageFilter, ImageView, Key, MouseButton, PixelRect,
    PointerKind, ScrollDelta, View, WindowOptions,
};

const SIZE: u32 = 64;
const INK: [u8; 4] = [20, 32, 43, 255]; // premultiplied (opaque)

struct Paint {
    canvas: ImageBuffer,
    view: ImageView,
    /// The last brush position in image pixels, to join dots into a line.
    last: Option<(f32, f32, f32)>,
    /// Where the pan started, in element coordinates.
    pan_from: Option<(f32, f32)>,
    erase: bool,
    hover: Option<(u32, u32)>,
    size: (f32, f32),
}

impl Paint {
    fn new() -> Paint {
        Paint {
            canvas: ImageBuffer::new(SIZE, SIZE),
            view: ImageView::new().checkerboard(Color::hex(0xffffff), Color::hex(0xe4e4e4), 10.).filter(ImageFilter::Auto).zoom(8.).pan(40., 60.),
            last: None,
            pan_from: None,
            erase: false,
            hover: None,
            size: (900., 600.),
        }
    }

    /// Paints a soft round dot (or erases with one) at image position `(x, y)` with `radius` pixels, and
    /// fills the gap from the previous dot. Returns the rectangle that changed.
    fn brush(&mut self, x: f32, y: f32, radius: f32, erase: bool) -> PixelRect {
        let mut dirty = PixelRect::default();
        let from = self.last.unwrap_or((x, y, radius));
        let steps = ((x - from.0).hypot(y - from.1) / (radius.min(from.2).max(0.5) * 0.5)).ceil().max(1.) as u32;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let (px, py, pr) = (from.0 + (x - from.0) * t, from.1 + (y - from.1) * t, from.2 + (radius - from.2) * t);
            dirty = dirty.union(stamp(&self.canvas, px, py, pr, erase));
        }
        self.last = Some((x, y, radius));
        dirty
    }
}

/// One dot with a one-pixel soft edge, composited over (or erased from) the buffer.
fn stamp(canvas: &ImageBuffer, x: f32, y: f32, radius: f32, erase: bool) -> PixelRect {
    let r = radius.max(0.5);
    let (x0, y0) = ((x - r - 1.).floor().max(0.) as u32, (y - r - 1.).floor().max(0.) as u32);
    let (x1, y1) = ((x + r + 1.).ceil().max(0.) as u32, (y + r + 1.).ceil().max(0.) as u32);
    let rect = PixelRect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
    canvas.edit(Some(rect), |pixels, width| {
        let height = pixels.len() as u32 / 4 / width;
        for py in y0..y1.min(height) {
            for px in x0..x1.min(width) {
                let d = ((px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y) - r).clamp(-0.5, 0.5);
                let coverage = 0.5 - d; // 1 inside, 0 outside, soft across one pixel
                if coverage <= 0. {
                    continue;
                }
                let i = ((py * width + px) * 4) as usize;
                let dst = &mut pixels[i..i + 4];
                for c in 0..4 {
                    // Premultiplied "over": src*coverage + dst*(1 - src_alpha*coverage); erasing keeps dst*(1 - coverage).
                    let src = if erase { 0. } else { INK[c] as f32 * coverage };
                    let src_alpha = if erase { coverage } else { INK[3] as f32 / 255. * coverage };
                    dst[c] = (src + dst[c] as f32 * (1. - src_alpha)).round().clamp(0., 255.) as u8;
                }
            }
        }
    });
    rect
}

impl View for Paint {
    fn view(&self, cx: &mut Cx) -> Element<Paint> {
        let (w, h) = cx.size();
        let readout = format!(
            "zoom {:.2}x   {}   {}   revision {}",
            self.view.zoom,
            self.hover.map_or("-".to_string(), |(x, y)| format!("pixel {x}, {y}")),
            if self.erase { "erasing" } else { "drawing" },
            self.canvas.revision()
        );
        let _ = (w, h);
        div()
            .w_full()
            .h_full()
            .bg(Color::hex(0x3a3f45))
            .child(
                image_surface(&self.canvas, self.view)
                    .id("canvas")
                    .w_full()
                    .h_full()
                    .on_mouse_down(|s: &mut Paint, cx, e| {
                        s.last = None;
                        if e.button == MouseButton::Left {
                            s.draw_at(e.local, e.pressure, e.pointer == PointerKind::Eraser, cx.pointer_samples().len());
                        } else {
                            s.pan_from = Some(e.local);
                        }
                    })
                    .on_drag(|s: &mut Paint, cx, e| {
                        s.hover = s.view.pixel_at(e.local, SIZE, SIZE);
                        if e.button == MouseButton::Left {
                            let origin = (e.pos.0 - e.local.0, e.pos.1 - e.local.1);
                            // Every sample since the last event: a fast stroke stays smooth.
                            let samples: Vec<_> = cx.pointer_samples().to_vec();
                            if samples.is_empty() {
                                s.draw_at(e.local, e.pressure, e.pointer == PointerKind::Eraser, 0);
                            }
                            for p in samples {
                                s.draw_at((p.pos.0 - origin.0, p.pos.1 - origin.1), p.pressure, p.kind == PointerKind::Eraser, 1);
                            }
                        } else if let Some(from) = s.pan_from {
                            s.view.pan = (s.view.pan.0 + e.local.0 - from.0, s.view.pan.1 + e.local.1 - from.1);
                            s.pan_from = Some(e.local);
                        }
                    })
                    .on_mouse_up(|s: &mut Paint, _, _| {
                        s.last = None;
                        s.pan_from = None;
                    }),
            )
            .child(div().absolute().left(16.).top(12.).child(text(readout).text_size(14.).text_color(Color::hex(0xe8eaed))))
    }

    fn event(&mut self, event: &Event, cx: &mut Cx) {
        self.size = cx.size();
        match event {
            Event::MouseMoved { pos } => self.hover = self.view.pixel_at(*pos, SIZE, SIZE),
            Event::Scroll { delta, pos } => {
                let dy = match *delta {
                    ScrollDelta::Lines(_, y) => y * 0.15,
                    ScrollDelta::Pixels(_, y) => y * 0.01,
                };
                self.view = self.view.zoom_at(*pos, dy.exp(), 0.5, 64.);
            }
            Event::KeyDown { key: Key::Char(c), .. } => match c.to_ascii_lowercase().as_str() {
                "0" => self.view = self.view.fit((SIZE, SIZE), self.size, 40.),
                "1" => self.view = self.view.zoom(1.).pan((self.size.0 - SIZE as f32) / 2., (self.size.1 - SIZE as f32) / 2.),
                "e" => self.erase = !self.erase,
                "c" => self.canvas.edit(None, |p, _| p.fill(0)),
                _ => {}
            },
            _ => {}
        }
    }
}

impl Paint {
    /// Draws with the pointer at element position `local`; the brush grows with pressure.
    fn draw_at(&mut self, local: (f32, f32), pressure: f32, eraser: bool, _samples: usize) {
        let (u, v) = self.view.local_to_image(local);
        let radius = 0.8 + pressure * 3.2;
        self.brush(u, v, radius, eraser || self.erase);
    }
}

fn main() {
    #[cfg(feature = "snapshot")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let (Some("--snapshot"), Some(path)) = (args.first().map(String::as_str), args.get(1)) {
            let scale = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.);
            let image = gyeol::snapshot::view(sample(), (520., 480.), scale).expect("render the snapshot");
            image.save_png(path).expect("save the png");
            println!("wrote {path} ({}x{})", image.width, image.height);
            return;
        }
    }
    let options = WindowOptions::new("gyeol image_surface").size(900., 600.).raw_pointer_samples(true);
    if let Err(e) = run_view_with(options, Paint::new()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// A scribble with varying brush size, partly erased, shown 7x with hard pixels.
#[cfg(feature = "snapshot")]
fn sample() -> Paint {
    let mut p = Paint::new();
    p.view = ImageView::new().checkerboard(Color::hex(0xffffff), Color::hex(0xe4e4e4), 10.).zoom(7.).pan(36., 40.);
    for i in 0..=60 {
        let t = i as f32 / 60.;
        p.brush(6. + t * 52., 32. + (t * std::f32::consts::TAU).sin() * 16., 0.8 + (t * std::f32::consts::PI).sin() * 3.2, false);
    }
    p.last = None;
    for i in 0..=10 {
        p.brush(30. + i as f32 * 0.5, 8. + i as f32 * 4.5, 2.5, true);
    }
    p.last = None;
    p.hover = Some((33, 32));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use gyeol::{testing::TestHost, PointerSample};

    fn alpha(p: &Paint, x: u32, y: u32) -> u8 {
        p.canvas.read(|px, w, _| px[((y * w + x) * 4 + 3) as usize])
    }

    #[test]
    fn a_stroke_paints_pixels_and_reports_only_its_dirty_rectangle() {
        let mut p = Paint::new();
        let before = p.canvas.revision();
        let dirty = p.brush(20.3, 20., 3., false);
        assert!(p.canvas.revision() > before);
        assert_eq!(alpha(&p, 20, 20), 255, "the middle of the dot is solid");
        assert_eq!(alpha(&p, 40, 40), 0, "far away is untouched");
        assert!(dirty.w < 12 && dirty.h < 12, "{dirty:?}");
        let edge = alpha(&p, 23, 20); // its centre is 3.2 from the dot's: a third covered
        assert!(edge > 0 && edge < 255, "the edge is soft: {edge}");
        p.last = None;
        p.brush(20.3, 20., 3., true);
        assert_eq!(alpha(&p, 20, 20), 0, "erasing clears it again");
    }

    #[test]
    fn wheel_zoom_keeps_the_pixel_under_the_cursor_and_the_pen_draws_at_the_right_pixel() {
        let mut h = TestHost::new(Paint::new(), (900., 600.));
        let at = (200., 150.);
        let before = h.state().view.local_to_image(at);
        h.scroll(at, ScrollDelta::Lines(0., 3.));
        let v = h.state().view;
        assert!(v.zoom > 8.);
        let after = v.local_to_image(at);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        // Pen at the middle of pixel (10, 12) with full pressure paints there.
        let p = h.state().view.image_to_local((10.5, 12.5));
        h.pen_down(PointerSample::pen(p, 1.));
        h.pen_up(p);
        assert_eq!(alpha(h.state(), 10, 12), 255);
        assert!(h.state().view.pixel_at(p, SIZE, SIZE) == Some((10, 12)));
    }

    #[test]
    fn pressure_sets_the_brush_size() {
        let mut light = Paint::new();
        light.draw_at(light.view.image_to_local((30., 30.)), 0.1, false, 0);
        let mut hard = Paint::new();
        hard.draw_at(hard.view.image_to_local((30., 30.)), 1., false, 0);
        let area = |p: &Paint| p.canvas.read(|px, _, _| px.chunks_exact(4).filter(|c| c[3] > 128).count());
        assert!(area(&hard) > area(&light) * 4, "{} vs {}", area(&hard), area(&light));
    }
}
