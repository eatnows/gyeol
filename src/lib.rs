//! **gyeol** (결): a native UI toolkit for Rust, written from scratch on winit, wgpu and cosmic-text.
//!
//! Early days: a renderer-agnostic [`Scene`], a wgpu [`Renderer`], a [`Shaper`] for measuring text,
//! input [`Event`]s (including input-method composition for Korean and friends), and a [`shell`]
//! that opens a window and runs an [`App`].

pub mod element;
mod error;
pub mod event;
mod gpu;
mod layout;
pub mod list;
pub mod renderer;
pub mod scene;
pub mod shaper;
pub mod shell;
pub mod testing;
mod text;
pub mod view;

pub use element::{div, text, Align, Cursor, Direction, Element, ElementId, Justify, Length, Overflow, Style};
pub use error::{Error, Result};
pub use event::{Event, Ime, Key, Modifiers, MouseButton, NamedKey, ScrollDelta};
pub use renderer::Renderer;
pub use scene::{Color, Quad, Rect, Scene, Text};
pub use list::uniform_list;
pub use shaper::Shaper;
pub use shell::{run, App, Cx, Platform};
pub use view::{run_view, View};
