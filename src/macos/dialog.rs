//! Native open/save panels (NSOpenPanel, NSSavePanel), run modally on the main thread.
use std::path::PathBuf;

use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};

/// The extensions of all `filters`, without dots; empty means "any file".
fn extensions(filters: &[(&str, &[&str])]) -> Vec<String> {
    filters.iter().flat_map(|(_, exts)| exts.iter()).map(|e| e.trim_start_matches('.').to_string()).collect()
}

fn restrict(panel: &NSSavePanel, filters: &[(&str, &[&str])]) {
    let exts = extensions(filters);
    if exts.is_empty() {
        return;
    }
    let exts: Vec<_> = exts.iter().map(|e| NSString::from_str(e)).collect();
    // `allowedFileTypes` is deprecated in favor of UTTypes but takes plain extensions, which is all
    // the API offers; it still works on current macOS.
    #[allow(deprecated)]
    // SAFETY: an array of strings on a live panel, on the main thread.
    unsafe {
        panel.setAllowedFileTypes(Some(&NSArray::from_vec(exts)));
    }
}

fn path_of(url: &NSURL) -> Option<PathBuf> {
    // SAFETY: a plain getter.
    unsafe { url.path() }.map(|p| PathBuf::from(p.to_string()))
}

/// Shows the open panel and returns the chosen file; `None` when cancelled or off the main thread.
pub fn pick_file(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: AppKit calls on the main thread with a panel we own.
    unsafe {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        restrict(&panel, filters);
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        path_of(&*panel.URLs().firstObject()?)
    }
}

/// Shows the save panel, pre-filled with `default_name`; `None` when cancelled or off the main thread.
pub fn pick_save_path(default_name: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: AppKit calls on the main thread with a panel we own.
    unsafe {
        let panel = NSSavePanel::savePanel(mtm);
        panel.setCanCreateDirectories(true);
        panel.setNameFieldStringValue(&NSString::from_str(default_name));
        restrict(&panel, filters);
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        path_of(&*panel.URL()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_flattened_and_lose_their_dots() {
        let f: &[(&str, &[&str])] = &[("Images", &["png", ".jpg"]), ("Data", &["json"])];
        assert_eq!(extensions(f), ["png", "jpg", "json"]);
        assert!(extensions(&[]).is_empty());
    }
}
