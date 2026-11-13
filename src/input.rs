//! A single-line text field: [`TextInput`] holds the editing state and [`text_input`] draws it.
//!
//! The app keeps one [`TextInput`] per field in its state. Drawing and mouse handling go through the
//! element returned by [`text_input`]; keyboard and input-method events are forwarded by the app from
//! [`View::event`](crate::View::event) to [`TextInput::event`], which also reports edits and Enter:
//!
//! ```ignore
//! struct Form { name: TextInput, note: TextInput }
//!
//! impl View for Form {
//!     fn view(&self, cx: &mut Cx) -> Element<Self> {
//!         let style = InputStyle::new(16.).placeholder("Name");
//!         div().px(12.).py(8.).bg(white).rounded(8.)
//!             .child(text_input(&self.name, &style, cx, |s: &mut Form| &mut s.name))
//!         // ... a second text_input for `note`
//!     }
//!     fn event(&mut self, event: &Event, cx: &mut Cx) {
//!         if self.name.event(event, cx) == InputChange::Submitted { /* Enter pressed */ }
//!         self.note.event(event, cx);
//!     }
//! }
//! ```
//!
//! **Focus.** Each state has its own `focused` flag. Pressing a field focuses it; pressing anywhere
//! else (the fields' outside-press handlers) blurs it, so at most one field is focused. Keys only
//! reach the focused field. Tab and Escape are reported as [`InputChange::Tab`] / [`InputChange::Escape`]
//! for the app to act on (see [`cycle_focus`]).
use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::{
    element::{div, text, Element},
    event::{Event, Ime, Key, Modifiers, NamedKey},
    scene::{Color, Rect},
    shaper::{Shaper, TextStyle},
    shell::Cx,
};

const BLINK: Duration = Duration::from_millis(530);
/// Room after the text so the caret at the end is not clipped.
const CARET_ROOM: f32 = 2.;

/// What a masked field draws for each character.
const MASK: char = '•';

/// What an event did to a field, for the app to react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputChange {
    /// Nothing the app needs to know about (the caret may still have moved).
    None,
    /// The text changed (typing, deleting, pasting, committing a composition).
    Edited,
    /// Enter was pressed in the focused field.
    Submitted,
    /// Tab was pressed; `back` when Shift was held. Move focus with [`cycle_focus`].
    Tab { back: bool },
    /// Escape was pressed in the focused field.
    Escape,
}

/// The state of one single-line text field: text, caret and selection, the input method's
/// provisional text, and whether the field has keyboard focus.
///
/// All offsets are byte indices on `char` boundaries. Editing methods work without a window, so they
/// can be tested headlessly; [`TextInput::event`] maps real events onto them.
pub struct TextInput {
    id: u64,
    text: String,
    caret: usize,
    anchor: usize,
    preedit: String,
    focused: bool,
    window_active: bool,
    epoch: Instant,
    revision: u64,
    /// The look the field was last drawn with; events need it to measure text.
    style: Cell<TextStyle>,
    ime_enabled: Cell<bool>,
    masked: bool,
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInput {
    pub fn new() -> TextInput {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        TextInput {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            text: String::new(),
            caret: 0,
            anchor: 0,
            preedit: String::new(),
            focused: false,
            window_active: true,
            epoch: Instant::now(),
            revision: 0,
            style: Cell::new(TextStyle::new(14.)),
            ime_enabled: Cell::new(false),
            masked: false,
        }
    }

    pub fn with_text(text: impl Into<String>) -> TextInput {
        let mut input = Self::new();
        input.set_text(text);
        input
    }

    // ---- masking -----------------------------------------------------------------------------

    /// Turns password-style masking on or off (off by default). While masked the field draws one
    /// `•` per character and never puts its text on the clipboard ([`copy`](Self::copy) and
    /// [`cut`](Self::cut) do nothing; paste still works). [`text`](Self::text) stays the plain text,
    /// and the caret, selection, clicks and composition all follow the masked display.
    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// `s` as drawn: itself, or one bullet per character when masked.
    fn display(&self, s: &str) -> String {
        if self.masked { MASK.to_string().repeat(s.chars().count()) } else { s.to_string() }
    }

    /// The byte offset in the drawn text of the plain-text byte offset `byte`.
    fn to_display(&self, byte: usize) -> usize {
        if self.masked { self.text[..byte.min(self.text.len())].chars().count() * MASK.len_utf8() } else { byte }
    }

    /// The plain-text byte offset (on a char boundary) for a byte offset in the drawn text.
    fn from_display(&self, shown: usize) -> usize {
        if !self.masked {
            return shown;
        }
        self.text.char_indices().nth(shown / MASK.len_utf8()).map_or(self.text.len(), |(i, _)| i)
    }

    /// The drawn text without the composition, for mapping clicks.
    fn display_text(&self) -> String {
        self.display(&self.text)
    }

    // ---- reading -----------------------------------------------------------------------------

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The caret's byte index.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range `(start, end)`; empty (`start == end`) when nothing is selected.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }

    /// The text an input method is composing, shown at the caret but not yet part of [`TextInput::text`].
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    pub fn is_composing(&self) -> bool {
        !self.preedit.is_empty()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Counts text changes; compare two readings to see whether the text changed in between.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    // ---- focus -------------------------------------------------------------------------------

    /// Gives this field keyboard focus. Blur the others yourself when focusing from code; pressing a
    /// field does that automatically.
    pub fn focus(&mut self) {
        self.focused = true;
        self.restart_blink();
    }

    /// Takes focus away and abandons an unfinished composition.
    pub fn blur(&mut self) {
        self.focused = false;
        self.preedit.clear();
        self.ime_enabled.set(false);
    }

    fn restart_blink(&mut self) {
        self.epoch = Instant::now();
    }

    // ---- editing -----------------------------------------------------------------------------

    /// Replaces the text; the caret goes to the end and the selection and composition are dropped.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = single_line(&text.into());
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.preedit.clear();
        self.revision += 1;
    }

    pub fn clear(&mut self) {
        self.set_text("");
    }

    fn replace(&mut self, a: usize, b: usize, with: &str) {
        self.text.replace_range(a..b, with);
        self.caret = a + with.len();
        self.anchor = self.caret;
        self.revision += 1;
    }

    /// Types `s` at the caret, replacing the selection. Line breaks and other control characters are dropped.
    pub fn insert(&mut self, s: &str) {
        let s = single_line(s);
        let (a, b) = self.selection();
        if s.is_empty() && a == b {
            return;
        }
        self.replace(a, b, &s);
    }

    /// Deletes the selection; false when there was none.
    pub fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        self.replace(a, b, "");
        true
    }

    /// Backspace: the selection, else the character (or word) before the caret.
    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.caret == 0 {
            return;
        }
        let from = if word { self.word_start(self.caret) } else { self.prev(self.caret) };
        self.replace(from, self.caret, "");
    }

    /// Delete: the selection, else the character (or word) after the caret.
    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() || self.caret >= self.text.len() {
            return;
        }
        let to = if word { self.word_end(self.caret) } else { self.next(self.caret) };
        self.replace(self.caret, to, "");
    }

    /// Deletes everything before the caret (Cmd+Backspace).
    pub fn delete_to_start(&mut self) {
        if !self.delete_selection() {
            self.replace(0, self.caret, "");
        }
    }

    // ---- moving and selecting ----------------------------------------------------------------

    /// Puts the caret at byte `index` (clamped to a character boundary). With `extend` the selection
    /// grows from where it started, otherwise it collapses.
    pub fn move_to(&mut self, index: usize, extend: bool) {
        let mut index = index.min(self.text.len());
        while !self.text.is_char_boundary(index) {
            index -= 1;
        }
        self.caret = index;
        if !extend {
            self.anchor = index;
        }
    }

    /// Left arrow. Without `extend`, a selection collapses to its start first.
    pub fn move_left(&mut self, word: bool, extend: bool) {
        let to = if word {
            self.word_start(self.caret)
        } else if self.has_selection() && !extend {
            self.selection().0
        } else {
            self.prev(self.caret)
        };
        self.move_to(to, extend);
    }

    /// Right arrow. Without `extend`, a selection collapses to its end first.
    pub fn move_right(&mut self, word: bool, extend: bool) {
        let to = if word {
            self.word_end(self.caret)
        } else if self.has_selection() && !extend {
            self.selection().1
        } else {
            self.next(self.caret)
        };
        self.move_to(to, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Selects the word around byte `index` (double click).
    pub fn select_word_at(&mut self, index: usize) {
        let (a, b) = self.word_at(index);
        self.anchor = a;
        self.caret = b;
    }

    /// The word around `index`; just after a word still picks that word, in whitespace it is empty.
    pub fn word_at(&self, index: usize) -> (usize, usize) {
        let on_word = |i: usize| self.text[i..].chars().next().is_some_and(is_word);
        let start = if index > 0 && !on_word(index) { self.prev(index) } else { index };
        if !on_word(start) {
            return (index, index);
        }
        let mut a = start;
        while a > 0 && is_word(self.text[..a].chars().next_back().unwrap()) {
            a = self.prev(a);
        }
        let mut b = start;
        while b < self.text.len() && is_word(self.text[b..].chars().next().unwrap()) {
            b = self.next(b);
        }
        (a, b)
    }

    /// The boundary before `from`, stepping over a whole cluster (a base character with its
    /// combining marks, or a decomposed Hangul syllable).
    fn prev(&self, from: usize) -> usize {
        let mut chars = self.text[..from].char_indices().rev();
        let Some((mut i, _)) = chars.next() else { return 0 };
        while self.text[i..].chars().next().is_some_and(extends) {
            match chars.next() {
                Some((j, _)) => i = j,
                None => break,
            }
        }
        i
    }

    fn next(&self, from: usize) -> usize {
        let mut chars = self.text[from..].char_indices();
        if chars.next().is_none() {
            return from;
        }
        for (i, c) in chars {
            if !extends(c) {
                return from + i;
            }
        }
        self.text.len()
    }

    fn word_start(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && !is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        while i > 0 && is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        i
    }

    fn word_end(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.text.len() && !is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        while i < self.text.len() && is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        i
    }

    // ---- clipboard ---------------------------------------------------------------------------

    /// Copies the selection to the clipboard (nothing when empty, and nothing while masked).
    pub fn copy(&self, cx: &Cx) {
        if self.has_selection() && !self.masked {
            cx.set_clipboard_text(self.selected_text());
        }
    }

    /// Copies the selection, then deletes it. Does nothing while masked.
    pub fn cut(&mut self, cx: &Cx) {
        if self.masked {
            return;
        }
        self.copy(cx);
        self.delete_selection();
    }

    /// Inserts the clipboard's text, replacing the selection.
    pub fn paste(&mut self, cx: &Cx) {
        if let Some(t) = cx.clipboard_text() {
            self.insert(&t);
        }
    }

    // ---- input method ------------------------------------------------------------------------

    /// Input-method composition: provisional text is shown at the caret and only a commit changes the
    /// text. Starting a composition replaces the selection, as the commit would.
    pub fn ime(&mut self, ime: Ime) {
        match ime {
            Ime::Preedit { text, .. } => {
                if self.preedit.is_empty() && !text.is_empty() {
                    self.delete_selection();
                }
                self.preedit = single_line(&text);
            }
            Ime::Commit(text) => {
                self.preedit.clear();
                self.insert(&text);
            }
        }
    }

    // ---- keyboard ----------------------------------------------------------------------------

    /// Keyboard editing. `text` is what the key types. While an input method is composing it owns
    /// the keyboard, so keys are ignored here.
    pub fn key(&mut self, key: &Key, text: Option<&str>, mods: Modifiers, cx: &Cx) -> InputChange {
        if self.is_composing() {
            return InputChange::None;
        }
        let before = self.revision;
        let word = if cfg!(target_os = "macos") { mods.alt } else { mods.ctrl };
        let shift = mods.shift;
        match key {
            Key::Named(NamedKey::Enter) => return InputChange::Submitted,
            Key::Named(NamedKey::Tab) => return InputChange::Tab { back: shift },
            Key::Named(NamedKey::Escape) => return InputChange::Escape,
            Key::Named(NamedKey::Backspace) if mods.command() => self.delete_to_start(),
            Key::Named(NamedKey::Backspace) => self.backspace(word),
            Key::Named(NamedKey::Delete) => self.delete(word),
            Key::Named(NamedKey::ArrowLeft) if mods.command() => self.move_home(shift),
            Key::Named(NamedKey::ArrowRight) if mods.command() => self.move_end(shift),
            Key::Named(NamedKey::ArrowLeft) => self.move_left(word, shift),
            Key::Named(NamedKey::ArrowRight) => self.move_right(word, shift),
            Key::Named(NamedKey::Home | NamedKey::ArrowUp) => self.move_home(shift),
            Key::Named(NamedKey::End | NamedKey::ArrowDown) => self.move_end(shift),
            Key::Char(c) if mods.command() => match c.to_ascii_lowercase().as_str() {
                "a" => self.select_all(),
                "c" => self.copy(cx),
                "x" => self.cut(cx),
                "v" => self.paste(cx),
                _ => {}
            },
            _ if mods.command() => {}
            _ => {
                if let Some(t) = text {
                    self.insert(t);
                }
            }
        }
        if self.revision != before { InputChange::Edited } else { InputChange::None }
    }

    /// Feeds one event to the field; call it from [`View::event`](crate::View::event) for every field.
    ///
    /// Keys and input-method events only affect a focused field. Mouse events are handled by the
    /// element from [`text_input`], so they are ignored here. The caret is scrolled into view.
    pub fn event(&mut self, event: &Event, cx: &mut Cx) -> InputChange {
        if let Event::FocusChanged(active) = event {
            self.window_active = *active;
            self.restart_blink();
            return InputChange::None;
        }
        if !self.focused {
            return InputChange::None;
        }
        let before = self.revision;
        let change = match event {
            Event::KeyDown { key, text, .. } => self.key(key, text.as_deref(), cx.modifiers, cx),
            Event::Ime(ime) => {
                self.ime(ime.clone());
                if self.revision != before { InputChange::Edited } else { InputChange::None }
            }
            _ => return InputChange::None,
        };
        self.restart_blink();
        self.reveal_caret(cx);
        change
    }

    // ---- drawing support ---------------------------------------------------------------------

    fn scroll_id(&self) -> (&'static str, u64) {
        ("gyeol-text-input", self.id)
    }

    /// What is drawn: the text with the composition spliced in at the caret.
    fn shown(&self) -> String {
        let mut shown = self.display(&self.text);
        shown.insert_str(self.to_display(self.caret), &self.display(&self.preedit));
        shown
    }

    fn caret_x(&self, shaper: &mut Shaper) -> f32 {
        shaper.caret_x(&self.shown(), self.style.get(), self.to_display(self.caret) + self.display(&self.preedit).len())
    }

    /// Scrolls the field horizontally so the caret is visible once the next frame is laid out.
    fn reveal_caret(&self, cx: &mut Cx) {
        let x = self.caret_x(cx.shaper);
        let h = Shaper::line_height(self.style.get().size);
        cx.scroll_to_reveal(self.scroll_id(), Rect::new(x, 0., CARET_ROOM, h));
    }

    fn caret_on(&self, blink: bool) -> bool {
        self.focused && self.window_active && (!blink || (self.epoch.elapsed().as_millis() / BLINK.as_millis()) % 2 == 0)
    }
}

/// Moves focus between `fields` as Tab does: to the next (or previous when `back`) after the
/// focused one; the first (or last) when none is focused.
pub fn cycle_focus(fields: &mut [&mut TextInput], back: bool) {
    let n = fields.len();
    if n == 0 {
        return;
    }
    let current = fields.iter().position(|f| f.focused);
    let next = match (current, back) {
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
        (None, false) => 0,
        (None, true) => n - 1,
    };
    for (i, f) in fields.iter_mut().enumerate() {
        if i == next {
            f.focus();
        } else if f.focused {
            f.blur();
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Characters that continue the previous one: combining marks, variation selectors, a joiner, and
/// the vowels and final consonants of decomposed Hangul syllables.
fn extends(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36f}' | '\u{1160}'..='\u{11ff}' | '\u{302e}' | '\u{302f}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}')
}

fn single_line(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// How a [`text_input`] looks. Background, border and padding belong to the element you wrap it in.
#[derive(Clone, Debug)]
pub struct InputStyle {
    pub font: TextStyle,
    pub color: Color,
    pub placeholder: String,
    pub placeholder_color: Color,
    pub selection: Color,
    pub caret: Color,
    /// Blink the caret (needs redraws every half second while focused). Off keeps it solid.
    pub blink: bool,
}

impl InputStyle {
    pub fn new(size: f32) -> InputStyle {
        InputStyle {
            font: TextStyle::new(size),
            color: Color::hex(0x14202b),
            placeholder: String::new(),
            placeholder_color: Color::hex(0x8a96a3),
            selection: Color::hex(0x3b82f6).with_alpha(0.3),
            caret: Color::hex(0x14202b),
            blink: true,
        }
    }

    pub fn font(mut self, font: TextStyle) -> Self {
        self.font = font;
        self
    }

    /// The text color; the caret follows it unless set separately afterwards.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self.caret = color;
        self
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn placeholder_color(mut self, color: Color) -> Self {
        self.placeholder_color = color;
        self
    }

    pub fn selection(mut self, color: Color) -> Self {
        self.selection = color;
        self
    }

    pub fn caret(mut self, color: Color) -> Self {
        self.caret = color;
        self
    }

    pub fn blink(mut self, on: bool) -> Self {
        self.blink = on;
        self
    }
}

/// Draws `input` as one line of text with its selection, caret, composition underline and
/// placeholder, scrolling sideways when the text is wider than the element. The element is as tall as
/// a line; give it a width (`.w(..)`, `.grow()`, or stretch it in a column) and wrap it in a styled
/// `div` for background, border and padding.
///
/// `field` finds this input inside the app state, for the mouse handlers: pressing focuses the field
/// and puts the caret there (shift extends, double click selects a word, triple click everything),
/// dragging selects, and pressing elsewhere blurs it.
pub fn text_input<S: 'static>(input: &TextInput, style: &InputStyle, cx: &mut Cx, field: impl Fn(&mut S) -> &mut TextInput + 'static) -> Element<S> {
    input.style.set(style.font);
    let font = style.font;
    let line_h = Shaper::line_height(font.size);
    let id = input.scroll_id();
    let shown = input.shown();
    let text_w = cx.shaper.width(&shown, font);
    let caret_x = input.caret_x(cx.shaper);

    let mut content = div().w(text_w + CARET_ROOM).h(line_h);
    if input.is_composing() {
        // The composition is underlined; nothing else is selected while it runs.
        let x0 = cx.shaper.caret_x(&shown, font, input.to_display(input.caret));
        content = content.child(div().absolute().left(x0).top(line_h - 3.).w((caret_x - x0).max(0.)).h(1.5).bg(style.color));
    } else if input.focused && input.has_selection() {
        let (a, b) = input.selection();
        let (x0, x1) = (cx.shaper.caret_x(&shown, font, input.to_display(a)), cx.shaper.caret_x(&shown, font, input.to_display(b)));
        content = content.child(div().absolute().left(x0).top(0.).w(x1 - x0).h(line_h).bg(style.selection).rounded(2.));
    }
    if shown.is_empty() {
        if !style.placeholder.is_empty() {
            content = content.child(text(style.placeholder.clone()).text_color(style.placeholder_color).absolute().left(0.).top(0.));
        }
    } else {
        content = content.child(text(shown));
    }
    if input.caret_on(style.blink) && (input.is_composing() || !input.has_selection()) {
        content = content.child(div().absolute().left(caret_x).top(2.).w(1.5).h(line_h - 4.).bg(style.caret));
    }

    if input.focused {
        if !input.ime_enabled.replace(true) {
            cx.set_ime_allowed(true);
        }
        if let Some(b) = cx.bounds(id) {
            let scroll = cx.scroll_offset(id).0;
            cx.set_ime_cursor_area(Rect::new(b.x + caret_x - scroll, b.y, 1., line_h));
        }
        if style.blink && input.window_active {
            let period = BLINK.as_millis();
            cx.request_redraw_after(Duration::from_millis((period - input.epoch.elapsed().as_millis() % period) as u64));
        }
    }

    let field = Rc::new(field);
    let (down, drag, outside) = (field.clone(), field.clone(), field);
    let mut root = div()
        .id(id)
        .h(line_h)
        .overflow_x_scroll()
        .cursor(crate::element::Cursor::Text)
        .text_size(font.size)
        .text_color(style.color)
        .on_mouse_down(move |s: &mut S, cx, e| {
            let input = down(s);
            input.focus();
            input.preedit.clear();
            let x = e.local.0 + cx.scroll_offset(input.scroll_id()).0;
            let index = input.from_display(cx.shaper.hit(&input.display_text(), input.style.get(), x));
            match e.click_count {
                1 => input.move_to(index, e.modifiers.shift),
                2 => input.select_word_at(index),
                _ => input.select_all(),
            }
            input.reveal_caret(cx);
        })
        .on_drag(move |s: &mut S, cx, e| {
            let input = drag(s);
            if e.click_count != 1 {
                return;
            }
            let x = e.local.0 + cx.scroll_offset(input.scroll_id()).0;
            let index = input.from_display(cx.shaper.hit(&input.display_text(), input.style.get(), x));
            input.move_to(index, true);
            input.restart_blink();
            input.reveal_caret(cx);
        })
        .on_mouse_down_outside(move |s: &mut S, _| {
            let input = outside(s);
            if input.focused {
                input.blur();
            }
        })
        .child(content);
    if let Some(family) = font.family {
        root = root.text_family(family);
    }
    if font.bold {
        root = root.text_bold();
    }
    if font.italic {
        root = root.text_italic();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event::NamedKey, testing::TestHost, view::View};

    struct Form {
        name: TextInput,
        note: TextInput,
        submitted: u32,
    }

    impl Form {
        fn new() -> Form {
            Form { name: TextInput::new(), note: TextInput::new(), submitted: 0 }
        }
    }

    fn style() -> InputStyle {
        InputStyle::new(16.).blink(false).placeholder("Type here")
    }

    impl View for Form {
        fn view(&self, cx: &mut Cx) -> Element<Form> {
            div()
                .p(20.)
                .gap(20.)
                .child(div().w(200.).child(text_input(&self.name, &style(), cx, |s: &mut Form| &mut s.name)))
                .child(div().w(200.).child(text_input(&self.note, &style(), cx, |s: &mut Form| &mut s.note)))
        }

        fn event(&mut self, event: &Event, cx: &mut Cx) {
            if self.name.event(event, cx) == InputChange::Submitted {
                self.submitted += 1;
            }
            self.note.event(event, cx);
        }
    }

    const NAME_AT: (f32, f32) = (30., 30.);
    const NOTE_AT: (f32, f32) = (30., 30. + 20. + 22.);

    fn host() -> TestHost<Form> {
        let mut h = TestHost::new(Form::new(), (300., 200.));
        h.frame();
        h
    }

    fn type_str(h: &mut TestHost<Form>, s: &str) {
        for c in s.chars() {
            let t = c.to_string();
            h.key(Key::Char(t.clone()), Some(&t));
        }
    }

    fn cmd() -> Modifiers {
        if cfg!(target_os = "macos") { Modifiers { logo: true, ..Default::default() } } else { Modifiers { ctrl: true, ..Default::default() } }
    }

    fn word() -> Modifiers {
        if cfg!(target_os = "macos") { Modifiers { alt: true, ..Default::default() } } else { Modifiers { ctrl: true, ..Default::default() } }
    }

    #[test]
    fn insert_delete_and_selection_replace() {
        let mut t = TextInput::with_text("hello world");
        t.move_to(5, false);
        t.insert(",");
        assert_eq!((t.text(), t.caret()), ("hello, world", 6));
        t.move_to(0, false);
        t.move_to(5, true);
        assert_eq!(t.selected_text(), "hello");
        t.insert("bye");
        assert_eq!(t.text(), "bye, world");
        t.select_all();
        t.backspace(false);
        assert_eq!((t.text(), t.caret()), ("", 0));
        t.backspace(false); // nothing left: no panic
        t.delete(false);
        t.insert("a\nb\tc");
        assert_eq!(t.text(), "abc", "single line: control characters are dropped");
    }

    #[test]
    fn word_and_line_movement() {
        let mut t = TextInput::with_text("foo bar  baz");
        t.move_left(true, false);
        assert_eq!(t.caret(), "foo bar  ".len());
        t.move_left(true, false);
        assert_eq!(t.caret(), "foo ".len());
        t.move_right(true, false);
        assert_eq!(t.caret(), "foo bar".len());
        t.move_home(true);
        assert_eq!(t.selection(), (0, "foo bar".len()));
        t.move_right(false, false);
        assert_eq!(t.caret(), "foo bar".len(), "a plain arrow collapses the selection to its end");
        t.move_end(false);
        t.backspace(true);
        assert_eq!(t.text(), "foo bar  ");
        t.move_home(false);
        t.delete(true);
        assert_eq!(t.text(), " bar  ", "delete-word removes the word, not the spaces after it");
    }

    #[test]
    fn double_click_word_and_shift_selection_helpers() {
        let mut t = TextInput::with_text("hello 한글 world");
        t.select_word_at("hello 한".len());
        assert_eq!(t.selected_text(), "한글");
        assert_eq!(t.word_at("hello".len()), (0, 5), "just after a word still picks that word");
    }

    #[test]
    fn korean_composition_becomes_text_only_on_commit() {
        let mut t = TextInput::new();
        for step in ["ㅎ", "하", "한"] {
            t.ime(Ime::Preedit { text: step.into(), cursor: None });
            assert_eq!((t.text(), t.preedit()), ("", step), "composing must not change the committed text");
        }
        t.ime(Ime::Commit("한".into()));
        t.ime(Ime::Preedit { text: "ㄱ".into(), cursor: None });
        t.ime(Ime::Preedit { text: "글".into(), cursor: None });
        assert_eq!((t.text(), t.preedit()), ("한", "글"));
        t.ime(Ime::Commit("글".into()));
        assert_eq!((t.text(), t.preedit(), t.caret()), ("한글", "", "한글".len()));
    }

    #[test]
    fn a_composition_can_be_cancelled_and_replaces_the_selection() {
        let mut t = TextInput::with_text("hello world");
        t.move_to(0, false);
        t.move_to(5, true);
        t.ime(Ime::Preedit { text: "ㅇ".into(), cursor: None });
        assert_eq!(t.text(), " world", "starting to compose replaces the selection");
        t.ime(Ime::Preedit { text: String::new(), cursor: None });
        assert!(!t.is_composing());
        assert_eq!(t.text(), " world", "cancelling leaves the text as it was");
        t.ime(Ime::Commit("안녕".into()));
        assert_eq!(t.text(), "안녕 world");
        assert_eq!(t.caret(), "안녕".len());
    }

    #[test]
    fn decomposed_hangul_and_combining_marks_move_as_one_character() {
        // 한 as conjoining jamo: choseong + jungseong + jongseong
        let mut t = TextInput::with_text("a\u{1112}\u{1161}\u{11ab}b");
        t.move_left(false, false);
        t.move_left(false, false);
        assert_eq!(t.caret(), 1, "one step over the whole syllable");
        t.move_end(false);
        t.move_left(false, false);
        t.backspace(false);
        assert_eq!(t.text(), "ab");
        let mut e = TextInput::with_text("e\u{301}x");
        e.move_left(false, false);
        e.backspace(false);
        assert_eq!(e.text(), "x", "the accent goes with its letter");
    }

    #[test]
    fn typing_focus_and_enter_through_the_app() {
        let mut h = host();
        assert!(h.scene().texts().any(|t| t.content == "Type here"), "an empty field shows its placeholder");
        h.click(NAME_AT);
        assert!(h.state().name.is_focused() && !h.state().note.is_focused());
        assert!(h.ime_allowed());
        type_str(&mut h, "kim");
        assert_eq!(h.state().name.text(), "kim");
        assert!(h.scene().texts().any(|t| t.content == "kim"));
        // Typing goes to the focused field only.
        h.click(NOTE_AT);
        assert!(h.state().note.is_focused() && !h.state().name.is_focused(), "pressing another field moves focus");
        type_str(&mut h, "x");
        assert_eq!((h.state().name.text(), h.state().note.text()), ("kim", "x"));
        h.key(Key::Named(NamedKey::Enter), None);
        assert_eq!(h.state().submitted, 0, "Enter in the note does not submit the name");
        h.click(NAME_AT);
        h.key(Key::Named(NamedKey::Enter), None);
        assert_eq!(h.state().submitted, 1);
        h.click((250., 190.));
        assert!(!h.state().name.is_focused(), "pressing outside every field blurs it");
        type_str(&mut h, "zzz");
        assert_eq!(h.state().name.text(), "kim", "an unfocused field ignores keys");
    }

    #[test]
    fn clicking_places_the_caret_and_dragging_selects() {
        let mut h = host();
        h.state_mut().name.set_text("hello world");
        h.frame();
        let wide = h.text_center("hello world");
        h.click((21., wide.1));
        assert_eq!(h.state().name.caret(), 0, "a click at the left edge puts the caret first");
        h.click((190., wide.1));
        assert_eq!(h.state().name.caret(), "hello world".len());
        h.mouse_down((21., wide.1));
        h.mouse_move((wide.0, wide.1));
        h.mouse_up((wide.0, wide.1));
        let (a, b) = h.state().name.selection();
        assert!(a == 0 && b > 0 && b < "hello world".len(), "dragging to the middle selects part: {a}..{b}");
        h.mouse_down_n((21., wide.1), 2);
        assert_eq!(h.state().name.selected_text(), "hello", "double click selects a word");
        h.mouse_down_n((21., wide.1), 3);
        assert_eq!(h.state().name.selected_text(), "hello world", "triple click selects all");
    }

    #[test]
    fn clipboard_copy_cut_and_paste() {
        let mut h = host();
        h.click(NAME_AT);
        type_str(&mut h, "one two");
        h.set_modifiers(cmd());
        h.key(Key::Char("a".into()), Some("a"));
        h.key(Key::Char("c".into()), Some("c"));
        assert_eq!(h.clipboard().as_deref(), Some("one two"));
        h.key(Key::Char("x".into()), Some("x"));
        assert_eq!(h.state().name.text(), "");
        h.set_clipboard("pasted\nline");
        h.key(Key::Char("v".into()), Some("v"));
        assert_eq!(h.state().name.text(), "pastedline", "paste is one line");
        h.set_modifiers(word());
        h.key(Key::Named(NamedKey::Backspace), None);
        assert_eq!(h.state().name.text(), "");
    }

    #[test]
    fn ime_through_the_app_shows_the_composition_and_tells_the_ime_where_the_caret_is() {
        let mut h = host();
        h.click(NAME_AT);
        h.ime(Ime::Preedit { text: "하".into(), cursor: None });
        assert!(h.scene().texts().any(|t| t.content == "하"), "the composition is drawn");
        assert_eq!(h.state().name.text(), "");
        let area = h.ime_cursor_area().expect("the field reports its caret");
        assert!(area.x > 30. && area.y >= 20., "{area:?}");
        h.key(Key::Char("x".into()), Some("x"));
        assert_eq!(h.state().name.text(), "", "keys belong to the input method while composing");
        h.ime(Ime::Commit("하".into()));
        assert_eq!(h.state().name.text(), "하");
    }

    #[test]
    fn long_text_scrolls_to_keep_the_caret_visible() {
        let mut h = host();
        h.click(NAME_AT);
        type_str(&mut h, "the quick brown fox jumps over the lazy dog");
        let id = h.state().name.scroll_id();
        h.frame();
        let offset = h.scroll_offset(id).0;
        assert!(offset > 0., "scrolled right to follow the caret");
        let at = h.state().name.caret_x(&mut Shaper::new());
        assert!(at - offset <= 200. + 1., "the caret is inside the 200px field: {at} - {offset}");
        h.key(Key::Named(NamedKey::Home), None);
        h.frame();
        assert_eq!(h.scroll_offset(id).0, 0., "Home scrolls back");
        // A click after scrolling maps through the offset.
        h.key(Key::Named(NamedKey::End), None);
        h.frame();
        h.click((21., 30.));
        let caret = h.state().name.caret();
        assert!(caret > 0, "the left edge of the scrolled field is not the start of the text (caret {caret})");
    }

    #[test]
    fn cycle_focus_wraps_and_blurs_the_rest() {
        let (mut a, mut b) = (TextInput::new(), TextInput::new());
        cycle_focus(&mut [&mut a, &mut b], false);
        assert!(a.is_focused() && !b.is_focused());
        cycle_focus(&mut [&mut a, &mut b], false);
        assert!(!a.is_focused() && b.is_focused());
        cycle_focus(&mut [&mut a, &mut b], false);
        assert!(a.is_focused());
        cycle_focus(&mut [&mut a, &mut b], true);
        assert!(b.is_focused() && !a.is_focused());
    }

    // ---- masking ----

    #[test]
    fn masked_editing_matches_the_plain_field_for_multibyte_text() {
        let mut plain = TextInput::new();
        let mut masked = TextInput::new();
        masked.set_masked(true);
        assert!(masked.is_masked() && !plain.is_masked());
        for t in [&mut plain, &mut masked] {
            t.insert("pw한글é😀x");
            t.move_left(false, false);
            t.move_left(false, false);
            t.move_left(false, true);
            t.insert("Z");
            t.move_home(false);
            t.move_right(false, true);
            t.move_right(false, true);
            t.backspace(false);
            t.move_end(false);
            t.delete(false);
        }
        assert_eq!(masked.text(), plain.text());
        assert_eq!((masked.caret(), masked.selection()), (plain.caret(), plain.selection()));
        assert_eq!(masked.selected_text(), plain.selected_text());
    }

    #[test]
    fn display_offsets_map_back_to_plain_byte_offsets() {
        let mut t = TextInput::with_text("a한😀é");
        t.set_masked(true);
        let ends: Vec<usize> = t.text().char_indices().map(|(i, _)| i).chain([t.text().len()]).collect();
        for (n, &byte) in ends.iter().enumerate() {
            assert_eq!(t.to_display(byte), n * 3);
            assert_eq!(t.from_display(n * 3), byte);
        }
        assert_eq!(t.from_display(10_000), t.text().len(), "past the end clamps");
        assert_eq!(t.shown(), "••••");
        assert_eq!(t.text(), "a한😀é", "the plain text is untouched");
    }

    #[test]
    fn a_masked_field_draws_bullets_not_the_text() {
        let mut h = host();
        h.state_mut().name.set_masked(true);
        h.click(NAME_AT);
        type_str(&mut h, "s3한글");
        assert_eq!(h.state().name.text(), "s3한글");
        assert!(h.scene().texts().any(|t| t.content == "••••"), "four bullets");
        assert!(h.scene().texts().all(|t| !t.content.contains("s3") && !t.content.contains('한')), "no plain text drawn");
    }

    #[test]
    fn copy_and_cut_are_blocked_while_masked_but_paste_works() {
        let mut h = host();
        h.state_mut().name.set_masked(true);
        h.click(NAME_AT);
        type_str(&mut h, "secret");
        h.set_modifiers(cmd());
        h.key(Key::Char("a".into()), Some("a"));
        h.set_clipboard("untouched");
        h.key(Key::Char("c".into()), Some("c"));
        assert_eq!(h.clipboard().as_deref(), Some("untouched"));
        h.key(Key::Char("x".into()), Some("x"));
        assert_eq!(h.clipboard().as_deref(), Some("untouched"));
        assert_eq!(h.state().name.text(), "secret", "cut neither copies nor deletes");
        h.set_clipboard("hunter2");
        h.key(Key::Char("v".into()), Some("v"));
        assert_eq!(h.state().name.text(), "hunter2", "paste replaces the selection");
        h.state_mut().name.set_masked(false);
        h.key(Key::Char("a".into()), Some("a"));
        h.key(Key::Char("c".into()), Some("c"));
        assert_eq!(h.clipboard().as_deref(), Some("hunter2"), "unmasked copies again");
    }

    #[test]
    fn clicks_selection_and_composition_follow_the_masked_display() {
        let mut h = host();
        h.state_mut().name.set_masked(true);
        h.state_mut().name.set_text("한글abc");
        h.frame();
        let bullets = h.text_center("•••••");
        h.click((21., bullets.1));
        assert_eq!(h.state().name.caret(), 0);
        h.click((190., bullets.1));
        assert_eq!(h.state().name.caret(), "한글abc".len());
        h.mouse_down((21., bullets.1));
        h.mouse_move((bullets.0, bullets.1));
        h.mouse_up((bullets.0, bullets.1));
        let (a, b) = h.state().name.selection();
        assert!("한글abc".is_char_boundary(a) && "한글abc".is_char_boundary(b) && a < b, "{a}..{b}");
        h.click((21., bullets.1));
        h.ime(Ime::Preedit { text: "하".into(), cursor: None });
        assert!(h.state().name.is_composing());
        assert!(h.scene().texts().any(|t| t.content == "••••••"), "the composition is masked too");
        h.ime(Ime::Commit("하".into()));
        assert_eq!(h.state().name.text(), "하한글abc");
    }

    #[test]
    fn a_long_masked_value_still_scrolls_to_the_caret() {
        let mut h = host();
        h.state_mut().name.set_masked(true);
        h.click(NAME_AT);
        type_str(&mut h, "the quick brown fox jumps over the lazy dog");
        h.frame();
        let id = h.state().name.scroll_id();
        assert!(h.scroll_offset(id).0 > 0.);
    }
}
