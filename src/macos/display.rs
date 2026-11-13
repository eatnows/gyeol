//! The system's display accessibility options (Reduce Transparency) on macOS.
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};

/// Whether "Reduce transparency" is on (System Settings > Accessibility > Display).
pub fn reduce_transparency() -> bool {
    // SAFETY: a plain getter on the shared workspace.
    unsafe { NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency() }
}

/// A subscription to changes of the display accessibility options; dropping it unsubscribes.
pub(crate) struct DisplayOptionsObserver {
    center: Retained<NSNotificationCenter>,
    token: Retained<NSObject>,
}

impl DisplayOptionsObserver {
    /// Calls `notify` whenever an option changes. The workspace posts on the main thread; `notify`
    /// should only hand the news to the event loop (for example through an `EventLoopProxy`).
    pub fn new(notify: impl Fn() + 'static) -> DisplayOptionsObserver {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| notify());
        // SAFETY: the notification name is AppKit's own static; the block owns what it uses and the
        // observer is removed in `Drop`.
        unsafe {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            let token = center.addObserverForName_object_queue_usingBlock(Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification), None, None, &block);
            DisplayOptionsObserver { center, token }
        }
    }
}

impl Drop for DisplayOptionsObserver {
    fn drop(&mut self) {
        // SAFETY: `token` is the observer this center returned.
        unsafe { self.center.removeObserver(&self.token) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The system setting as `defaults` reports it; the key does not exist until the user changes it, which means off.
    fn defaults_value() -> bool {
        let out = std::process::Command::new("defaults").args(["read", "com.apple.universalaccess", "reduceTransparency"]).output().expect("run defaults");
        String::from_utf8_lossy(&out.stdout).trim() == "1"
    }

    /// Reads the real system setting; run with `cargo test -- --ignored`. Never changes the setting.
    #[test]
    #[ignore = "reads the machine's accessibility settings"]
    fn matches_the_systems_stored_preference() {
        assert_eq!(reduce_transparency(), defaults_value());
    }

    #[test]
    fn subscribing_and_dropping_is_safe_without_a_notification() {
        let seen = std::rc::Rc::new(std::cell::Cell::new(0));
        let s = seen.clone();
        let observer = DisplayOptionsObserver::new(move || s.set(s.get() + 1));
        drop(observer);
        assert_eq!(seen.get(), 0);
    }
}
