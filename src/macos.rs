//! macOS window chrome that winit does not expose: traffic-light placement and window vibrancy,
//! plus [`pen`] input.
pub mod pen;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowButton, NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: winit hands out the live NSView of this window; we only borrow it for the call.
    let view: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast())? };
    view.window()
}

/// Puts the close/minimize/zoom buttons `x` points from the window's left edge and `y` points from its
/// top, by growing the titlebar to `button height + 2 * y` and centering the buttons in it. AppKit
/// resets the layout on resize, so call this again after resizes.
pub fn place_traffic_lights(window: &Window, (x, y): (f32, f32)) {
    let Some(ns) = ns_window(window) else { return };
    let button = |b| ns.standardWindowButton(b);
    let (Some(close), Some(mini), Some(zoom)) = (
        button(NSWindowButton::NSWindowCloseButton),
        button(NSWindowButton::NSWindowMiniaturizeButton),
        button(NSWindowButton::NSWindowZoomButton),
    ) else {
        return;
    };
    let Some(titlebar) = (unsafe { close.superview() }) else { return };
    let spacing = mini.frame().origin.x - close.frame().origin.x;
    let height = close.frame().size.height + 2. * y as f64;
    let window_h = ns.frame().size.height;
    let bar = NSRect::new(NSPoint::new(0., window_h - height), NSSize::new(titlebar.frame().size.width, height));
    // The titlebar view sits in a container that spans the same strip; resize both.
    if let Some(container) = unsafe { titlebar.superview() } {
        unsafe { container.setFrame(bar) };
    }
    unsafe { titlebar.setFrame(NSRect::new(NSPoint::new(0., 0.), bar.size)) };
    for (i, b) in [close, mini, zoom].into_iter().enumerate() {
        let at = NSPoint::new(x as f64 + i as f64 * spacing, (height - b.frame().size.height) / 2.);
        unsafe { b.setFrameOrigin(at) };
    }
}

/// Puts a behind-window blur (an NSVisualEffectView) under the content view. Needs a transparent window.
pub fn add_vibrancy(window: &Window) {
    let (Some(ns), Some(mtm)) = (ns_window(window), MainThreadMarker::new()) else { return };
    let Some(content) = ns.contentView() else { return };
    let Some(frame_view) = (unsafe { content.superview() }) else { return };
    // SAFETY: plain AppKit setters on a view we just created, on the main thread (checked via `mtm`).
    unsafe {
        let effect = NSVisualEffectView::new(mtm);
        effect.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        effect.setFrame(content.frame());
        effect.setAutoresizingMask(NSAutoresizingMaskOptions::NSViewWidthSizable | NSAutoresizingMaskOptions::NSViewHeightSizable);
        frame_view.addSubview_positioned_relativeTo(&effect, NSWindowOrderingMode::NSWindowBelow, Some(&content));
    }
}
