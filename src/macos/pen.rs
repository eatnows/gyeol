//! Pen input on macOS, read from NSEvent tablet data.
//!
//! winit turns tablet events into plain mouse events and drops pressure, tilt and the pen/eraser
//! distinction, so this module installs an `NSEvent` *local monitor*: AppKit shows every mouse and
//! tablet-proximity event to the monitor before it is dispatched to the window, and the monitor
//! returns the event unchanged so winit handles it as usual. Readings are queued as
//! [`PointerSample`]s and the shell hands them to the app next to winit's own event, matched by
//! position (see [`PointerQueue`]), so the order of monitor and winit does not matter.
//!
//! Apple documents tablet data as mouse events whose subtype is `tabletPoint`; the pen or eraser is
//! announced by separate `tabletProximity` events, which are remembered until the next one.
use std::{cell::{Cell, RefCell}, ptr::NonNull, rc::Rc};

use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventSubtype, NSEventType, NSPointingDeviceType, NSView};
use objc2_foundation::MainThreadMarker;
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::pointer::{PointerKind, PointerQueue, PointerSample};

pub(crate) struct PenInput {
    queue: Rc<RefCell<PointerQueue>>,
    monitor: Option<Retained<AnyObject>>,
}

impl PenInput {
    /// Starts watching `window`'s mouse and tablet events. With `raw_mouse_samples`, AppKit's merging of
    /// mouse movement into one event per frame is switched off (for the whole app).
    pub fn install(window: &Window, raw_mouse_samples: bool) -> PenInput {
        let queue = Rc::new(RefCell::new(PointerQueue::default()));
        let mut pen = PenInput { queue: queue.clone(), monitor: None };
        let (Some(_mtm), Ok(handle)) = (MainThreadMarker::new(), window.window_handle()) else { return pen };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return pen };
        // SAFETY: winit hands out the live NSView of this window; retaining it keeps it alive for the monitor.
        let Some(view): Option<Retained<NSView>> = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }) else { return pen };
        if raw_mouse_samples {
            // SAFETY: a plain class-level setter, called on the main thread.
            unsafe { NSEvent::setMouseCoalescingEnabled(false) };
        }
        let device = Cell::new(PointerKind::Mouse);
        let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a live event for the duration of the call.
            if let Some(sample) = unsafe { read(event.as_ref(), &view, &device) } {
                queue.borrow_mut().push(sample);
            }
            event.as_ptr() // unchanged: winit still gets it
        });
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseUp
            | NSEventMask::LeftMouseDragged
            | NSEventMask::RightMouseDown
            | NSEventMask::RightMouseUp
            | NSEventMask::RightMouseDragged
            | NSEventMask::OtherMouseDown
            | NSEventMask::OtherMouseUp
            | NSEventMask::OtherMouseDragged
            | NSEventMask::MouseMoved
            | NSEventMask::TabletProximity;
        // SAFETY: the block only touches state it owns; it is removed again in `Drop`.
        pen.monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        pen
    }

    /// Pen data for the winit pointer event at `pos`.
    pub fn take(&mut self, pos: (f32, f32)) -> (Vec<PointerSample>, Option<PointerSample>) {
        self.queue.borrow_mut().take(pos)
    }
}

impl Drop for PenInput {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            // SAFETY: `monitor` came from `addLocalMonitor...` and is removed once.
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

/// Reads one event; `device` remembers what the last proximity event said is near the tablet.
///
/// # Safety
/// `event` must be a live event of one of the types in the monitor's mask.
unsafe fn read(event: &NSEvent, view: &NSView, device: &Cell<PointerKind>) -> Option<PointerSample> {
    let ty = unsafe { event.r#type() };
    if ty == NSEventType::TabletProximity {
        device.set(if unsafe { event.isEnteringProximity() } { kind_of(unsafe { event.pointingDeviceType() }) } else { PointerKind::Mouse });
        return None;
    }
    // Only our own window's events; anything else (other windows, the titlebar) is not ours.
    let mtm = MainThreadMarker::new()?;
    let ours = unsafe { event.window(mtm) }.is_some_and(|w| view.window().is_some_and(|v| Retained::as_ptr(&w) == Retained::as_ptr(&v)));
    if !ours {
        return None;
    }
    let subtype = unsafe { event.subtype() };
    if subtype == NSEventSubtype::TabletProximity {
        device.set(if unsafe { event.isEnteringProximity() } { kind_of(unsafe { event.pointingDeviceType() }) } else { PointerKind::Mouse });
        return None;
    }
    let tablet = subtype == NSEventSubtype::TabletPoint;
    let point = view.convertPoint_fromView(unsafe { event.locationInWindow() }, None);
    let bounds = view.bounds();
    let pos = top_left((point.x, point.y), bounds.size.height, view.isFlipped());
    let (pressed, released) = (is_down_or_drag(ty), is_up(ty));
    Some(build(Raw {
        pos,
        pressed,
        released,
        tablet,
        kind: device.get(),
        pressure: if tablet { unsafe { event.pressure() } } else { 0. },
        tilt: if tablet { Some(unsafe { event.tilt() }).map(|p| (p.x, p.y)) } else { None }, // NSPoint -> (x, y)
        rotation: if tablet { Some(unsafe { event.rotation() }) } else { None },
        time: unsafe { event.timestamp() },
    }))
}

fn is_down_or_drag(ty: NSEventType) -> bool {
    [NSEventType::LeftMouseDown, NSEventType::LeftMouseDragged, NSEventType::RightMouseDown, NSEventType::RightMouseDragged, NSEventType::OtherMouseDown, NSEventType::OtherMouseDragged]
        .contains(&ty)
}

fn is_up(ty: NSEventType) -> bool {
    [NSEventType::LeftMouseUp, NSEventType::RightMouseUp, NSEventType::OtherMouseUp].contains(&ty)
}

fn kind_of(device: NSPointingDeviceType) -> PointerKind {
    if device == NSPointingDeviceType::Eraser {
        PointerKind::Eraser
    } else if device == NSPointingDeviceType::Cursor {
        PointerKind::Mouse // a tablet puck behaves as a mouse
    } else {
        PointerKind::Pen
    }
}

/// A point in the view's coordinates as logical pixels from the top-left. A flipped view (winit's is)
/// already has its origin there; otherwise y counts up from the bottom.
fn top_left((x, y): (f64, f64), view_height: f64, flipped: bool) -> (f32, f32) {
    (x as f32, if flipped { y } else { view_height - y } as f32)
}

/// What one event said, as plain numbers.
struct Raw {
    pos: (f32, f32),
    pressed: bool,
    released: bool,
    tablet: bool,
    kind: PointerKind,
    pressure: f32,
    tilt: Option<(f64, f64)>,
    rotation: Option<f32>,
    time: f64,
}

/// Turns one event into a sample. Tablet events keep the device's pressure (0 on release); everything
/// else is a mouse with pressure 1 while a button is down. NSEvent tilt is -1..1 already, with y
/// positive towards the user (the bottom of the screen); it is clamped, and NaN becomes 0.
fn build(raw: Raw) -> PointerSample {
    let clean = |v: f64| if v.is_nan() { 0. } else { v.clamp(-1., 1.) as f32 };
    let pressure = if raw.released {
        0.
    } else if raw.tablet {
        raw.pressure.clamp(0., 1.)
    } else if raw.pressed {
        1.
    } else {
        0.
    };
    PointerSample {
        pos: raw.pos,
        pressure,
        tilt: raw.tilt.filter(|_| raw.tablet).map(|(x, y)| (clean(x), clean(y))),
        rotation: raw.rotation.filter(|_| raw.tablet),
        kind: if raw.tablet && raw.kind == PointerKind::Mouse { PointerKind::Pen } else if raw.tablet { raw.kind } else { PointerKind::Mouse },
        time: raw.time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> Raw {
        Raw { pos: (1., 2.), pressed: false, released: false, tablet: true, kind: PointerKind::Pen, pressure: 0.5, tilt: Some((0.25, -0.5)), rotation: Some(90.), time: 3. }
    }

    #[test]
    fn coordinates_are_top_left_in_logical_pixels() {
        assert_eq!(top_left((10., 20.), 600., true), (10., 20.), "winit's view is flipped: already top-left");
        assert_eq!(top_left((10., 20.), 600., false), (10., 580.), "an unflipped view counts y from the bottom");
    }

    #[test]
    fn tilt_is_clamped_to_unit_range_and_nan_is_zero() {
        let s = build(Raw { tilt: Some((3., f64::NAN)), ..raw() });
        assert_eq!(s.tilt, Some((1., 0.)));
        assert_eq!(build(Raw { tilt: Some((-9., -0.5)), ..raw() }).tilt, Some((-1., -0.5)));
        assert_eq!(build(raw()).tilt, Some((0.25, -0.5)));
    }

    #[test]
    fn tablet_events_keep_pressure_and_kind_and_releases_are_zero() {
        let s = build(Raw { pressed: true, pressure: 0.8, ..raw() });
        assert_eq!((s.pressure, s.kind, s.rotation, s.time), (0.8, PointerKind::Pen, Some(90.), 3.));
        assert_eq!(build(Raw { pressure: 7., ..raw() }).pressure, 1., "pressure is clamped");
        assert_eq!(build(Raw { released: true, pressure: 0.9, ..raw() }).pressure, 0.);
        let eraser = build(Raw { kind: PointerKind::Eraser, ..raw() });
        assert_eq!(eraser.kind, PointerKind::Eraser);
        // A tablet event with no proximity seen yet is still a pen.
        assert_eq!(build(Raw { kind: PointerKind::Mouse, ..raw() }).kind, PointerKind::Pen);
    }

    #[test]
    fn plain_mouse_events_are_a_mouse_with_full_pressure_while_down() {
        let mouse = Raw { tablet: false, kind: PointerKind::Pen, pressure: 0.3, tilt: Some((1., 1.)), rotation: Some(5.), ..raw() };
        let down = build(Raw { pressed: true, ..mouse });
        assert_eq!((down.pressure, down.kind, down.tilt, down.rotation), (1., PointerKind::Mouse, None, None));
        let hover = build(Raw { tablet: false, kind: PointerKind::Pen, pressure: 0.3, tilt: None, rotation: None, ..raw() });
        assert_eq!(hover.pressure, 0.);
        let up = build(Raw { tablet: false, released: true, ..raw() });
        assert_eq!(up.pressure, 0.);
    }

    #[test]
    fn pointing_device_types_map_to_kinds() {
        assert_eq!(kind_of(NSPointingDeviceType::Pen), PointerKind::Pen);
        assert_eq!(kind_of(NSPointingDeviceType::Eraser), PointerKind::Eraser);
        assert_eq!(kind_of(NSPointingDeviceType::Cursor), PointerKind::Mouse);
        assert_eq!(kind_of(NSPointingDeviceType::Unknown), PointerKind::Pen);
    }
}
