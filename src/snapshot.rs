//! Headless snapshots: render a [`View`] or a [`Scene`] to an image without a window, a display or
//! screen-recording permission, so tools (and AI agents) can look at what an app draws.
//!
//! ```ignore
//! # use gyeol::{div, snapshot, Element, View, Cx};
//! # struct App; impl View for App { fn view(&self, _: &mut Cx) -> Element<Self> { div() } }
//! let image = snapshot::view(App, (900., 600.), 2.).expect("a GPU adapter");
//! image.save_png("out.png").unwrap(); // needs the `snapshot` cargo feature
//! ```
//!
//! The image is drawn by the same renderer as a window, so gradients, shadows, inset shadows, text and
//! backdrop blur all appear. Two things are **not** part of it: macOS window vibrancy
//! (`NSVisualEffectView`) and anything else the window system composites behind or around the window
//! (title bar, traffic lights, window shadow). A window that relies on vibrancy shows its translucent
//! parts over the chosen [`Background`] instead.
use crate::{
    error::Result,
    scene::{Color, Scene},
    shaper::Shaper,
    testing::TestHost,
    view::View,
    Renderer,
};

/// What transparent pixels are drawn over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// Keep the alpha channel as it is (straight, not premultiplied).
    Transparent,
    /// A light gray checkerboard, so transparency is visible. The default.
    Checkerboard,
    /// A solid color; the image comes out fully opaque.
    Color(Color),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub background: Background,
}

impl Default for Options {
    fn default() -> Self {
        Options { background: Background::Checkerboard }
    }
}

/// An RGBA8 image, rows from the top, straight (non-premultiplied) alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The `[r, g, b, a]` of the pixel at (`x`, `y`), counted from the top left in device pixels.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Writes the image as a PNG file.
    #[cfg(feature = "snapshot")]
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let fail = |what: &str, e: &dyn std::fmt::Display| crate::Error(format!("{what}: {e}"));
        let file = std::fs::File::create(path.as_ref()).map_err(|e| fail("create png", &e))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| fail("write png header", &e))?;
        writer.write_image_data(&self.rgba).map_err(|e| fail("write png data", &e))
    }
}

/// Renders `view`'s first frame in a window of `size` logical pixels on a display with `scale` device
/// pixels per logical pixel (1 or 2, say), over a checkerboard.
pub fn view<S: View>(state: S, size: (f32, f32), scale: f32) -> Result<Image> {
    view_with(state, size, scale, &Options::default())
}

pub fn view_with<S: View>(state: S, size: (f32, f32), scale: f32, options: &Options) -> Result<Image> {
    let mut host = TestHost::with_scale(state, size, scale);
    let mut renderer = Renderer::offscreen(size, scale)?;
    let premultiplied = host.render(&mut renderer);
    Ok(finish(premultiplied, &renderer, options))
}

/// Renders `scene`, laid out in `size` logical pixels, over a checkerboard.
pub fn scene(scene: &Scene, size: (f32, f32), scale: f32) -> Result<Image> {
    scene_with(scene, size, scale, &Options::default())
}

pub fn scene_with(scene: &Scene, size: (f32, f32), scale: f32, options: &Options) -> Result<Image> {
    let mut renderer = Renderer::offscreen(size, scale)?;
    let premultiplied = renderer.render_to_image(scene, &mut Shaper::new());
    Ok(finish(premultiplied, &renderer, options))
}

/// Turns the renderer's premultiplied pixels into the requested kind of image.
fn finish(mut pixels: Vec<u8>, renderer: &Renderer, options: &Options) -> Image {
    let (scale, (w, h)) = (renderer.scale_factor(), renderer.size_in_pixels());
    let square = (8. * scale).round().max(1.) as u32;
    for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let a = px[3] as f32 / 255.;
        let behind = match options.background {
            Background::Transparent => {
                if px[3] > 0 {
                    for c in &mut px[..3] {
                        *c = (*c as f32 / a).round().min(255.) as u8;
                    }
                }
                continue;
            }
            Background::Checkerboard => {
                let light = (x / square + y / square) % 2 == 0;
                if light { [255., 255., 255.] } else { [204., 204., 204.] }
            }
            Background::Color(c) => [c.r * 255., c.g * 255., c.b * 255.],
        };
        for k in 0..3 {
            px[k] = (px[k] as f32 + behind[k] * (1. - a)).round().min(255.) as u8;
        }
        px[3] = 255;
    }
    Image { width: w, height: h, rgba: pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Quad, Rect};

    /// The image, or `None` (with a note) on a machine with no GPU adapter.
    fn render(scene: &Scene, size: (f32, f32), scale: f32, background: Background) -> Option<Image> {
        match scene_with(scene, size, scale, &Options { background }) {
            Ok(image) => Some(image),
            Err(e) => {
                eprintln!("skipping, no GPU to render with: {e}");
                None
            }
        }
    }

    fn solid(c: u32) -> Background {
        Background::Color(Color::hex(c))
    }

    fn red_range(image: &Image, y: u32, xs: std::ops::Range<u32>) -> u8 {
        let reds: Vec<u8> = xs.map(|x| image.pixel(x, y)[0]).collect();
        reds.iter().max().unwrap() - reds.iter().min().unwrap()
    }

    #[test]
    fn a_gradient_quad_runs_from_its_first_color_to_its_second() {
        let mut scene = Scene::default();
        let (black, white) = (Color::hex(0x000000), Color::hex(0xffffff));
        scene.push_quad(Quad::new(Rect::new(0., 0., 100., 20.), black).gradient(90., black, white));
        let Some(image) = render(&scene, (100., 20.), 1., solid(0xff0000)) else { return };
        let (left, middle, right) = (image.pixel(2, 10), image.pixel(50, 10), image.pixel(97, 10));
        assert!(left[0] < 15 && right[0] > 240, "left {left:?} right {right:?}");
        assert!((110..145).contains(&middle[0]), "the middle is about half way: {middle:?}");
        assert_eq!(middle[3], 255);
        let reds: Vec<u8> = (0..100).map(|x| image.pixel(x, 10)[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] <= w[1]), "never gets darker towards the right");
    }

    #[test]
    fn the_scale_factor_sets_the_pixel_size_and_the_background_shows_through() {
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., 10., 10.), Color::hex(0x00ff00)));
        let Some(one) = render(&scene, (20., 10.), 1., Background::Transparent) else { return };
        let Some(two) = render(&scene, (20., 10.), 2., Background::Transparent) else { return };
        assert_eq!((one.width, one.height, two.width, two.height), (20, 10, 40, 20));
        assert_eq!(one.pixel(5, 5), [0, 255, 0, 255]);
        assert_eq!(two.pixel(15, 15), [0, 255, 0, 255], "10 logical px are 20 device px");
        assert_eq!(two.pixel(25, 5)[3], 0, "outside the quad it stays transparent");

        let Some(checker) = render(&scene, (40., 10.), 1., Background::Checkerboard) else { return };
        assert_eq!(checker.pixel(20, 2), [255, 255, 255, 255], "the first 8px square is light");
        assert_eq!(checker.pixel(28, 2), [204, 204, 204, 255], "the next one is dark");
        assert_eq!(checker.pixel(5, 5), [0, 255, 0, 255], "an opaque quad hides it");
    }

    /// Black stripes, 4px on and 4px off, with a nearly clear panel over the middle.
    fn striped(blur: bool) -> Scene {
        let mut scene = Scene { background: Some(Color::hex(0xffffff)), ..Default::default() };
        for i in 0..25 {
            scene.push_quad(Quad::new(Rect::new(i as f32 * 8., 0., 4., 100.), Color::hex(0x000000)));
        }
        let panel = Quad::new(Rect::new(40., 20., 120., 60.), Color::hex(0xffffff).with_alpha(0.1)).rounded(12.);
        scene.push_quad(if blur { panel.backdrop_blurred(8., 1.) } else { panel });
        scene
    }

    #[test]
    fn a_backdrop_blur_smooths_what_is_behind_the_panel_and_leaves_the_rest_alone() {
        let (Some(sharp), Some(blurred)) = (
            render(&striped(false), (200., 100.), 1., solid(0xff0000)),
            render(&striped(true), (200., 100.), 1., solid(0xff0000)),
        ) else {
            return;
        };
        // Without the blur the stripes show through the panel at full contrast.
        assert!(red_range(&sharp, 50, 70..130) > 200);
        // With it they melt into a nearly flat gray, a different value from both black and white.
        assert!(red_range(&blurred, 50, 70..130) < 30, "smooth: {}", red_range(&blurred, 50, 70..130));
        let middle = blurred.pixel(100, 50)[0];
        assert!((90..200).contains(&middle), "gray, neither stripe color: {middle}");
        // Outside the panel nothing changed, and the corner is rounded off.
        assert!(red_range(&blurred, 50, 0..30) > 200);
        assert_eq!(sharp.pixel(10, 50), blurred.pixel(10, 50));
        assert_eq!(blurred.pixel(41, 21), sharp.pixel(41, 21), "the rounded corner is not blurred");
    }

    /// An opaque wallpaper over the whole image with a nearly clear blurred panel inset from its edges.
    fn walled(blur_radius: f32, size: (f32, f32)) -> Scene {
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., size.0, size.1), Color::hex(0x202040)));
        let panel = Rect::new(16., 16., size.0 - 32., size.1 - 32.);
        scene.push_quad(Quad::new(panel, Color::hex(0xffffff).with_alpha(0.1)).rounded(10.).backdrop_blurred(blur_radius, 1.));
        scene
    }

    #[test]
    fn a_blurred_panel_stays_opaque_over_an_opaque_backdrop_right_up_to_its_edge() {
        for scale in [1., 2.] {
            let Some(image) = render(&walled(8., (120., 80.)), (120., 80.), scale, Background::Transparent) else { return };
            let (y, x0) = ((40. * scale) as u32, (16. * scale) as u32);
            let alphas: Vec<u8> = (x0 - 3..x0 + 8).map(|x| image.pixel(x, y)[3]).collect();
            assert!(alphas.iter().all(|&a| a >= 254), "scale {scale}: the panel edge is see-through: {alphas:?}");
        }
    }
}
