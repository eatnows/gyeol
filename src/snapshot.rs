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
    /// Where the pointer is for [`view_with`], in logical pixels: elements under it are hovered, so
    /// hover styles show. `None` leaves the pointer at the window's top-left corner like a fresh window.
    pub pointer: Option<(f32, f32)>,
    /// For [`view_with`]: time to let pass before the picture is taken, running the frames and
    /// [`View::tick`] calls the app asked for (an animation shown at second `advance`).
    pub advance: std::time::Duration,
    /// For [`view_with`]: render as if the system's Reduce Transparency option were on, so
    /// `Cx::reduce_transparency` is true (the app also receives `Event::DisplayOptionsChanged`).
    pub reduce_transparency: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { background: Background::Checkerboard, pointer: None, advance: std::time::Duration::ZERO, reduce_transparency: false }
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
    if let Some(pointer) = options.pointer {
        host.mouse_move(pointer);
    }
    if options.reduce_transparency {
        host.set_reduce_transparency(true);
    }
    host.advance(options.advance);
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
    use crate::scene::{Gradient, Quad, RadialSize, Rect, Shadow};

    /// The image, or `None` (with a note) on a machine with no GPU adapter.
    fn render(scene: &Scene, size: (f32, f32), scale: f32, background: Background) -> Option<Image> {
        match scene_with(scene, size, scale, &Options { background, ..Options::default() }) {
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

    #[test]
    fn an_edge_on_a_device_pixel_boundary_is_not_blurred_at_any_scale() {
        // Edges are anti-aliased over one device pixel, not one logical pixel: at scale 2 the outermost
        // pixel of a quad whose edge sits on a pixel boundary is fully covered.
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., 10., 10.), Color::hex(0x00ff00)));
        for scale in [1., 2., 3.] {
            let Some(image) = render(&scene, (20., 10.), scale, Background::Transparent) else { return };
            let n = (10. * scale) as u32;
            assert_eq!(image.pixel(0, 0), [0, 255, 0, 255], "scale {scale}: the corner pixel");
            assert_eq!(image.pixel(n - 1, 5), [0, 255, 0, 255], "scale {scale}: the last covered column");
            assert_eq!(image.pixel(n, 5)[3], 0, "scale {scale}: the first column outside");
        }
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

    #[test]
    fn a_big_blur_does_not_smear_the_image_border_into_the_panel() {
        // The wallpaper's outermost pixels are only partly covered at scale 2, so clamping the blur to
        // them used to make the panel's left part see-through (and show a checkerboard behind it).
        let Some(image) = render(&walled(34., (240., 160.)), (240., 160.), 2., Background::Transparent) else { return };
        let alphas: Vec<u8> = (36..200).step_by(4).map(|x| image.pixel(x, 160)[3]).collect();
        assert!(alphas.iter().all(|&a| a >= 254), "see-through near the left edge: {alphas:?}");
    }

    fn close(pixel: [u8; 4], expected: [u8; 3], tolerance: i32) -> bool {
        (0..3).all(|k| (pixel[k] as i32 - expected[k] as i32).abs() <= tolerance)
    }

    #[test]
    fn a_three_stop_linear_gradient_passes_through_its_middle_color() {
        let (red, green, blue) = (Color::hex(0xff0000), Color::hex(0x00ff00), Color::hex(0x0000ff));
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 20.), red).gradient_layer(Gradient::linear(90.).stop(red).stop(green).stop(blue)));
        let Some(image) = render(&scene, (200., 20.), 1., solid(0xffffff)) else { return };
        assert!(close(image.pixel(1, 10), [255, 0, 0], 8), "{:?}", image.pixel(1, 10));
        assert!(close(image.pixel(100, 10), [0, 255, 0], 8), "the middle stop is green: {:?}", image.pixel(100, 10));
        assert!(close(image.pixel(198, 10), [0, 0, 255], 8), "{:?}", image.pixel(198, 10));
        assert!(close(image.pixel(50, 10), [127, 127, 0], 10), "halfway to green: {:?}", image.pixel(50, 10));

        // An explicit position moves the middle stop: green at 25%.
        let mut scene = Scene::default();
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 20.), red).gradient_layer(Gradient::linear(90.).stop(red).stop_at(green, 0.25).stop(blue)));
        let Some(image) = render(&scene, (200., 20.), 1., solid(0xffffff)) else { return };
        assert!(close(image.pixel(50, 10), [0, 255, 0], 8), "{:?}", image.pixel(50, 10));
    }

    #[test]
    fn a_radial_gradient_is_centered_where_asked_and_reaches_its_last_stop_at_its_radius() {
        let (white, black) = (Color::hex(0xffffff), Color::hex(0x000000));
        let mut scene = Scene::default();
        let g = Gradient::radial((0.25, 0.5), RadialSize::Circle(50.)).stop(white).stop(black);
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 100.), black).gradient_layer(g));
        let Some(image) = render(&scene, (200., 100.), 1., solid(0xff0000)) else { return };
        assert!(close(image.pixel(50, 50), [255, 255, 255], 6), "center: {:?}", image.pixel(50, 50));
        let half = image.pixel(75, 50);
        assert!(close(half, [127, 127, 127], 10), "half way out: {half:?}");
        assert!(close(image.pixel(110, 50), [0, 0, 0], 2), "beyond the radius: {:?}", image.pixel(110, 50));
        assert_eq!(image.pixel(50, 4), image.pixel(50, 95), "symmetric above and below");

        // An ellipse: wider than tall.
        let mut scene = Scene::default();
        let g = Gradient::radial((0.5, 0.5), RadialSize::Ellipse(80., 20.)).stop(white).stop(black);
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 100.), black).gradient_layer(g));
        let Some(image) = render(&scene, (200., 100.), 1., solid(0xff0000)) else { return };
        assert!(image.pixel(100 + 40, 50)[0] > 100, "40px sideways is inside the 80px radius");
        assert!(close(image.pixel(100, 50 + 25), [0, 0, 0], 2), "25px down is outside the 20px radius");
    }

    #[test]
    fn fading_to_transparent_keeps_the_color_instead_of_going_gray() {
        // red -> transparent: premultiplied interpolation keeps the color pure red and only fades the
        // opacity; a straight (non-premultiplied) lerp towards transparent black would go dark red.
        let mut scene = Scene::default();
        let g = Gradient::linear(90.).stop(Color::hex(0xff0000)).stop(Color::TRANSPARENT);
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 20.), Color::TRANSPARENT).gradient_layer(g));
        let Some(image) = render(&scene, (200., 20.), 1., Background::Transparent) else { return };
        let (start, middle) = (image.pixel(1, 10), image.pixel(100, 10));
        assert!(close(start, [255, 0, 0], 8) && start[3] > 245, "{start:?}");
        assert!(close(middle, [255, 0, 0], 8), "still red, only more transparent: {middle:?}");
        assert!((120..136).contains(&middle[3]), "half transparent: {middle:?}");
        assert!(image.pixel(198, 10)[3] < 8);
    }

    #[test]
    fn gradient_layers_stack_bottom_to_top_over_the_background_color() {
        // A radial glow over a linear base over a color: the glow's center shows the glow, far away the base.
        let mut scene = Scene::default();
        let base = Gradient::linear(90.).stop(Color::hex(0x000080)).stop(Color::hex(0x000080));
        let glow = Gradient::radial((0.5, 0.5), RadialSize::Circle(40.)).stop(Color::hex(0xffff00)).stop_at(Color::TRANSPARENT, 1.);
        scene.push_quad(Quad::new(Rect::new(0., 0., 200., 100.), Color::hex(0xff0000)).gradient_layer(base).gradient_layer(glow));
        let Some(image) = render(&scene, (200., 100.), 1., solid(0xffffff)) else { return };
        assert!(close(image.pixel(100, 50), [255, 255, 0], 6), "glow on top: {:?}", image.pixel(100, 50));
        assert!(close(image.pixel(10, 50), [0, 0, 128], 3), "base over the red background: {:?}", image.pixel(10, 50));
    }

    const WHITE: Color = Color::hex(0xffffff);

    fn on_white(quad: Quad, size: (f32, f32), scale: f32) -> Option<Image> {
        let mut scene = Scene { background: Some(WHITE), ..Default::default() };
        scene.push_quad(quad);
        render(&scene, size, scale, solid(0xffffff))
    }

    #[test]
    fn several_outer_shadows_are_all_drawn() {
        // Two hard shadows pushed to opposite sides of a 40px box.
        let quad = Quad::new(Rect::new(80., 40., 40., 40.), Color::hex(0x808080))
            .shadow_layer(Shadow::new(-50., 0., 0., 0., Color::hex(0xff0000)))
            .shadow_layer(Shadow::new(50., 0., 0., 0., Color::hex(0x0000ff)));
        let Some(image) = on_white(quad, (200., 120.), 1.) else { return };
        assert!(close(image.pixel(50, 60), [255, 0, 0], 3), "left shadow: {:?}", image.pixel(50, 60));
        assert!(close(image.pixel(150, 60), [0, 0, 255], 3), "right shadow: {:?}", image.pixel(150, 60));
        assert!(close(image.pixel(100, 60), [128, 128, 128], 3), "the box covers its own shadows");
        assert!(close(image.pixel(100, 110), [255, 255, 255], 1), "nothing elsewhere");
    }

    #[test]
    fn an_outer_shadow_falls_off_like_a_gaussian_with_sigma_half_the_blur() {
        // A big box with a 20px blur (sigma 10): the shadow is half strong at the edge, then fades as
        // 1 - Phi(d / sigma): 16% at 10px out, 2% at 20px.
        let quad = Quad::new(Rect::new(100., 100., 200., 100.), WHITE).shadowed(0., 0., 20., Color::hex(0x000000));
        let Some(image) = on_white(quad, (400., 300.), 1.) else { return };
        let at = |dy: u32| image.pixel(200, 100 - dy)[0] as i32; // above the top edge, away from corners
        for (d, expected) in [(1, 138), (10, 214), (20, 249)] {
            assert!((at(d) - expected).abs() <= 10, "{d}px out: {} vs about {expected}", at(d));
        }
        assert!((1..30).all(|d| at(d) <= at(d + 1)), "monotonically lighter further out");
    }

    #[test]
    fn spread_grows_a_shadow() {
        let shadow = |spread| {
            Quad::new(Rect::new(80., 40., 40., 40.), Color::hex(0x808080)).shadow_layer(Shadow::new(0., 0., 0., spread, Color::hex(0)))
        };
        let (Some(plain), Some(spread)) = (on_white(shadow(0.), (200., 120.), 1.), on_white(shadow(10.), (200., 120.), 1.)) else { return };
        assert_eq!(plain.pixel(74, 60)[0], 255, "6px outside, no spread: no shadow");
        assert!(spread.pixel(74, 60)[0] < 5, "6px outside, 10px spread: inside the shadow");
        assert_eq!(spread.pixel(66, 60)[0], 255, "14px outside is past the spread");
        // A negative spread shrinks it: a 40px box's shadow dropped 30px down and shrunk by 10 on each
        // side covers x 90..110, y 80..100.
        let drop = Quad::new(Rect::new(80., 40., 40., 40.), Color::hex(0x808080)).shadow_layer(Shadow::new(0., 30., 0., -10., Color::hex(0xff0000)));
        let Some(shrunk) = on_white(drop, (200., 120.), 1.) else { return };
        assert!(close(shrunk.pixel(100, 90), [255, 0, 0], 3), "inside the shrunk shadow: {:?}", shrunk.pixel(100, 90));
        assert_eq!(shrunk.pixel(85, 90)[1], 255, "where an unshrunk shadow would have been");
    }

    #[test]
    fn inset_and_outer_shadows_work_together() {
        let quad = Quad::new(Rect::new(60., 40., 80., 60.), Color::hex(0x808080))
            .rounded(10.)
            .inset_shadow_layer(Shadow::new(0., 2., 0., 0., WHITE))
            .shadow_layer(Shadow::new(0., 10., 12., 0., Color::hex(0)));
        let Some(image) = on_white(quad, (200., 160.), 2.) else { return };
        // At scale 2: the top highlight is 2 logical px = 4 device px thick just inside the top edge.
        assert!(close(image.pixel(200, 82), [255, 255, 255], 6), "highlight: {:?}", image.pixel(200, 82));
        assert!(close(image.pixel(200, 140), [128, 128, 128], 3), "the middle keeps its fill: {:?}", image.pixel(200, 140));
        assert!(image.pixel(200, 215)[0] < 200, "the outer shadow darkens the area just below the box: {:?}", image.pixel(200, 215));
        assert_eq!(image.pixel(200, 60)[0], 255, "and not above it, where its offset keeps it away: {:?}", image.pixel(200, 60));
    }

    #[test]
    fn an_inset_shadows_spread_makes_a_band_inside_the_border() {
        let quad = Quad::new(Rect::new(40., 40., 100., 60.), WHITE).inset_shadow_layer(Shadow::new(0., 0., 0., 5., Color::hex(0)));
        let Some(image) = on_white(quad, (200., 140.), 1.) else { return };
        assert!(image.pixel(42, 70)[0] < 5, "2px in: inside the 5px band");
        assert!(image.pixel(70, 43)[0] < 5);
        assert_eq!(image.pixel(70, 70)[0], 255, "the middle is untouched");
        assert_eq!(image.pixel(70, 47)[0], 255, "7px in is past the band");
    }

    #[cfg(feature = "svg")]
    mod icons {
        use super::*;
        use crate::icon::{Icon, IconData};
        use std::sync::Arc;

        const CHEVRON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
        const DISC: &str = r#"<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>"#;
        const RING: &str = r#"<svg viewBox="0 0 24 24"><path fill-rule="evenodd" d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Zm0 6a4 4 0 1 1 0 8 4 4 0 0 1 0-8Z"/></svg>"#;

        fn icon_scene(svg: &str, bounds: Rect, color: Color) -> Scene {
            let mut scene = Scene::default();
            scene.push_icon(Icon::new(IconData::cached(svg), bounds, color));
            scene
        }

        fn coverage(image: &Image) -> usize {
            (0..image.height).flat_map(|y| (0..image.width).map(move |x| (x, y))).filter(|&(x, y)| image.pixel(x, y)[3] > 0).count()
        }

        #[test]
        fn a_stroke_icon_draws_in_the_tint_color_and_only_where_its_stroke_is() {
            let scene = icon_scene(CHEVRON, Rect::new(0., 0., 48., 48.), Color::hex(0x2a9d8f));
            let Some(image) = render(&scene, (48., 48.), 1., Background::Transparent) else { return };
            assert!(coverage(&image) > 100, "something was drawn");
            // The chevron's tip is at (15, 12) of 24 -> (30, 24) of 48; its stroke is 4px wide there.
            assert_eq!(image.pixel(29, 24), [0x2a, 0x9d, 0x8f, 255], "solid tint in the middle of the stroke");
            assert_eq!(image.pixel(6, 6)[3], 0, "empty far from the stroke");
            assert_eq!(image.pixel(40, 40)[3], 0);
            // Every drawn pixel is the tint color (un-premultiplied), just with partial coverage at the edges.
            for y in 0..48 {
                for x in 0..48 {
                    let p = image.pixel(x, y);
                    if p[3] > 128 {
                        assert!(close(p, [0x2a, 0x9d, 0x8f], 4), "({x}, {y}) {p:?}");
                    }
                }
            }
        }

        #[test]
        fn the_tint_alpha_fades_the_icon_and_a_clip_cuts_it() {
            let faint = icon_scene(DISC, Rect::new(0., 0., 40., 40.), Color::hex(0xff0000).with_alpha(0.5));
            let Some(image) = render(&faint, (40., 40.), 1., Background::Transparent) else { return };
            let middle = image.pixel(20, 20);
            assert!(close(middle, [255, 0, 0], 3) && (124..132).contains(&middle[3]), "half transparent red: {middle:?}");

            let mut clipped = Scene::default();
            clipped.push_icon(Icon::new(IconData::cached(DISC), Rect::new(0., 0., 40., 40.), Color::hex(0)).clipped(Some(Rect::new(0., 0., 20., 40.))));
            let Some(image) = render(&clipped, (40., 40.), 1., Background::Transparent) else { return };
            assert_eq!(image.pixel(10, 20)[3], 255);
            assert_eq!(image.pixel(30, 20)[3], 0, "the right half is clipped away");
        }

        #[test]
        fn icons_are_crisp_at_scale_2_rasterized_at_device_resolution() {
            let scene = icon_scene(DISC, Rect::new(2., 2., 10., 10.), Color::hex(0x000000));
            let (Some(one), Some(two)) = (render(&scene, (14., 14.), 1., Background::Transparent), render(&scene, (14., 14.), 2., Background::Transparent)) else { return };
            assert_eq!((one.width, two.width), (14, 28));
            // At scale 2 the disc spans device pixels 4..24 with a sharp edge: its radius is 10 device px,
            // so the edge pixel at (4, 14) is only the 1px anti-aliased fringe, not a blurry upscaled one.
            let row: Vec<u8> = (0..28).map(|x| two.pixel(x, 14)[3]).collect();
            let soft = row.iter().filter(|a| **a > 0 && **a < 255).count();
            assert!(soft <= 4, "at most two soft pixels per edge, got {soft}: {row:?}");
            assert_eq!(row[14], 255);
            assert_eq!(row[1], 0);
            // The area scales with the square of the scale: 4x the pixels (within edge rounding).
            let area = |image: &Image| (0..image.height).flat_map(|y| (0..image.width).map(move |x| (x, y))).map(|(x, y)| image.pixel(x, y)[3] as f32 / 255.).sum::<f32>();
            let ratio = area(&two) / area(&one);
            assert!((3.9..4.1).contains(&ratio), "{ratio}");
        }

        #[test]
        fn a_filled_shape_fills_its_interior_and_evenodd_leaves_a_hole() {
            let disc = icon_scene(DISC, Rect::new(0., 0., 48., 48.), Color::hex(0x336699));
            let Some(image) = render(&disc, (48., 48.), 1., Background::Transparent) else { return };
            assert_eq!(image.pixel(24, 24), [0x33, 0x66, 0x99, 255], "the middle of a filled disc");
            assert_eq!(image.pixel(24, 8), [0x33, 0x66, 0x99, 255]);
            assert_eq!(image.pixel(2, 2)[3], 0, "outside the circle");

            let ring = icon_scene(RING, Rect::new(0., 0., 48., 48.), Color::hex(0x336699));
            let Some(image) = render(&ring, (48., 48.), 1., Background::Transparent) else { return };
            assert_eq!(image.pixel(24, 24)[3], 0, "evenodd leaves the inner disc empty");
            assert_eq!(image.pixel(24, 8), [0x33, 0x66, 0x99, 255], "the ring itself is filled (y 4..16 of 48 is inside r 20, outside r 8)");
        }

        #[test]
        fn icons_follow_painting_order_and_get_blurred_by_a_later_backdrop_blur() {
            // An icon painted before a blurred panel is blurred by it; one painted after stays sharp.
            let mut scene = Scene { background: Some(Color::hex(0xffffff)), ..Default::default() };
            scene.push_icon(Icon::new(Arc::new(IconData::parse(DISC).unwrap()), Rect::new(10., 10., 40., 40.), Color::hex(0)));
            scene.push_quad(Quad::new(Rect::new(0., 0., 60., 30.), Color::hex(0xffffff).with_alpha(0.1)).backdrop_blurred(6., 1.));
            scene.push_icon(Icon::new(IconData::cached(DISC), Rect::new(70., 10., 40., 40.), Color::hex(0)));
            let Some(image) = render(&scene, (120., 60.), 1., solid(0xffffff)) else { return };
            let sharp_edge = image.pixel(70 + 20, 16)[0]; // just inside the top of the right disc (its edge is at y 13.3)
            assert!(sharp_edge < 10, "the later icon is untouched by the blur: {sharp_edge}");
            let blurred_top = image.pixel(30, 16)[0]; // just inside the first disc's top, under the blur panel
            let blurred_below = image.pixel(30, 40)[0]; // below the panel: still sharp black
            assert!(blurred_below < 10, "outside the panel the first disc is sharp: {blurred_below}");
            assert!(blurred_top > blurred_below + 20, "inside the panel its edge is smeared lighter: {blurred_top} vs {blurred_below}");
        }
    }

    fn image_scene(buffer: &crate::ImageBuffer, origin: (f32, f32), zoom: f32, filter: crate::ImageFilter, clip: Option<Rect>) -> Scene {
        let mut scene = Scene::default();
        scene.push_image(crate::ImageDraw { buffer: buffer.clone(), origin, zoom, filter, checkerboard: None, opacity: 1., clip });
        scene
    }

    #[test]
    fn an_image_is_zoomed_with_hard_pixels_and_clipped_to_its_rectangle() {
        let buffer = crate::ImageBuffer::from_rgba(2, 2, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]].concat()).unwrap();
        let scene = image_scene(&buffer, (5., 5.), 10., crate::ImageFilter::Nearest, Some(Rect::new(0., 0., 20., 20.)));
        let Some(image) = render(&scene, (40., 40.), 1., solid(0x202020)) else { return };
        assert_eq!(image.pixel(10, 10), [255, 0, 0, 255], "the first pixel, magnified");
        assert_eq!(image.pixel(14, 14), [255, 0, 0, 255], "still inside it: a hard edge");
        assert_eq!(image.pixel(16, 10), [0, 255, 0, 255], "the second pixel");
        assert_eq!(image.pixel(10, 16), [0, 0, 255, 255]);
        assert_eq!(image.pixel(10, 4), [0x20, 0x20, 0x20, 255], "above the image: the background");
        assert_eq!(image.pixel(22, 10), [0x20, 0x20, 0x20, 255], "the image runs to x=25 but the clip ends at 20");
        // The same, auto-filtered at 10x, is nearest too; and Linear smooths the edge.
        let smooth = image_scene(&buffer, (5., 5.), 10., crate::ImageFilter::Linear, None);
        let Some(image) = render(&smooth, (40., 40.), 1., solid(0x202020)) else { return };
        let edge = image.pixel(15, 10);
        assert!(edge[0] > 60 && edge[0] < 200 && edge[1] > 60 && edge[1] < 200, "between red and green at their border: {edge:?}");
        let auto = image_scene(&buffer, (5., 5.), 10., crate::ImageFilter::Auto, None);
        let Some(image) = render(&auto, (40., 40.), 1., solid(0x202020)) else { return };
        assert_eq!(image.pixel(14, 10), [255, 0, 0, 255]);
        let small = image_scene(&buffer, (5., 5.), 1., crate::ImageFilter::Auto, None);
        let Some(image) = render(&small, (40., 40.), 1., solid(0x202020)) else { return };
        assert_eq!(image.pixel(5, 5), [255, 0, 0, 255], "at 1x each pixel lands on a pixel");
        assert_eq!(image.pixel(6, 5), [0, 255, 0, 255]);
    }

    #[test]
    fn transparent_pixels_show_the_checkerboard_and_the_background() {
        let buffer = crate::ImageBuffer::new(8, 8);
        buffer.write_rect(crate::PixelRect::new(0, 0, 4, 8), &[128, 0, 0, 128].repeat(32)); // half-transparent red on the left
        let mut scene = Scene::default();
        let (light, dark) = (Color::hex(0xffffff), Color::hex(0x000000));
        scene.push_image(crate::ImageDraw {
            buffer: buffer.clone(),
            origin: (0., 0.),
            zoom: 4.,
            filter: crate::ImageFilter::Nearest,
            checkerboard: Some(crate::Checkerboard { light, dark, cell: 8. }),
            opacity: 1.,
            clip: None,
        });
        let Some(image) = render(&scene, (32., 32.), 1., solid(0x336699)) else { return };
        // Right half is transparent image: the checkerboard (cells of 8) shows, never the background.
        let (a, b) = (image.pixel(20, 2), image.pixel(28, 2));
        assert_eq!([a[0], a[1], a[2]], [255, 255, 255], "first cell is light");
        assert_eq!([b[0], b[1], b[2]], [0, 0, 0], "the next is dark");
        assert_eq!(image.pixel(20, 10)[0], 0, "and the row below flips");
        // Left half: half-transparent red over the checker: over white it is pink, over black dark red.
        let (over_light, over_dark) = (image.pixel(2, 2), image.pixel(10, 2));
        assert!(over_light[0] > 200 && over_light[1] > 100 && over_light[1] < 140, "{over_light:?}");
        assert!(over_dark[0] > 100 && over_dark[0] < 140 && over_dark[1] < 10, "{over_dark:?}");
    }

    /// Rows (top to bottom) that hold any dark text pixel.
    fn inked_rows(image: &Image) -> Vec<u32> {
        (0..image.height).filter(|&y| (0..image.width).any(|x| image.pixel(x, y)[0] < 128)).collect()
    }

    #[test]
    fn wrapped_text_draws_several_lines_inside_its_width_and_short_text_one() {
        use crate::{div, text, view::View, Cx, Element};
        struct Bubble(&'static str);
        impl View for Bubble {
            fn view(&self, _: &mut Cx) -> Element<Bubble> {
                div().child(div().max_w(120.).child(text(self.0).text_size(14.).text_color(Color::hex(0)).wrap_text()))
            }
        }
        let render = |t| view(Bubble(t), (300., 120.), 1.);
        let (Ok(long), Ok(short)) = (render("a fairly long sentence that cannot fit on one short line"), render("short")) else {
            eprintln!("skipping, no GPU");
            return;
        };
        let (long_rows, short_rows) = (inked_rows(&long), inked_rows(&short));
        assert!(long_rows.last().unwrap() - long_rows[0] > 2 * 14, "several lines: {long_rows:?}");
        assert!(short_rows.last().unwrap() - short_rows[0] < 19, "one line: {short_rows:?}");
        assert!((0..long.height).all(|y| (120..300).all(|x| long.pixel(x, y)[0] > 128)), "nothing drawn right of the 120px width");
    }
}
