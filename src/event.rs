//! Input events, independent of the windowing library.

/// Which modifier keys are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Cmd on macOS, the Windows key elsewhere.
    pub logo: bool,
}

impl Modifiers {
    /// The platform's shortcut modifier: Cmd on macOS, Ctrl elsewhere.
    pub fn command(self) -> bool {
        if cfg!(target_os = "macos") { self.logo } else { self.ctrl }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// How far the wheel or trackpad moved. Positive `y` scrolls content up (towards its start).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// Notched wheels report lines.
    Lines(f32, f32),
    /// Trackpads report logical pixels.
    Pixels(f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that produces text, as the layout labels it (`"a"`, `"1"`, `" "`).
    Char(String),
    Named(NamedKey),
    Other,
}

/// Input-method (IME) composition, e.g. Korean syllables being assembled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    /// The text being composed, shown provisionally. Empty means the composition ended.
    /// `cursor` is a byte range inside `text`, if the input method reports one.
    Preedit { text: String, cursor: Option<(usize, usize)> },
    /// Finished text to insert.
    Commit(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    MouseMoved { pos: (f32, f32) },
    MousePressed { button: MouseButton, pos: (f32, f32), click_count: u32 },
    MouseReleased { button: MouseButton, pos: (f32, f32) },
    Scroll { delta: ScrollDelta, pos: (f32, f32) },
    KeyDown {
        key: Key,
        /// Text this key press types (never control characters). Ignore it while a shortcut modifier is held.
        text: Option<String>,
        repeat: bool,
    },
    KeyUp { key: Key },
    Ime(Ime),
    /// The window gained or lost keyboard focus.
    FocusChanged(bool),
}
