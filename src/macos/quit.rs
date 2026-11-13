//! Cmd+Q, the app menu's Quit and the Dock's Quit call `-[NSApplication terminate:]`, which never
//! asks the window (no `windowShouldClose:`, so winit sends no `CloseRequested`). It does ask the
//! application delegate's `applicationShouldTerminate:`, which winit's delegate does not implement.
//! We add that method to winit's delegate class at runtime: it cancels the termination and tells the
//! app, which then decides (and exits the event loop itself when it agrees).
use std::sync::Mutex;

use objc2::{ffi, runtime::{AnyObject, Sel}, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;

static NOTIFY: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// `NSTerminateCancel`: do not terminate now.
const TERMINATE_CANCEL: usize = 0;

extern "C" fn should_terminate(_this: *mut AnyObject, _sel: Sel, _sender: *mut AnyObject) -> usize {
    if let Some(notify) = NOTIFY.lock().ok().as_deref().and_then(|n| n.as_ref()) {
        notify();
        TERMINATE_CANCEL
    } else {
        1 // NSTerminateNow: nobody is listening
    }
}

/// Routes quit requests to `notify` (called on the main thread; hand the news to the event loop).
/// Returns false when the hook could not be installed, in which case quitting behaves as before.
pub(crate) fn install(notify: impl Fn() + Send + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    // SAFETY: the delegate is the one winit registered; we only add a method its class lacks.
    unsafe {
        let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else { return false };
        *NOTIFY.lock().unwrap() = Some(Box::new(notify));
        let imp: unsafe extern "C" fn() = std::mem::transmute(should_terminate as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize);
        let object = &*(objc2::rc::Retained::as_ptr(&delegate) as *const AnyObject);
        let class = object.class();
        let added = ffi::class_addMethod(class as *const _ as *mut _, sel!(applicationShouldTerminate:).as_ptr().cast(), Some(imp), c"Q@:@".as_ptr());
        // `added` is also false when an earlier window already added it, which is just as good.
        added || class.responds_to(sel!(applicationShouldTerminate:))
    }
}
