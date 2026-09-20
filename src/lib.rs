//! **gyeolui** (결): a native UI toolkit for Rust, written from scratch on winit, wgpu and cosmic-text.
//!
//! Early days: the pieces so far are a renderer-agnostic [`Scene`], a wgpu [`Renderer`] and a
//! [`shell`] that opens a window and runs an [`App`].

mod error;
pub mod renderer;
pub mod scene;
pub mod shell;

pub use error::{Error, Result};
pub use renderer::Renderer;
pub use scene::{Color, Quad, Rect, Scene, Text};
pub use shell::{run, App, Frame};
