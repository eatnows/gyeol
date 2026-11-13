//! macOS window chrome that winit does not expose: traffic-light placement and window vibrancy,
//! plus [`pen`] input.
pub mod display;
pub mod pen;

use objc2::rc::Retained;
use crate::shell::{Appearance, Material};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
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

fn ns_material(material: Material) -> NSVisualEffectMaterial {
    match material {
        Material::UnderWindowBackground => NSVisualEffectMaterial::UnderWindowBackground,
        Material::HudWindow => NSVisualEffectMaterial::HUDWindow,
        Material::Sidebar => NSVisualEffectMaterial::Sidebar,
        Material::Popover => NSVisualEffectMaterial::Popover,
        Material::Menu => NSVisualEffectMaterial::Menu,
        Material::FullScreenUI => NSVisualEffectMaterial::FullScreenUI,
        Material::UnderPageBackground => NSVisualEffectMaterial::UnderPageBackground,
        Material::Titlebar => NSVisualEffectMaterial::Titlebar,
    }
}

/// Forces the window to the dark or light Aqua appearance; `System` leaves it following the system.
pub fn set_appearance(window: &Window, appearance: Appearance) {
    let Some(ns) = ns_window(window) else { return };
    // SAFETY: the appearance names are AppKit's own statics, and setAppearance is a plain setter.
    let named = unsafe {
        match appearance {
            Appearance::System => return,
            Appearance::Dark => NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua),
            Appearance::Light => NSAppearance::appearanceNamed(NSAppearanceNameAqua),
        }
    };
    unsafe { ns.setAppearance(named.as_deref()) };
}

/// Puts a behind-window blur (an NSVisualEffectView) under the content view. Needs a transparent window.
pub fn add_vibrancy(window: &Window, material: Material) {
    let (Some(ns), Some(mtm)) = (ns_window(window), MainThreadMarker::new()) else { return };
    let Some(content) = ns.contentView() else { return };
    let Some(frame_view) = (unsafe { content.superview() }) else { return };
    // SAFETY: plain AppKit setters on a view we just created, on the main thread (checked via `mtm`).
    unsafe {
        let effect = NSVisualEffectView::new(mtm);
        effect.setMaterial(ns_material(material));
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        // Active, not FollowsWindowActiveState: the glass must not go flat gray when the window loses focus.
        effect.setState(NSVisualEffectState::Active);
        effect.setEmphasized(false);
        effect.setFrame(content.frame());
        effect.setAutoresizingMask(NSAutoresizingMaskOptions::NSViewWidthSizable | NSAutoresizingMaskOptions::NSViewHeightSizable);
        frame_view.addSubview_positioned_relativeTo(&effect, NSWindowOrderingMode::NSWindowBelow, Some(&content));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_material_maps_to_its_own_appkit_material() {
        use Material::*;
        let all = [UnderWindowBackground, HudWindow, Sidebar, Popover, Menu, FullScreenUI, UnderPageBackground, Titlebar];
        let mapped: Vec<isize> = all.iter().map(|m| ns_material(*m).0).collect();
        assert_eq!(mapped, [21, 13, 7, 6, 5, 15, 22, 3], "AppKit's raw values");
        assert_eq!(Material::default(), UnderWindowBackground, "the default keeps the old look");
        assert_eq!(Appearance::default(), Appearance::System);
    }
}
