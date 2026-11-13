//! **gyeol** (결): a native UI toolkit for Rust, written from scratch on winit, wgpu and cosmic-text.
//!
//! Early days: a renderer-agnostic [`Scene`], a wgpu [`Renderer`], a [`Shaper`] for measuring text,
//! input [`Event`]s (including input-method composition for Korean and friends), and a [`shell`]
//! that opens a window and runs an [`App`].

pub mod element;
mod error;
#[cfg(target_os = "macos")]
mod macos;
pub mod event;
pub mod icon;
pub mod image;
pub mod input;
mod gpu;
#[cfg(feature = "svg")]
mod icon_render;
mod image_render;
mod layout;
pub mod pointer;
pub mod list;
pub mod renderer;
pub mod scene;
pub mod shaper;
pub mod shell;
pub mod snapshot;
pub mod testing;
mod text;
pub mod view;

pub use element::{div, icon_from, image_surface, paths, text, Align, Cursor, Direction, Element, ElementId, HoverStyle, Justify, Length, MouseEvent, Overflow, Style};
pub use error::{Error, Result};
#[cfg(feature = "svg")]
pub use element::icon;
pub use event::{Event, Ime, Key, Modifiers, MouseButton, NamedKey, ScrollDelta, SystemTheme};
pub use image::{Checkerboard, ImageBuffer, ImageDraw, ImageFilter, ImageView, PixelRect};
pub use input::{cycle_focus, text_input, InputChange, InputStyle, TextInput};
pub use pointer::{PointerKind, PointerSample};
pub use renderer::{Acquired, Renderer};
pub use scene::{BackdropBlur, Capped, Color, Gradient, GradientKind, GradientStop, RadialSize, MAX_GRADIENT_LAYERS, MAX_GRADIENT_STOPS, Path, PathCommand, Quad, Rect, Scene, Shadow, Text, MAX_SHADOWS};
pub use icon::{FillRule, Icon, IconCommand, IconData, IconFill, IconShape, IconStroke, LineCap, LineJoin};
pub use list::{list_rows, uniform_list};
pub use shaper::{Shaper, TextStyle};
pub use shell::{run, run_with, App, Appearance, Cx, Material, Platform, WindowOptions};
pub use view::{run_view, run_view_with, View};
