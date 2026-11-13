//! Pointer input beyond the mouse: pen pressure, tilt, and the eraser end of a stylus.
//!
//! The app sees one platform-independent API: [`PointerKind`] and [`PointerSample`], delivered through
//! [`MouseEvent`](crate::MouseEvent) (`pointer`, `pressure`, `tilt`) and [`Cx::pointer_samples`](crate::Cx::pointer_samples)
//! (every sample since the previous event, for smooth strokes). Each platform fills them in from its
//! own event source; winit's events carry no pen data on desktop, so this is gyeol's own path.
//!
//! | Platform | Pen data |
//! |---|---|
//! | macOS | implemented (NSEvent tablet data via an event monitor) |
//! | Windows, Linux, others | **not implemented**: always [`PointerKind::Mouse`], pressure 1 while pressed |
//!
//! A platform that cannot supply a pen behaves exactly like before: a mouse.
use std::collections::VecDeque;

/// What is pointing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerKind {
    /// A mouse, trackpad, or anything that has no pressure: pressure is 1 while a button is down.
    #[default]
    Mouse,
    /// The tip of a stylus.
    Pen,
    /// The eraser end of a stylus.
    Eraser,
}

/// One reading of the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSample {
    /// Position in logical pixels from the window's top-left corner.
    pub pos: (f32, f32),
    /// 0 to 1. A mouse reports 1 while a button is down and 0 otherwise; a pen reports its pressure
    /// (0 while hovering).
    pub pressure: f32,
    /// How far the pen leans, each axis from -1 to 1 (x: towards the right, y: towards the bottom);
    /// `None` when the device cannot tell.
    pub tilt: Option<(f32, f32)>,
    /// Barrel rotation in degrees, if the device reports it.
    pub rotation: Option<f32>,
    pub kind: PointerKind,
    /// Seconds on the system's event clock; only differences are meaningful.
    pub time: f64,
}

impl PointerSample {
    /// A mouse reading: pressure 1 while `pressed`.
    pub fn mouse(pos: (f32, f32), pressed: bool) -> PointerSample {
        PointerSample { pos, pressure: if pressed { 1. } else { 0. }, tilt: None, rotation: None, kind: PointerKind::Mouse, time: 0. }
    }

    /// A pen reading with `pressure` (clamped to 0..=1).
    pub fn pen(pos: (f32, f32), pressure: f32) -> PointerSample {
        PointerSample { pos, pressure: pressure.clamp(0., 1.), tilt: None, rotation: None, kind: PointerKind::Pen, time: 0. }
    }

    pub fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = pressure.clamp(0., 1.);
        self
    }

    pub fn eraser(mut self) -> Self {
        self.kind = PointerKind::Eraser;
        self
    }

    pub fn tilt(mut self, x: f32, y: f32) -> Self {
        self.tilt = Some((x.clamp(-1., 1.), y.clamp(-1., 1.)));
        self
    }

    pub fn at_time(mut self, time: f64) -> Self {
        self.time = time;
        self
    }
}

/// Most samples kept while waiting for the matching window event.
const QUEUE_CAP: usize = 512;
/// Positions closer than this (logical pixels) count as the same point.
const SAME_POINT: f32 = 0.51;

/// Samples a platform has seen but the window system has not delivered as events yet.
///
/// The platform side pushes samples as it sees them; the shell takes them when winit reports a
/// pointer event. They are matched by position, not by arrival order, so it does not matter whether
/// the platform's observer runs before or after winit handles the same event.
#[derive(Default)]
pub(crate) struct PointerQueue {
    samples: VecDeque<PointerSample>,
    latest: Option<PointerSample>,
}

impl PointerQueue {
    pub fn push(&mut self, sample: PointerSample) {
        if self.samples.len() == QUEUE_CAP {
            self.samples.pop_front();
        }
        self.latest = Some(sample);
        self.samples.push_back(sample);
    }

    /// The samples up to and including the one at `pos` (all of them if none is there) and the
    /// reading that describes the event: the one at `pos`, else the newest known.
    pub fn take(&mut self, pos: (f32, f32)) -> (Vec<PointerSample>, Option<PointerSample>) {
        let at = self.samples.iter().position(|s| (s.pos.0 - pos.0).abs() < SAME_POINT && (s.pos.1 - pos.1).abs() < SAME_POINT);
        let taken: Vec<PointerSample> = match at {
            Some(i) => self.samples.drain(..=i).collect(),
            None => self.samples.drain(..).collect(),
        };
        let current = if at.is_some() { taken.last().copied() } else { self.latest };
        (taken, current)
    }
}

/// Pen input from the operating system. Platforms without an implementation report nothing, which
/// the app sees as a mouse.
#[cfg(target_os = "macos")]
pub(crate) use crate::macos::pen::PenInput;

#[cfg(not(target_os = "macos"))]
pub(crate) struct PenInput;

#[cfg(not(target_os = "macos"))]
impl PenInput {
    pub fn install(_window: &winit::window::Window, _raw_mouse_samples: bool) -> PenInput {
        PenInput
    }

    pub fn take(&mut self, _pos: (f32, f32)) -> (Vec<PointerSample>, Option<PointerSample>) {
        (Vec::new(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_matched_by_position_not_by_arrival_order() {
        let mut q = PointerQueue::default();
        for x in [10., 20., 30.] {
            q.push(PointerSample::pen((x, 5.), x / 100.));
        }
        // The window event for the middle sample arrives first: it gets everything up to it.
        let (taken, current) = q.take((20., 5.));
        assert_eq!(taken.len(), 2);
        assert_eq!(current.unwrap().pressure, 0.2);
        // Then the last one.
        let (taken, current) = q.take((30., 5.));
        assert_eq!((taken.len(), current.unwrap().pressure), (1, 0.3));
        // An event the platform side never saw (or saw after the fact) falls back to the newest reading.
        let (taken, current) = q.take((99., 99.));
        assert!(taken.is_empty());
        assert_eq!(current.unwrap().pos, (30., 5.));
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut q = PointerQueue::default();
        for i in 0..QUEUE_CAP + 10 {
            q.push(PointerSample::mouse((i as f32, 0.), true));
        }
        assert_eq!(q.take((-1., -1.)).0.len(), QUEUE_CAP);
    }

    #[test]
    fn builders_clamp() {
        let s = PointerSample::pen((0., 0.), 1.7).tilt(-3., 0.5).eraser();
        assert_eq!((s.pressure, s.tilt, s.kind), (1., Some((-1., 0.5)), PointerKind::Eraser));
    }
}
