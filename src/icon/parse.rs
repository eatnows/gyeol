//! Reading SVG text into [`IconData`] (the `svg` feature).
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::{Arc, Mutex, OnceLock},
};

use svgtypes::{PointsParser, SimplePathSegment, SimplifyingPathParser, Transform};

use super::{FillRule, IconCommand, IconData, IconFill, IconShape, IconStroke, LineCap, LineJoin};
use crate::{error::Result, scene::Rect, Error};

/// How far `0.5522847498` of a radius a cubic's control points sit to draw a quarter circle.
const KAPPA: f32 = 0.552_284_8;

impl IconData {
    /// Parses the SVG `source` into an icon.
    ///
    /// **Elements:** `svg` (its `viewBox`, or `width`/`height`, and the presentation attributes on it),
    /// `g`, `path`, `rect` (with `rx`/`ry`), `circle`, `ellipse`, `line`, `polyline`, `polygon`. Anything
    /// else (`defs`, `use`, `text`, `image`, `clipPath`, ...) is ignored.
    ///
    /// **Path data:** all of `M m L l H h V v C c S s Q q T t A a Z z`; arcs become cubics.
    ///
    /// **Presentation** (as attributes or in a `style="..."` attribute, inherited down through groups):
    /// `fill`, `fill-rule` (`nonzero`/`evenodd`), `fill-opacity`, `stroke`, `stroke-width`,
    /// `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `stroke-opacity`, `opacity`, and
    /// `transform` (`matrix`, `translate`, `scale`, `rotate`, `skewX`, `skewY`). Colors are not kept: the
    /// icon is tinted with a single color when drawn, so `currentColor`, `#fff` and `red` all mean "paint
    /// with the tint" and only `none` turns a paint off. Not supported: gradients and patterns
    /// (`url(...)`), dashes, `style` sheets, units other than plain numbers and `px`, percentages.
    ///
    /// An unset `fill` is black (so painted), as in SVG. A path with a syntax error is drawn up to the error.
    pub fn parse(source: &str) -> Result<IconData> {
        let doc = roxmltree::Document::parse(source).map_err(|e| Error(format!("parse svg: {e}")))?;
        let root = doc.root_element();
        if root.tag_name().name() != "svg" {
            return Err(Error("parse svg: the root element is not <svg>".into()));
        }
        let view_box = view_box(&root);
        let mut shapes = Vec::new();
        let state = State::default().apply(&root);
        // The root's own transform is not part of the SVG model for <svg>; children start from identity.
        walk(&root, &state, &mut shapes);
        Ok(IconData::new(view_box, shapes))
    }

    /// [`IconData::parse`], remembered by the text of the source so that calling it every frame costs a
    /// hash. A source that does not parse gives an icon that draws nothing (and prints the error once).
    pub fn cached(source: &str) -> Arc<IconData> {
        static CACHE: OnceLock<Mutex<HashMap<u64, Arc<IconData>>>> = OnceLock::new();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        let key = hasher.finish();
        let mut cache = CACHE.get_or_init(Default::default).lock().expect("icon cache");
        cache
            .entry(key)
            .or_insert_with(|| {
                Arc::new(IconData::parse(source).unwrap_or_else(|e| {
                    eprintln!("gyeol: {e}");
                    IconData::new(Rect::new(0., 0., 24., 24.), Vec::new())
                }))
            })
            .clone()
    }
}

fn number(text: &str) -> Option<f32> {
    let text = text.trim();
    f32::from_str(text.strip_suffix("px").unwrap_or(text).trim()).ok().filter(|v| v.is_finite())
}

fn attribute(node: &roxmltree::Node, name: &str) -> f32 {
    node.attribute(name).and_then(number).unwrap_or(0.)
}

fn view_box(root: &roxmltree::Node) -> Rect {
    if let Some(v) = root.attribute("viewBox") {
        let parts: Vec<f32> = v.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        if let [x, y, w, h] = parts[..] {
            if w > 0. && h > 0. {
                return Rect::new(x, y, w, h);
            }
        }
    }
    let size = |name| root.attribute(name).and_then(number).filter(|v| *v > 0.);
    Rect::new(0., 0., size("width").unwrap_or(24.), size("height").unwrap_or(24.))
}

/// The presentation state inherited down the tree.
#[derive(Clone)]
struct State {
    fill: bool,
    fill_rule: FillRule,
    fill_opacity: f32,
    stroke: bool,
    stroke_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
    stroke_opacity: f32,
    opacity: f32,
    transform: [f32; 6],
}

impl Default for State {
    fn default() -> Self {
        State {
            fill: true,
            fill_rule: FillRule::NonZero,
            fill_opacity: 1.,
            stroke: false,
            stroke_width: 1.,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.,
            stroke_opacity: 1.,
            opacity: 1.,
            transform: [1., 0., 0., 1., 0., 0.],
        }
    }
}

fn paint_on(value: &str) -> bool {
    !matches!(value.trim(), "none" | "transparent")
}

fn multiply(p: [f32; 6], c: [f32; 6]) -> [f32; 6] {
    [
        p[0] * c[0] + p[2] * c[1],
        p[1] * c[0] + p[3] * c[1],
        p[0] * c[2] + p[2] * c[3],
        p[1] * c[2] + p[3] * c[3],
        p[0] * c[4] + p[2] * c[5] + p[4],
        p[1] * c[4] + p[3] * c[5] + p[5],
    ]
}

impl State {
    fn set(&mut self, name: &str, value: &str) {
        let value = value.trim();
        match name {
            "fill" => self.fill = paint_on(value),
            "fill-rule" => self.fill_rule = if value == "evenodd" { FillRule::EvenOdd } else { FillRule::NonZero },
            "fill-opacity" => self.fill_opacity = number(value).unwrap_or(1.).clamp(0., 1.),
            "stroke" => self.stroke = paint_on(value),
            "stroke-width" => self.stroke_width = number(value).unwrap_or(1.).max(0.),
            "stroke-linecap" => {
                self.cap = match value {
                    "round" => LineCap::Round,
                    "square" => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "stroke-linejoin" => {
                self.join = match value {
                    "round" => LineJoin::Round,
                    "bevel" => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "stroke-miterlimit" => self.miter_limit = number(value).unwrap_or(4.).max(1.),
            "stroke-opacity" => self.stroke_opacity = number(value).unwrap_or(1.).clamp(0., 1.),
            "opacity" => self.opacity *= number(value).unwrap_or(1.).clamp(0., 1.),
            "transform" => {
                if let Ok(t) = Transform::from_str(value) {
                    self.transform = multiply(self.transform, [t.a as f32, t.b as f32, t.c as f32, t.d as f32, t.e as f32, t.f as f32]);
                }
            }
            _ => {}
        }
    }

    /// This state with `node`'s own attributes (and `style="a: b; c: d"` declarations) applied.
    fn apply(&self, node: &roxmltree::Node) -> State {
        let mut state = self.clone();
        for attr in node.attributes() {
            state.set(attr.name(), attr.value());
        }
        if let Some(style) = node.attribute("style") {
            for declaration in style.split(';') {
                if let Some((name, value)) = declaration.split_once(':') {
                    state.set(name.trim(), value);
                }
            }
        }
        state
    }
}

fn walk(node: &roxmltree::Node, state: &State, shapes: &mut Vec<IconShape>) {
    for child in node.children().filter(|c| c.is_element()) {
        let name = child.tag_name().name();
        if name == "g" || name == "svg" || name == "a" {
            walk(&child, &state.apply(&child), shapes);
            continue;
        }
        let commands = match name {
            "path" => path_commands(child.attribute("d").unwrap_or("")),
            "rect" => rect_commands(&child),
            "circle" => {
                let r = attribute(&child, "r");
                ellipse_commands(attribute(&child, "cx"), attribute(&child, "cy"), r, r)
            }
            "ellipse" => ellipse_commands(attribute(&child, "cx"), attribute(&child, "cy"), attribute(&child, "rx"), attribute(&child, "ry")),
            "line" => vec![
                IconCommand::MoveTo(attribute(&child, "x1"), attribute(&child, "y1")),
                IconCommand::LineTo(attribute(&child, "x2"), attribute(&child, "y2")),
            ],
            "polyline" | "polygon" => poly_commands(child.attribute("points").unwrap_or(""), name == "polygon"),
            _ => continue,
        };
        if commands.is_empty() {
            continue;
        }
        let s = state.apply(&child);
        let fill = s.fill.then_some(IconFill { rule: s.fill_rule, opacity: s.fill_opacity * s.opacity });
        let stroke = (s.stroke && s.stroke_width > 0.).then_some(IconStroke {
            width: s.stroke_width,
            cap: s.cap,
            join: s.join,
            miter_limit: s.miter_limit,
            opacity: s.stroke_opacity * s.opacity,
        });
        if fill.is_some() || stroke.is_some() {
            shapes.push(IconShape { commands, transform: s.transform, fill, stroke });
        }
    }
}

fn path_commands(d: &str) -> Vec<IconCommand> {
    let mut out = Vec::new();
    for segment in SimplifyingPathParser::from(d) {
        let Ok(segment) = segment else { break };
        out.push(match segment {
            SimplePathSegment::MoveTo { x, y } => IconCommand::MoveTo(x as f32, y as f32),
            SimplePathSegment::LineTo { x, y } => IconCommand::LineTo(x as f32, y as f32),
            SimplePathSegment::CurveTo { x1, y1, x2, y2, x, y } => IconCommand::CubicTo(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32),
            SimplePathSegment::Quadratic { x1, y1, x, y } => IconCommand::QuadTo(x1 as f32, y1 as f32, x as f32, y as f32),
            SimplePathSegment::ClosePath => IconCommand::Close,
        });
    }
    out
}

fn poly_commands(points: &str, close: bool) -> Vec<IconCommand> {
    let mut out: Vec<IconCommand> = PointsParser::from(points)
        .enumerate()
        .map(|(i, (x, y))| if i == 0 { IconCommand::MoveTo(x as f32, y as f32) } else { IconCommand::LineTo(x as f32, y as f32) })
        .collect();
    if close && !out.is_empty() {
        out.push(IconCommand::Close);
    }
    out
}

/// An ellipse as four cubic quarter arcs, starting at its rightmost point and going clockwise on screen.
fn ellipse_commands(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<IconCommand> {
    if rx <= 0. || ry <= 0. {
        return Vec::new();
    }
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    vec![
        IconCommand::MoveTo(cx + rx, cy),
        IconCommand::CubicTo(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry),
        IconCommand::CubicTo(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy),
        IconCommand::CubicTo(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry),
        IconCommand::CubicTo(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy),
        IconCommand::Close,
    ]
}

fn rect_commands(node: &roxmltree::Node) -> Vec<IconCommand> {
    let (x, y, w, h) = (attribute(node, "x"), attribute(node, "y"), attribute(node, "width"), attribute(node, "height"));
    if w <= 0. || h <= 0. {
        return Vec::new();
    }
    // `rx` and `ry` default to each other, and are limited to half the side.
    let (rx, ry) = match (node.attribute("rx").and_then(number), node.attribute("ry").and_then(number)) {
        (Some(rx), Some(ry)) => (rx, ry),
        (Some(r), None) | (None, Some(r)) => (r, r),
        (None, None) => (0., 0.),
    };
    let (rx, ry) = (rx.clamp(0., w / 2.), ry.clamp(0., h / 2.));
    if rx == 0. || ry == 0. {
        return vec![
            IconCommand::MoveTo(x, y),
            IconCommand::LineTo(x + w, y),
            IconCommand::LineTo(x + w, y + h),
            IconCommand::LineTo(x, y + h),
            IconCommand::Close,
        ];
    }
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let (r, b) = (x + w, y + h);
    vec![
        IconCommand::MoveTo(x + rx, y),
        IconCommand::LineTo(r - rx, y),
        IconCommand::CubicTo(r - rx + kx, y, r, y + ry - ky, r, y + ry),
        IconCommand::LineTo(r, b - ry),
        IconCommand::CubicTo(r, b - ry + ky, r - rx + kx, b, r - rx, b),
        IconCommand::LineTo(x + rx, b),
        IconCommand::CubicTo(x + rx - kx, b, x, b - ry + ky, x, b - ry),
        IconCommand::LineTo(x, y + ry),
        IconCommand::CubicTo(x, y + ry - ky, x + rx - kx, y, x + rx, y),
        IconCommand::Close,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use IconCommand::*;

    fn one(svg: &str) -> IconShape {
        let icon = IconData::parse(svg).expect("parses");
        assert_eq!(icon.shapes.len(), 1, "{svg}");
        icon.shapes[0].clone()
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_lucide_style_root_gives_stroked_unfilled_shapes_with_inherited_style() {
        let icon = IconData::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                 stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>"#,
        )
        .unwrap();
        assert_eq!(icon.view_box, Rect::new(0., 0., 24., 24.));
        assert_eq!(icon.shapes.len(), 2);
        for shape in &icon.shapes {
            assert_eq!(shape.fill, None);
            let stroke = shape.stroke.expect("stroked");
            assert_eq!((stroke.width, stroke.cap, stroke.join), (2., LineCap::Round, LineJoin::Round));
        }
        assert_eq!(icon.shapes[0].commands, vec![MoveTo(5., 12.), LineTo(19., 12.)]);
        assert_eq!(icon.shapes[1].commands, vec![MoveTo(12., 5.), LineTo(19., 12.), LineTo(12., 19.)], "relative commands resolve to absolute");
    }

    #[test]
    fn path_commands_h_v_s_t_and_close_are_resolved() {
        let shape = one(r#"<svg viewBox="0 0 10 10"><path d="M1 1 H5 V4 h-2 v-1 C 1 1 2 2 3 3 S 5 5 6 6 Q 7 7 8 8 T 9 9 z"/></svg>"#);
        let c = &shape.commands;
        assert_eq!(c[..5], [MoveTo(1., 1.), LineTo(5., 1.), LineTo(5., 4.), LineTo(3., 4.), LineTo(3., 3.)]);
        assert_eq!(c[5], CubicTo(1., 1., 2., 2., 3., 3.));
        // S reflects the previous control point (2,2) about (3,3): (4,4).
        assert_eq!(c[6], CubicTo(4., 4., 5., 5., 6., 6.));
        assert_eq!(c[7], QuadTo(7., 7., 8., 8.));
        // T reflects the previous quad control (7,7) about (8,8): (9,9).
        assert_eq!(c[8], QuadTo(9., 9., 9., 9.));
        assert_eq!(*c.last().unwrap(), Close);
        assert_eq!(shape.fill.expect("fill defaults on").rule, FillRule::NonZero);
    }

    #[test]
    fn arcs_become_cubics_that_end_where_they_should() {
        // Half a circle of radius 5 from (0,5) to (10,5).
        let shape = one(r#"<svg viewBox="0 0 10 10"><path d="M0 5 A5 5 0 0 1 10 5"/></svg>"#);
        let curves: Vec<&IconCommand> = shape.commands.iter().filter(|c| matches!(c, CubicTo(..))).collect();
        assert!(curves.len() >= 2, "a half circle takes at least two cubics: {:?}", shape.commands);
        let Some(CubicTo(.., x, y)) = curves.last().copied().copied() else { panic!() };
        assert!(near(x, 10.) && near(y, 5.), "{x} {y}");
        // The sweep flag picks the side: sweep=1 from left to right goes over the top (y < 5).
        let tops: Vec<f32> = shape.commands.iter().filter_map(|c| if let CubicTo(_, y1, ..) = c { Some(*y1) } else { None }).collect();
        assert!(tops.iter().all(|y| *y <= 5. + 1e-3));
        let flipped = one(r#"<svg viewBox="0 0 10 10"><path d="M0 5 A5 5 0 0 0 10 5"/></svg>"#);
        assert!(flipped.commands.iter().any(|c| matches!(c, CubicTo(_, y1, ..) if *y1 > 5.)), "sweep=0 goes under");
    }

    #[test]
    fn basic_shapes_become_paths() {
        let circle = one(r#"<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/></svg>"#);
        assert_eq!(circle.commands[0], MoveTo(16., 12.));
        assert_eq!(circle.commands.iter().filter(|c| matches!(c, CubicTo(..))).count(), 4);
        let ellipse = one(r#"<svg viewBox="0 0 24 24"><ellipse cx="10" cy="10" rx="6" ry="3"/></svg>"#);
        assert_eq!(ellipse.commands[0], MoveTo(16., 10.));
        assert!(ellipse.commands.iter().any(|c| matches!(c, CubicTo(.., x, y) if near(*x, 10.) && near(*y, 13.))), "the bottom is (10, 13)");

        let rect = one(r#"<svg viewBox="0 0 24 24"><rect x="2" y="3" width="10" height="6"/></svg>"#);
        assert_eq!(rect.commands, vec![MoveTo(2., 3.), LineTo(12., 3.), LineTo(12., 9.), LineTo(2., 9.), Close]);
        let rounded = one(r#"<svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/></svg>"#);
        assert_eq!(rounded.commands[0], MoveTo(5., 3.), "rx also sets ry and starts after the corner");
        assert_eq!(rounded.commands.iter().filter(|c| matches!(c, CubicTo(..))).count(), 4);
        let huge = one(r#"<svg viewBox="0 0 24 24"><rect width="10" height="10" rx="99"/></svg>"#);
        assert_eq!(huge.commands[0], MoveTo(5., 0.), "the radius is capped at half the side");

        let line = one(r#"<svg viewBox="0 0 24 24" stroke="black"><line x1="1" y1="2" x2="3" y2="4"/></svg>"#);
        assert_eq!(line.commands, vec![MoveTo(1., 2.), LineTo(3., 4.)]);
        let polyline = one(r#"<svg viewBox="0 0 24 24"><polyline points="1,2 3,4 5,6"/></svg>"#);
        assert_eq!(polyline.commands, vec![MoveTo(1., 2.), LineTo(3., 4.), LineTo(5., 6.)]);
        let polygon = one(r#"<svg viewBox="0 0 24 24"><polygon points="1 2, 3 4, 5 6"/></svg>"#);
        assert_eq!(polygon.commands, vec![MoveTo(1., 2.), LineTo(3., 4.), LineTo(5., 6.), Close]);
    }

    #[test]
    fn paint_inheritance_groups_style_attributes_and_transforms() {
        let icon = IconData::parse(
            r##"<svg viewBox="0 0 24 24" fill="currentColor">
                 <g stroke="red" stroke-width="3" opacity="0.5" transform="translate(2 3)">
                   <path d="M0 0h1" fill="none" style="stroke-linecap: square; stroke-opacity: 0.5"/>
                   <path d="M0 0h1" fill-rule="evenodd" stroke="none" transform="scale(2)"/>
                 </g>
                 <desc>ignored</desc><defs><path d="M0 0"/></defs><use href="#x"/>
               </svg>"##,
        )
        .unwrap();
        assert_eq!(icon.shapes.len(), 2, "defs, desc and use are ignored");
        let (a, b) = (&icon.shapes[0], &icon.shapes[1]);
        assert_eq!(a.fill, None, "fill=none on the element wins over the root's");
        let stroke = a.stroke.unwrap();
        assert_eq!((stroke.width, stroke.cap), (3., LineCap::Square));
        assert!(near(stroke.opacity, 0.25), "stroke-opacity times the group's opacity: {}", stroke.opacity);
        assert_eq!(a.transform, [1., 0., 0., 1., 2., 3.]);
        assert_eq!(b.stroke, None, "stroke=none");
        let fill = b.fill.unwrap();
        assert_eq!(fill.rule, FillRule::EvenOdd);
        assert!(near(fill.opacity, 0.5));
        assert_eq!(b.transform, [2., 0., 0., 2., 2., 3.], "the group's translate then the shape's scale");
    }

    #[test]
    fn the_view_box_falls_back_to_the_size_and_bad_input_is_an_error() {
        assert_eq!(IconData::parse(r#"<svg width="16" height="20"/>"#).unwrap().view_box, Rect::new(0., 0., 16., 20.));
        assert_eq!(IconData::parse(r#"<svg viewBox="2 3 40 50"/>"#).unwrap().view_box, Rect::new(2., 3., 40., 50.));
        assert_eq!(IconData::parse("<svg/>").unwrap().view_box, Rect::new(0., 0., 24., 24.));
        assert!(IconData::parse("<svg").is_err());
        assert!(IconData::parse("<html/>").is_err());
        // A syntax error keeps what came before it.
        let shape = one(r#"<svg viewBox="0 0 10 10"><path d="M1 1 L2 2 L oops"/></svg>"#);
        assert_eq!(shape.commands, vec![MoveTo(1., 1.), LineTo(2., 2.)]);
        // `cached` returns the same icon for the same text.
        let svg = r#"<svg viewBox="0 0 5 5"><path d="M0 0h1"/></svg>"#;
        assert!(Arc::ptr_eq(&IconData::cached(svg), &IconData::cached(svg)));
    }
}
