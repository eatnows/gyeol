//! Pixel images the app owns and draws: [`ImageBuffer`] holds the pixels, [`image_surface`](crate::image_surface)
//! shows them with a zoom and pan, and [`ImageView`] converts between element and image coordinates.
//!
//! The buffer is a shared handle (`Clone` is cheap and every clone is the same image): the app keeps one
//! and edits it with [`ImageBuffer::edit`]; the element only holds a reference. Each edit bumps a
//! revision number, and the renderer uploads to the GPU only when the revision changed, and then only
//! the rectangle that was reported dirty.
//!
//! **Pixel format: RGBA, 8 bits per channel, premultiplied alpha** (each color channel is already
//! multiplied by alpha), rows from the top, no padding between rows. Colors are sRGB-encoded values
//! used as they are, like every other color in gyeol.
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use crate::{
    error::{Error, Result},
    scene::{Color, Rect},
};

/// A rectangle of pixels: `x`, `y` from the top-left, `w` by `h` pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl PixelRect {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> PixelRect {
        PixelRect { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The smallest rectangle containing both (an empty one contributes nothing).
    pub fn union(self, other: PixelRect) -> PixelRect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let (x1, y1) = ((self.x + self.w).max(other.x + other.w), (self.y + self.h).max(other.y + other.h));
        PixelRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    /// The part inside a `width` x `height` image.
    pub fn clamped(self, width: u32, height: u32) -> PixelRect {
        let (x, y) = (self.x.min(width), self.y.min(height));
        PixelRect { x, y, w: self.w.min(width - x), h: self.h.min(height - y) }
    }
}

/// How much edit history is kept for partial uploads; a GPU copy older than this is re-uploaded whole.
const HISTORY: usize = 64;

pub(crate) struct State {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    revision: u64,
    /// The edits that made revisions `floor + 1 ..= revision`; `None` is "everything changed".
    history: VecDeque<(u64, Option<PixelRect>)>,
    floor: u64,
}

/// What a GPU copy at some revision must upload to catch up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dirty {
    Nothing,
    Rect(PixelRect),
    All,
}

impl State {
    fn record(&mut self, dirty: Option<PixelRect>) {
        self.revision += 1;
        self.history.push_back((self.revision, dirty));
        while self.history.len() > HISTORY {
            self.floor = self.history.pop_front().map_or(self.floor, |(r, _)| r);
        }
    }

    pub(crate) fn dirty_since(&self, revision: u64) -> Dirty {
        if revision >= self.revision {
            return Dirty::Nothing;
        }
        if revision < self.floor {
            return Dirty::All;
        }
        let mut rect = PixelRect::default();
        for (r, dirty) in &self.history {
            if *r > revision {
                match dirty {
                    Some(d) => rect = rect.union(*d),
                    None => return Dirty::All,
                }
            }
        }
        Dirty::Rect(rect)
    }
}

struct Shared {
    id: u64,
    state: Mutex<State>,
}

/// A pixel buffer shared between the app (which edits it) and the elements that draw it.
/// See the [module](self) for the pixel format.
#[derive(Clone)]
pub struct ImageBuffer {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for ImageBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (w, h) = self.size();
        write!(f, "ImageBuffer(#{} {w}x{h} rev {})", self.shared.id, self.revision())
    }
}

impl ImageBuffer {
    /// A fully transparent `width` x `height` image.
    pub fn new(width: u32, height: u32) -> ImageBuffer {
        Self::make(width, height, vec![0; width as usize * height as usize * 4])
    }

    /// An image from premultiplied RGBA pixels, `width * height * 4` bytes, rows from the top.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<ImageBuffer> {
        if pixels.len() != width as usize * height as usize * 4 {
            return Err(Error(format!("image buffer: {} bytes is not {width}x{height} RGBA", pixels.len())));
        }
        Ok(Self::make(width, height, pixels))
    }

    fn make(width: u32, height: u32, pixels: Vec<u8>) -> ImageBuffer {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let state = State { width, height, pixels, revision: 1, history: VecDeque::new(), floor: 1 };
        ImageBuffer { shared: Arc::new(Shared { id: NEXT.fetch_add(1, Ordering::Relaxed), state: Mutex::new(state) }) }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Identifies this image (all clones share it); the renderer keys its GPU copy on it.
    pub fn id(&self) -> u64 {
        self.shared.id
    }

    /// Width and height in pixels.
    pub fn size(&self) -> (u32, u32) {
        let s = self.lock();
        (s.width, s.height)
    }

    /// Changes whenever the pixels do. Compare two readings to see whether anything changed.
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Reads the pixels: `f(pixels, width, height)`.
    pub fn read<R>(&self, f: impl FnOnce(&[u8], u32, u32) -> R) -> R {
        let s = self.lock();
        f(&s.pixels, s.width, s.height)
    }

    /// Edits the pixels with `f(pixels, width)` (the whole buffer, rows of `width * 4` bytes) and bumps
    /// the revision. `dirty` is the rectangle `f` changes, so only that part is uploaded to the GPU;
    /// `None` means anywhere. A `dirty` rectangle that lies outside the image changes nothing and
    /// does not bump the revision.
    pub fn edit<R>(&self, dirty: Option<PixelRect>, f: impl FnOnce(&mut [u8], u32) -> R) -> R {
        let mut s = self.lock();
        let rect = dirty.map(|d| d.clamped(s.width, s.height));
        let width = s.width;
        let out = f(&mut s.pixels, width);
        if rect.is_none_or(|r| !r.is_empty()) {
            s.record(rect);
        }
        out
    }

    /// Replaces the pixels of `rect` (clipped to the image) with `data`, `rect.w * rect.h * 4` bytes.
    /// Does nothing when `data` has the wrong length.
    pub fn write_rect(&self, rect: PixelRect, data: &[u8]) {
        if data.len() != rect.w as usize * rect.h as usize * 4 {
            return;
        }
        self.edit(Some(rect), |pixels, width| {
            let width = width as usize;
            let height = pixels.len() / 4 / width.max(1);
            for row in 0..rect.h as usize {
                let y = rect.y as usize + row;
                if y >= height {
                    break;
                }
                let x0 = (rect.x as usize).min(width);
                let n = (rect.w as usize).min(width - x0);
                let dst = (y * width + x0) * 4;
                let src = row * rect.w as usize * 4;
                pixels[dst..dst + n * 4].copy_from_slice(&data[src..src + n * 4]);
            }
        });
    }

    /// Sets every pixel to premultiplied `rgba`.
    pub fn fill(&self, rgba: [u8; 4]) {
        self.edit(None, |pixels, _| pixels.chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&rgba)));
    }

    /// Changes the size to `width` x `height`; the image becomes fully transparent.
    pub fn resize(&self, width: u32, height: u32) {
        let mut s = self.lock();
        s.width = width;
        s.height = height;
        s.pixels = vec![0; width as usize * height as usize * 4];
        s.history.clear();
        s.revision += 1;
        s.floor = s.revision;
    }

    pub(crate) fn with_state<R>(&self, f: impl FnOnce(&State, &[u8]) -> R) -> R {
        let s = self.lock();
        f(&s, &s.pixels)
    }
}

impl State {
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

/// How the image is sampled when it is drawn at a size other than its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFilter {
    /// Smooth: right for zooming out. (No mipmaps yet, so far below half size it aliases.)
    Linear,
    /// Hard pixel edges: right for zooming far in, to see single pixels.
    Nearest,
    /// Linear below 2x zoom, nearest from 2x up.
    #[default]
    Auto,
}

/// Zoom at which [`ImageFilter::Auto`] switches to nearest.
pub const AUTO_NEAREST_ZOOM: f32 = 2.;

impl ImageFilter {
    pub(crate) fn nearest_at(self, zoom: f32) -> bool {
        match self {
            ImageFilter::Linear => false,
            ImageFilter::Nearest => true,
            ImageFilter::Auto => zoom >= AUTO_NEAREST_ZOOM,
        }
    }
}

/// A two-color checkerboard drawn behind the image's pixels, to make transparency visible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Checkerboard {
    pub light: Color,
    pub dark: Color,
    /// Square size in logical pixels on screen (not scaled by the zoom).
    pub cell: f32,
}

/// How an image is placed in its element: zoom, pan, sampling and an optional checkerboard.
///
/// The image's pixel `(u, v)` appears at element-local `pan + (u, v) * zoom`, where element-local
/// coordinates start at the element's top-left corner (the `local` of a
/// [`MouseEvent`](crate::MouseEvent)). The element is as big as the image by default and clips
/// what falls outside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageView {
    /// Screen pixels (logical) per image pixel.
    pub zoom: f32,
    /// Where the image's top-left corner is, relative to the element's top-left corner.
    pub pan: (f32, f32),
    pub filter: ImageFilter,
    pub checkerboard: Option<Checkerboard>,
}

impl Default for ImageView {
    fn default() -> Self {
        ImageView { zoom: 1., pan: (0., 0.), filter: ImageFilter::Auto, checkerboard: None }
    }
}

impl ImageView {
    pub fn new() -> ImageView {
        ImageView::default()
    }

    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    pub fn pan(mut self, x: f32, y: f32) -> Self {
        self.pan = (x, y);
        self
    }

    pub fn filter(mut self, filter: ImageFilter) -> Self {
        self.filter = filter;
        self
    }

    pub fn checkerboard(mut self, light: Color, dark: Color, cell: f32) -> Self {
        self.checkerboard = Some(Checkerboard { light, dark, cell });
        self
    }

    /// Element-local position of image pixel coordinates `(u, v)` (fractions allowed: `(0.5, 0.5)` is
    /// the middle of the first pixel).
    pub fn image_to_local(&self, (u, v): (f32, f32)) -> (f32, f32) {
        (self.pan.0 + u * self.zoom, self.pan.1 + v * self.zoom)
    }

    /// Image pixel coordinates under an element-local position; `floor` them for the pixel index.
    pub fn local_to_image(&self, (x, y): (f32, f32)) -> (f32, f32) {
        ((x - self.pan.0) / self.zoom, (y - self.pan.1) / self.zoom)
    }

    /// The pixel index under an element-local position, or `None` outside a `width` x `height` image.
    pub fn pixel_at(&self, local: (f32, f32), width: u32, height: u32) -> Option<(u32, u32)> {
        let (u, v) = self.local_to_image(local);
        (u >= 0. && v >= 0. && u < width as f32 && v < height as f32).then(|| (u as u32, v as u32))
    }

    /// Zooms by `factor` (clamped to `min..=max` in total) keeping the image point under `anchor`
    /// (element-local, e.g. the cursor) where it is.
    pub fn zoom_at(mut self, anchor: (f32, f32), factor: f32, min: f32, max: f32) -> Self {
        let new = (self.zoom * factor).clamp(min, max);
        let k = new / self.zoom;
        self.pan = (anchor.0 - (anchor.0 - self.pan.0) * k, anchor.1 - (anchor.1 - self.pan.1) * k);
        self.zoom = new;
        self
    }

    /// Zoom and pan that fit an `image` (width, height) inside a `view` (width, height) box, centered,
    /// with `margin` pixels to spare on every side.
    pub fn fit(mut self, image: (u32, u32), view: (f32, f32), margin: f32) -> Self {
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        let zoom = ((view.0 - 2. * margin) / iw).min((view.1 - 2. * margin) / ih).max(f32::MIN_POSITIVE);
        self.zoom = zoom;
        self.pan = ((view.0 - iw * zoom) / 2., (view.1 - ih * zoom) / 2.);
        self
    }
}

/// An image placed in a scene (what [`image_surface`](crate::image_surface) paints).
#[derive(Clone, Debug)]
pub struct ImageDraw {
    pub buffer: ImageBuffer,
    /// Where the image's top-left corner is in the window.
    pub origin: (f32, f32),
    pub zoom: f32,
    pub filter: ImageFilter,
    pub checkerboard: Option<Checkerboard>,
    /// Multiplies the image's alpha.
    pub opacity: f32,
    /// Nothing is drawn outside this (the element's box intersected with the ancestors' clips).
    pub clip: Option<Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev_rect(b: &ImageBuffer, since: u64) -> Dirty {
        b.with_state(|s, _| s.dirty_since(since))
    }

    #[test]
    fn edits_bump_the_revision_and_report_the_union_of_what_changed_since_a_revision() {
        let b = ImageBuffer::new(16, 16);
        let r0 = b.revision();
        assert_eq!(rev_rect(&b, r0), Dirty::Nothing);
        b.write_rect(PixelRect::new(1, 2, 3, 4), &[1; 48]);
        b.write_rect(PixelRect::new(10, 8, 2, 2), &[1; 16]);
        assert_eq!(b.revision(), r0 + 2);
        assert_eq!(rev_rect(&b, r0), Dirty::Rect(PixelRect::new(1, 2, 11, 8)));
        assert_eq!(rev_rect(&b, r0 + 1), Dirty::Rect(PixelRect::new(10, 8, 2, 2)), "a copy that saw the first edit needs only the second");
        b.fill([0, 0, 0, 0]);
        assert_eq!(rev_rect(&b, r0 + 1), Dirty::All, "an edit with no rectangle may have changed anything");
        assert_eq!(rev_rect(&b, b.revision()), Dirty::Nothing);
    }

    #[test]
    fn old_history_falls_back_to_a_full_upload_and_resize_resets_it() {
        let b = ImageBuffer::new(4, 4);
        let start = b.revision();
        for _ in 0..HISTORY + 5 {
            b.write_rect(PixelRect::new(0, 0, 1, 1), &[0; 4]);
        }
        assert_eq!(rev_rect(&b, start), Dirty::All, "too far behind to patch");
        assert!(matches!(rev_rect(&b, b.revision() - 3), Dirty::Rect(_)), "recent copies still patch");
        let before = b.revision();
        b.resize(8, 8);
        assert_eq!(b.size(), (8, 8));
        assert_eq!(rev_rect(&b, before), Dirty::All);
        b.read(|p, w, h| assert_eq!((p.len(), w, h), (8 * 8 * 4, 8, 8)));
    }

    #[test]
    fn dirty_rectangles_outside_the_image_change_nothing_and_inside_ones_are_clipped() {
        let b = ImageBuffer::new(10, 10);
        let r = b.revision();
        b.edit(Some(PixelRect::new(20, 20, 5, 5)), |_, _| {});
        assert_eq!(b.revision(), r, "nothing in the image changed");
        b.edit(Some(PixelRect::new(8, 8, 9, 9)), |_, _| {});
        assert_eq!(rev_rect(&b, r), Dirty::Rect(PixelRect::new(8, 8, 2, 2)));
        b.write_rect(PixelRect::new(8, 8, 4, 4), &[7; 64]); // sticks out: clipped, no panic
        b.read(|p, w, _| assert_eq!(&p[(9 * w as usize + 9) * 4..][..4], &[7; 4]));
        let before = b.revision();
        b.write_rect(PixelRect::new(0, 0, 2, 2), &[0; 3]); // wrong length: ignored
        assert_eq!(b.revision(), before);
        assert!(ImageBuffer::from_rgba(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn clones_are_the_same_image() {
        let a = ImageBuffer::new(2, 2);
        let b = a.clone();
        a.fill([1, 2, 3, 4]);
        assert_eq!(b.revision(), a.revision());
        assert_eq!(b.id(), a.id());
        b.read(|p, _, _| assert_eq!(&p[..4], &[1, 2, 3, 4]));
    }

    #[test]
    fn coordinates_convert_both_ways_through_zoom_and_pan() {
        let v = ImageView::new().zoom(4.).pan(10., -6.);
        assert_eq!(v.image_to_local((0., 0.)), (10., -6.));
        assert_eq!(v.image_to_local((2.5, 1.)), (20., -2.));
        assert_eq!(v.local_to_image((20., -2.)), (2.5, 1.));
        for p in [(0., 0.), (13.5, 7.25), (-3., 40.)] {
            let back = v.image_to_local(v.local_to_image(p));
            assert!((back.0 - p.0).abs() < 1e-4 && (back.1 - p.1).abs() < 1e-4);
        }
        assert_eq!(v.pixel_at((11., -5.), 8, 8), Some((0, 0)));
        assert_eq!(v.pixel_at((9., -5.), 8, 8), None, "left of the image");
        assert_eq!(v.pixel_at((10. + 8. * 4. - 0.1, -6. + 7. * 4.), 8, 8), Some((7, 7)), "the last pixel");
        assert_eq!(v.pixel_at((10. + 8. * 4., -6.), 8, 8), None, "just past the right edge");
    }

    #[test]
    fn zooming_at_a_point_keeps_the_image_under_it_still() {
        let v = ImageView::new().zoom(2.).pan(30., 10.);
        let anchor = (100., 80.);
        let before = v.local_to_image(anchor);
        let z = v.zoom_at(anchor, 1.5, 0.1, 64.);
        assert_eq!(z.zoom, 3.);
        let after = z.local_to_image(anchor);
        assert!((before.0 - after.0).abs() < 1e-4 && (before.1 - after.1).abs() < 1e-4);
        assert_eq!(v.zoom_at(anchor, 100., 0.1, 8.).zoom, 8., "clamped to the maximum");
    }

    #[test]
    fn fit_centers_the_image_in_the_box() {
        let v = ImageView::new().fit((100, 50), (400., 300.), 10.);
        assert_eq!(v.zoom, 3.8);
        assert!((v.pan.0 - 10.).abs() < 1e-4 && (v.pan.1 - (300. - 190.) / 2.).abs() < 1e-4, "{:?}", v.pan);
    }
}
