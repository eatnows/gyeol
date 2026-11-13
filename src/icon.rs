//! Vector icons: the data a [`crate::scene::Scene`] draws for an SVG, independent of any renderer.
//!
//! An [`IconData`] is a list of filled and/or stroked paths in the SVG's own coordinate system plus its
//! `viewBox`. The `svg` cargo feature adds [`IconData::parse`], which reads the usual icon-set subset of
//! SVG (Lucide, Feather, Material, Phosphor, ...); see its docs for what is supported.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::scene::{Color, Rect};

/// One segment of an icon path, in the icon's coordinates. Relative commands, arcs, `H`/`V`/`S`/`T`
/// shortcuts are already resolved into these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// Control point, end point.
    QuadTo(f32, f32, f32, f32),
    /// Two control points, end point.
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// SVG `fill-rule`: which regions of a self-overlapping or nested path count as inside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// SVG `stroke-linecap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// SVG `stroke-linejoin`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// How a shape is filled. The color is not here: icons are tinted with one color when drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconFill {
    pub rule: FillRule,
    /// `fill-opacity` times the opacity of the groups around it.
    pub opacity: f32,
}

/// How a shape is stroked, in the shape's own (pre-transform) units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconStroke {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub opacity: f32,
}

/// A path with its paint. Painted fill first, then stroke, like SVG.
#[derive(Clone, Debug, PartialEq)]
pub struct IconShape {
    pub commands: Vec<IconCommand>,
    /// `[a, b, c, d, e, f]` of the SVG matrix `matrix(a b c d e f)` mapping the commands into icon space.
    pub transform: [f32; 6],
    pub fill: Option<IconFill>,
    pub stroke: Option<IconStroke>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A parsed icon. Share it with an `Arc`: renderers cache what they draw per icon.
#[derive(Clone, Debug)]
pub struct IconData {
    id: u64,
    /// The SVG `viewBox`: the part of the icon's coordinate system that is drawn.
    pub view_box: Rect,
    pub shapes: Vec<IconShape>,
}

impl PartialEq for IconData {
    fn eq(&self, other: &Self) -> bool {
        self.view_box == other.view_box && self.shapes == other.shapes
    }
}

impl IconData {
    pub fn new(view_box: Rect, shapes: Vec<IconShape>) -> IconData {
        IconData { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), view_box, shapes }
    }

    /// A number that is unique to this icon, for renderers' caches.
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// An icon placed on the screen: `bounds` in logical pixels, tinted with `color` (its alpha included).
/// The icon is scaled to fit inside `bounds` keeping its aspect ratio, centered (SVG's default
/// `preserveAspectRatio="xMidYMid meet"`).
#[derive(Clone, Debug)]
pub struct Icon {
    pub data: Arc<IconData>,
    pub bounds: Rect,
    pub color: Color,
    /// Only the part inside this rectangle is drawn.
    pub clip: Option<Rect>,
}

impl Icon {
    pub fn new(data: Arc<IconData>, bounds: Rect, color: Color) -> Icon {
        Icon { data, bounds, color, clip: None }
    }

    pub fn clipped(mut self, clip: Option<Rect>) -> Icon {
        self.clip = clip;
        self
    }
}

#[cfg(feature = "svg")]
mod parse;
