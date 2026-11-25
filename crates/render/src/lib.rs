#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! A software rasteriser for [`Scene`]s, used by `slickbi-cli render` and the image-diff harness.
//!
//! Shapes are drawn with `tiny-skia`; text is shaped and rasterised with `ab_glyph` using the
//! Inter font bundled in this crate, so the output does not depend on the host's installed fonts.
//! A scene is resolution-independent; [`rasterize`] renders it at a device scale into a pixmap,
//! and [`encode_png`] turns that into the bytes of a PNG file.

use ab_glyph::{Font, FontArc, Glyph, OutlinedGlyph, PxScale};
use slickbi_scene::{ArcSlice, Bar, LEGEND_HEIGHT, Scene, SeriesKind, TITLE_SPACE, Tick};
use thiserror::Error;
use tiny_skia::{
    BlendMode, Color, FilterQuality, IntSize, Paint, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, Transform,
};

/// The background colour of the chart panel.
const BACKGROUND: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// The colour of axis lines and tick marks.
const AXIS: [f32; 4] = [0.72, 0.75, 0.79, 1.0];
/// The colour of gridlines.
const GRID: [f32; 4] = [0.93, 0.94, 0.96, 1.0];
/// The colour of axis labels.
const TEXT: [f32; 4] = [0.24, 0.28, 0.33, 1.0];

/// The font size for axis labels, in points.
const LABEL_SIZE: f32 = 11.0;

/// Rasterises a scene into a pixmap at the given device scale (points to pixels).
///
/// # Panics
///
/// Panics if the scene has a non-positive size.
pub fn rasterize(scene: &Scene, scale: f32) -> Pixmap {
    rasterize_with(scene, scale, false)
}

/// Rasterises the free-tier preview: 1× with a watermark.
///
/// # Panics
///
/// Panics if the scene has a non-positive size.
pub fn rasterize_preview(scene: &Scene) -> Pixmap {
    rasterize_with(scene, 1.0, true)
}

fn rasterize_with(scene: &Scene, scale: f32, watermark: bool) -> Pixmap {
    let width = (scene.width * scale).round().max(1.0) as u32;
    let height = (scene.height * scale).round().max(1.0) as u32;
    let mut pixmap = Pixmap::new(width, height).expect("non-zero pixmap");
    pixmap.fill(color(BACKGROUND));

    let mut canvas = Canvas {
        pixmap,
        scale,
        font: FontArc::try_from_slice(include_bytes!("../fonts/Inter-Regular.ttf"))
            .expect("bundled font"),
    };

    canvas.draw_axes(scene);
    canvas.draw_series(scene);
    canvas.draw_labels(scene);
    canvas.draw_legend(scene);
    canvas.draw_titles(scene);
    if watermark {
        canvas.draw_watermark();
    }
    canvas.pixmap
}

/// Encodes a pixmap as PNG bytes.
///
/// # Errors
///
/// Returns [`RenderError`] if the pixmap cannot be encoded.
pub fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, RenderError> {
    pixmap
        .encode_png()
        .map_err(|error| RenderError::Png(error.to_string()))
}

/// Rasterises a scene at `scale` and encodes it as a PNG.
///
/// # Errors
///
/// Returns [`RenderError`] if the pixmap cannot be encoded.
pub fn render_png(scene: &Scene, scale: f32) -> Result<Vec<u8>, RenderError> {
    let pixmap = rasterize(scene, scale);
    encode_png(&pixmap)
}

/// Rasterises the free-tier preview (1×, watermarked) and encodes it as a PNG.
///
/// # Errors
///
/// Returns [`RenderError`] if the pixmap cannot be encoded.
pub fn render_preview_png(scene: &Scene) -> Result<Vec<u8>, RenderError> {
    let pixmap = rasterize_preview(scene);
    encode_png(&pixmap)
}

/// Renders a scene to a standalone SVG document.
///
/// SVG is vector, so this is resolution-independent: the document carries the scene's point
/// geometry directly. Text is emitted with the bundled Inter family and falls back to the system
/// sans-serif; a print pipeline that needs pixel-exact type should embed the pinned font.
///
/// Every label is XML-escaped, because category values come from the user's data.
pub fn render_svg(scene: &Scene) -> String {
    use std::fmt::Write as _;

    const AXIS_TEXT: &str = "#3d4754";
    const GRID: &str = "#edf0f4";
    const AXIS: &str = "#b8c0c9";
    const FONT: &str = "Inter, sans-serif";

    let width = scene.width.max(1.0);
    let height = scene.height.max(1.0);
    let plot = scene.plot;
    let has_axes = !scene.x_axis.ticks.is_empty() || !scene.y_axis.ticks.is_empty();

    let mut out = String::with_capacity(2048);
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.2}" height="{height:.2}" viewBox="0 0 {width:.2} {height:.2}">"#
    );
    let _ = write!(
        out,
        r##"<rect x="0" y="0" width="{width:.2}" height="{height:.2}" fill="#ffffff"/>"##
    );

    if has_axes {
        for tick in &scene.y_axis.ticks {
            let _ = write!(
                out,
                r#"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{GRID}" stroke-width="1"/>"#,
                plot.x,
                tick.position,
                plot.x + plot.width,
                tick.position
            );
        }
        let _ = write!(
            out,
            r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="none" stroke="{AXIS}" stroke-width="1"/>"#,
            plot.x, plot.y, plot.width, plot.height
        );
    }

    for series in &scene.series {
        let fill = hex(series.color);
        let opacity = series.color[3];
        match &series.kind {
            SeriesKind::Bars(bars) => {
                for bar in bars {
                    let _ = write!(
                        out,
                        r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{fill}" fill-opacity="{opacity:.3}"/>"#,
                        bar.x,
                        bar.y,
                        bar.width.max(0.0),
                        bar.height.max(0.0)
                    );
                }
            }
            SeriesKind::Line(points) => {
                if points.len() >= 2 {
                    let _ = write!(
                        out,
                        r#"<polyline points="{}" fill="none" stroke="{fill}" stroke-width="1.5" stroke-opacity="{opacity:.3}"/>"#,
                        polyline(points)
                    );
                }
            }
            SeriesKind::Area(points) => {
                if points.len() >= 2 {
                    let _ = write!(
                        out,
                        r#"<polygon points="{}" fill="{fill}" fill-opacity="0.18"/>"#,
                        area_polygon(points, plot.y + plot.height)
                    );
                    let _ = write!(
                        out,
                        r#"<polyline points="{}" fill="none" stroke="{fill}" stroke-width="1.5"/>"#,
                        polyline(points)
                    );
                }
            }
            SeriesKind::Points(points) => {
                for point in points {
                    let _ = write!(
                        out,
                        r#"<circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{fill}"/>"#,
                        point.x, point.y, point.radius
                    );
                }
            }
            SeriesKind::Arc(slice) => {
                let _ = write!(
                    out,
                    r#"<path d="{}" fill="{fill}" fill-opacity="{opacity:.3}"/>"#,
                    arc_path(slice)
                );
            }
            SeriesKind::Text(text) => {
                let _ = write!(
                    out,
                    r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="30" fill="{AXIS_TEXT}" text-anchor="middle" dominant-baseline="middle">{}</text>"#,
                    scene.width / 2.0,
                    scene.height / 2.0,
                    escape_xml(text)
                );
            }
        }
    }

    for label in &scene.labels {
        let _ = write!(
            out,
            r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="10" fill="{AXIS_TEXT}" text-anchor="middle">{}</text>"#,
            label.x,
            label.y,
            escape_xml(&label.text)
        );
    }

    if !scene.legend.is_empty() {
        let mut x = plot.x;
        let center_y = plot.y - LEGEND_HEIGHT / 2.0;
        for entry in &scene.legend {
            let _ = write!(
                out,
                r#"<rect x="{x:.2}" y="{:.2}" width="10" height="10" fill="{}"/>"#,
                center_y - 5.0,
                hex(entry.color)
            );
            x += 14.0;
            let _ = write!(
                out,
                r#"<text x="{x:.2}" y="{center_y:.2}" font-family="{FONT}" font-size="11" fill="{AXIS_TEXT}" dominant-baseline="middle">{}</text>"#,
                escape_xml(&entry.label)
            );
            // A fixed advance keeps SVG layout independent of the reader's font metrics.
            x += entry.label.chars().count() as f32 * 6.2 + 14.0;
        }
    }

    if let Some(title) = &scene.y_title {
        let legend = if scene.legend.is_empty() {
            0.0
        } else {
            LEGEND_HEIGHT
        };
        let _ = write!(
            out,
            r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="11" fill="{AXIS_TEXT}" dominant-baseline="middle">{}</text>"#,
            plot.x,
            plot.y - legend - TITLE_SPACE / 2.0,
            escape_xml(title)
        );
    }
    if let Some(title) = &scene.x_title {
        let _ = write!(
            out,
            r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="11" fill="{AXIS_TEXT}" text-anchor="middle">{}</text>"#,
            plot.x + plot.width / 2.0,
            plot.y + plot.height + 20.0,
            escape_xml(title)
        );
    }

    if has_axes {
        for tick in &scene.y_axis.ticks {
            let _ = write!(
                out,
                r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="11" fill="{AXIS_TEXT}" text-anchor="end" dominant-baseline="middle">{}</text>"#,
                plot.x - 6.0,
                tick.position,
                escape_xml(&tick.text)
            );
        }
        for tick in &scene.x_axis.ticks {
            let _ = write!(
                out,
                r#"<text x="{:.2}" y="{:.2}" font-family="{FONT}" font-size="11" fill="{AXIS_TEXT}" text-anchor="middle">{}</text>"#,
                tick.position,
                plot.y + plot.height + 15.0,
                escape_xml(&tick.text)
            );
        }
    }

    out.push_str("</svg>");
    out
}

/// A CSS hex colour from an RGBA array.
fn hex(color: [f32; 4]) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

/// An SVG `points` list for a polyline.
fn polyline(points: &[slickbi_scene::Point]) -> String {
    let mut out = String::new();
    for (index, point) in points.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{:.2},{:.2}", point.x, point.y));
    }
    out
}

/// An SVG `points` list for a filled area, closed down to the baseline.
fn area_polygon(points: &[slickbi_scene::Point], baseline: f32) -> String {
    let mut out = polyline(points);
    let last = points.last().expect("caller checks length");
    let first = &points[0];
    out.push_str(&format!(
        " {:.2},{baseline:.2} {:.2},{baseline:.2}",
        last.x, first.x
    ));
    out
}

/// An SVG path for a pie or donut wedge, walked in short segments like the rasteriser.
fn arc_path(slice: &slickbi_scene::ArcSlice) -> String {
    let sweep = slice.end - slice.start;
    let steps = ((sweep.abs() / std::f32::consts::TAU) * 90.0)
        .ceil()
        .max(2.0) as usize;
    let point = |angle: f32, radius: f32| {
        (
            slice.center.x + radius * angle.sin(),
            slice.center.y - radius * angle.cos(),
        )
    };
    let angle_at = |step: usize| slice.start + sweep * (step as f32 / steps as f32);

    let mut out = if slice.inner_radius <= 0.0 {
        format!("M {:.2} {:.2}", slice.center.x, slice.center.y)
    } else {
        let (x, y) = point(slice.start, slice.radius);
        format!("M {x:.2} {y:.2}")
    };

    let first = if slice.inner_radius <= 0.0 { 0 } else { 1 };
    for step in first..=steps {
        let (x, y) = point(angle_at(step), slice.radius);
        out.push_str(&format!(" L {x:.2} {y:.2}"));
    }
    if slice.inner_radius > 0.0 {
        for step in (0..=steps).rev() {
            let (x, y) = point(angle_at(step), slice.inner_radius);
            out.push_str(&format!(" L {x:.2} {y:.2}"));
        }
    }
    out.push_str(" Z");
    out
}

/// Escapes the characters that would break an XML text node or attribute.
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

struct Canvas {
    pixmap: Pixmap,
    scale: f32,
    font: FontArc,
}

impl Canvas {
    fn draw_axes(&mut self, scene: &Scene) {
        // A pie has no axes; the categories are named in the legend instead.
        if scene.x_axis.ticks.is_empty() && scene.y_axis.ticks.is_empty() {
            return;
        }

        let plot = scale_rect(scene.plot, self.scale);
        let stroke = Stroke {
            width: self.scale,
            ..Stroke::default()
        };

        // Gridlines at each y tick, then the axis border on top.
        for tick in &scene.y_axis.ticks {
            let y = tick.position * self.scale;
            self.line(plot.left(), y, plot.right(), y, &GRID, &stroke);
        }
        self.line(
            plot.left(),
            plot.top(),
            plot.right(),
            plot.top(),
            &AXIS,
            &stroke,
        );
        self.line(
            plot.left(),
            plot.bottom(),
            plot.right(),
            plot.bottom(),
            &AXIS,
            &stroke,
        );
        self.line(
            plot.left(),
            plot.top(),
            plot.left(),
            plot.bottom(),
            &AXIS,
            &stroke,
        );
        self.line(
            plot.right(),
            plot.top(),
            plot.right(),
            plot.bottom(),
            &AXIS,
            &stroke,
        );

        // Tick marks and labels.
        for tick in &scene.y_axis.ticks {
            self.y_tick(tick, plot);
        }
        for tick in &scene.x_axis.ticks {
            self.x_tick(tick, plot);
        }
    }

    fn y_tick(&mut self, tick: &Tick, plot: Rect) {
        let y = tick.position * self.scale;
        let stroke = Stroke {
            width: self.scale,
            ..Stroke::default()
        };
        self.line(
            plot.left() - 4.0 * self.scale,
            y,
            plot.left(),
            y,
            &AXIS,
            &stroke,
        );
        self.text(
            &tick.text,
            LABEL_SIZE,
            plot.left() - 7.0 * self.scale,
            y,
            &TEXT,
            Anchor::Right,
        );
    }

    fn x_tick(&mut self, tick: &Tick, plot: Rect) {
        let x = tick.position * self.scale;
        let stroke = Stroke {
            width: self.scale,
            ..Stroke::default()
        };
        self.line(
            x,
            plot.bottom(),
            x,
            plot.bottom() + 4.0 * self.scale,
            &AXIS,
            &stroke,
        );
        self.text(
            &tick.text,
            LABEL_SIZE,
            x,
            plot.bottom() + 7.0 * self.scale,
            &TEXT,
            Anchor::Center,
        );
    }

    fn draw_series(&mut self, scene: &Scene) {
        for series in &scene.series {
            match &series.kind {
                SeriesKind::Bars(bars) => {
                    for bar in bars {
                        self.bar(bar, series.color);
                    }
                }
                SeriesKind::Line(points) => self.line_series(points, series.color, false),
                SeriesKind::Area(points) => self.line_series(points, series.color, true),
                SeriesKind::Points(points) => {
                    for point in points {
                        self.circle(point.x, point.y, point.radius, series.color);
                    }
                }
                SeriesKind::Arc(slice) => self.arc(slice, series.color),
                SeriesKind::Text(text) => {
                    let x = scene.width * self.scale / 2.0;
                    let y = (scene.height / 2.0 - 18.0) * self.scale;
                    self.text(text, 30.0, x, y, &TEXT, Anchor::Center);
                }
            }
        }
    }

    /// Draws the data labels at their marks.
    fn draw_labels(&mut self, scene: &Scene) {
        for label in &scene.labels {
            self.text(
                &label.text,
                10.0,
                label.x * self.scale,
                label.y * self.scale,
                &TEXT,
                Anchor::Center,
            );
        }
    }

    /// Fills a pie wedge by walking the arc in short segments, which needs no arc primitive.
    fn arc(&mut self, slice: &ArcSlice, color: [f32; 4]) {
        let sweep = slice.end - slice.start;
        let steps = ((sweep.abs() / std::f32::consts::TAU) * 180.0)
            .ceil()
            .max(1.0) as usize;
        let point = |angle: f32, radius: f32| {
            (
                slice.center.x + radius * angle.sin(),
                slice.center.y - radius * angle.cos(),
            )
        };
        let angle_at = |step: usize| slice.start + sweep * (step as f32 / steps as f32);

        let mut path = PathBuilder::new();
        if slice.inner_radius <= 0.0 {
            path.move_to(slice.center.x * self.scale, slice.center.y * self.scale);
            for step in 0..=steps {
                let (x, y) = point(angle_at(step), slice.radius);
                path.line_to(x * self.scale, y * self.scale);
            }
        } else {
            let (x, y) = point(slice.start, slice.radius);
            path.move_to(x * self.scale, y * self.scale);
            for step in 1..=steps {
                let (x, y) = point(angle_at(step), slice.radius);
                path.line_to(x * self.scale, y * self.scale);
            }
            for step in (0..=steps).rev() {
                let (x, y) = point(angle_at(step), slice.inner_radius);
                path.line_to(x * self.scale, y * self.scale);
            }
        }
        path.close();
        if let Some(path) = path.finish() {
            self.pixmap.fill_path(
                &path,
                &solid(color),
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// Draws a horizontal legend across the top margin, one swatch and label per entry.
    fn draw_legend(&mut self, scene: &Scene) {
        if scene.legend.is_empty() {
            return;
        }
        let scale = self.scale;
        let swatch = 10.0 * scale;
        let mut x = scene.plot.x * scale;
        let center_y = (scene.plot.y - LEGEND_HEIGHT / 2.0) * scale;
        for entry in &scene.legend {
            if let Some(rect) = Rect::from_xywh(x, center_y - swatch / 2.0, swatch, swatch) {
                self.pixmap
                    .fill_rect(rect, &solid(entry.color), Transform::identity(), None);
            }
            x += swatch + 4.0 * scale;
            let width = self.text(
                &entry.label,
                LABEL_SIZE,
                x,
                center_y - LABEL_SIZE * scale / 2.0,
                &TEXT,
                Anchor::Left,
            );
            x += width + 14.0 * scale;
        }
    }

    /// Draws the axis titles: the y title above the plot and the x title below it.
    fn draw_titles(&mut self, scene: &Scene) {
        let plot = scene.plot;
        if let Some(title) = &scene.y_title {
            let legend = if scene.legend.is_empty() {
                0.0
            } else {
                LEGEND_HEIGHT
            };
            let center_y = (plot.y - legend - TITLE_SPACE / 2.0) * self.scale;
            self.text(
                title,
                LABEL_SIZE,
                plot.x * self.scale,
                center_y - LABEL_SIZE * self.scale / 2.0,
                &TEXT,
                Anchor::Left,
            );
        }
        if let Some(title) = &scene.x_title {
            let y = (plot.y + plot.height + 20.0) * self.scale;
            self.text(
                title,
                LABEL_SIZE,
                (plot.x + plot.width / 2.0) * self.scale,
                y,
                &TEXT,
                Anchor::Center,
            );
        }
    }

    /// Draws the free-tier watermark in the bottom-right corner.
    ///
    /// A preview is 1× and carries this badge; a Pro print export is unwatermarked and at 2–4×.
    fn draw_watermark(&mut self) {
        let size = 12.0;
        let padding = 8.0 * self.scale;
        let x = self.pixmap.width() as f32 - padding;
        let y = self.pixmap.height() as f32 - padding - size * self.scale;
        let color = [0.20, 0.24, 0.30, 0.35];
        self.text("SlickBI preview", size, x, y, &color, Anchor::Right);
    }

    fn bar(&mut self, bar: &Bar, color: [f32; 4]) {
        let rect = Rect::from_xywh(
            bar.x * self.scale,
            bar.y * self.scale,
            (bar.width * self.scale).max(1.0),
            (bar.height * self.scale).max(1.0),
        )
        .expect("valid rect");
        let paint = solid(color);
        self.pixmap
            .fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn line_series(&mut self, points: &[slickbi_scene::Point], color: [f32; 4], area: bool) {
        if points.is_empty() {
            return;
        }
        let mut path = PathBuilder::new();
        path.move_to(points[0].x * self.scale, points[0].y * self.scale);
        for point in &points[1..] {
            path.line_to(point.x * self.scale, point.y * self.scale);
        }

        if area {
            // Close down to the bottom of the plot (the baseline), then back to the start.
            path.line_to(
                points.last().expect("non-empty").x * self.scale,
                self.pixmap.height() as f32,
            );
            path.line_to(points[0].x * self.scale, self.pixmap.height() as f32);
            path.close();
        }

        let path = path.finish().expect("series path");

        if area {
            let mut fill = solid([color[0], color[1], color[2], 0.18]);
            fill.anti_alias = true;
            self.pixmap.fill_path(
                &path,
                &fill,
                tiny_skia::FillRule::EvenOdd,
                Transform::identity(),
                None,
            );
        }

        let stroke = Stroke {
            width: 1.5 * self.scale,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(&path, &solid(color), &stroke, Transform::identity(), None);
    }

    fn circle(&mut self, x: f32, y: f32, radius: f32, color: [f32; 4]) {
        let mut path = PathBuilder::new();
        path.push_circle(x * self.scale, y * self.scale, radius * self.scale);
        let circle = path.finish().expect("circle path");
        self.pixmap.fill_path(
            &circle,
            &solid(color),
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: &[f32; 4], stroke: &Stroke) {
        let mut path = PathBuilder::new();
        path.move_to(x0, y0);
        path.line_to(x1, y1);
        let line = path.finish().expect("line path");
        self.pixmap
            .stroke_path(&line, &solid(*color), stroke, Transform::identity(), None);
    }

    fn text(
        &mut self,
        text: &str,
        size: f32,
        x: f32,
        y: f32,
        color: &[f32; 4],
        anchor: Anchor,
    ) -> f32 {
        let px = size * self.scale;
        let scale = PxScale::from(px);
        let factor = px / self.font.height_unscaled();

        let mut glyphs = Vec::new();
        let mut pen = 0.0f32;
        let mut previous = None;
        for character in text.chars() {
            let id = self.font.glyph_id(character);
            if let Some(previous) = previous {
                pen += self.font.kern_unscaled(previous, id) * factor;
            }
            glyphs.push(Glyph {
                id,
                scale,
                position: ab_glyph::point(pen, 0.0),
            });
            pen += self.font.h_advance_unscaled(id) * factor;
            previous = Some(id);
        }

        let width = pen;
        let start = match anchor {
            Anchor::Center => x - width / 2.0,
            Anchor::Right => x - width,
            Anchor::Left => x,
        };
        let ascent = self.font.ascent_unscaled() * factor;
        let baseline = y + ascent;

        for glyph in glyphs {
            let pen_x = glyph.position.x;
            let Some(outlined) = self.font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            let gx = start + pen_x + bounds.min.x;
            let gy = baseline + bounds.min.y;
            if let Some(pixmap) = glyph_pixmap(&outlined, *color) {
                self.pixmap.draw_pixmap(
                    gx as i32,
                    gy as i32,
                    pixmap.as_ref(),
                    &PixmapPaint {
                        opacity: 1.0,
                        blend_mode: BlendMode::SourceOver,
                        quality: FilterQuality::Bilinear,
                    },
                    Transform::identity(),
                    None,
                );
            }
        }
        width
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Left,
    Center,
    Right,
}

fn glyph_pixmap(outlined: &OutlinedGlyph, color: [f32; 4]) -> Option<Pixmap> {
    let bounds = outlined.px_bounds();
    let width = (bounds.width().ceil() as u32).max(1);
    let height = (bounds.height().ceil() as u32).max(1);
    let mut data = vec![0u8; (width * height * 4) as usize];

    // tiny-skia pixmaps are premultiplied RGBA, so scale each channel by coverage times the
    // colour's alpha. A fully opaque colour (the usual case) is unchanged by the alpha factor.
    outlined.draw(|x, y, coverage| {
        if x < width && y < height {
            let index = ((y * width) + x) as usize * 4;
            let alpha = coverage * color[3];
            data[index] = (color[0] * 255.0 * alpha) as u8;
            data[index + 1] = (color[1] * 255.0 * alpha) as u8;
            data[index + 2] = (color[2] * 255.0 * alpha) as u8;
            data[index + 3] = (alpha * 255.0) as u8;
        }
    });

    Pixmap::from_vec(data, IntSize::from_wh(width, height)?)
}

fn solid(color: [f32; 4]) -> Paint<'static> {
    Paint {
        shader: tiny_skia::Shader::SolidColor(
            Color::from_rgba(color[0], color[1], color[2], color[3]).expect("valid color"),
        ),
        anti_alias: true,
        ..Paint::default()
    }
}

fn color(color: [f32; 4]) -> Color {
    Color::from_rgba(color[0], color[1], color[2], color[3]).expect("valid color")
}

fn scale_rect(rect: slickbi_scene::Rect, scale: f32) -> Rect {
    Rect::from_xywh(
        rect.x * scale,
        rect.y * scale,
        rect.width * scale,
        rect.height * scale,
    )
    .expect("valid rect")
}

/// Anything that can go wrong while rendering.
#[derive(Debug, Error)]
pub enum RenderError {
    /// PNG encoding failed.
    #[error("cannot encode the image: {0}")]
    Png(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Float64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use slickbi_planner::{ChannelKind, OutputColumn};
    use slickbi_scene::{Margins, SceneOptions, SceneSpec, build};
    use slickbi_spec::{DataType as SpecType, FieldId, Mark};
    use std::sync::Arc;

    fn column(kind: ChannelKind, name: &str) -> OutputColumn {
        OutputColumn {
            kind,
            name: name.to_owned(),
            field: FieldId::new(name),
            title: name.to_owned(),
            data_type: SpecType::Text,
            format: None,
            aggregate: false,
        }
    }

    fn bar_scene() -> Scene {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
        ]);
        let mut x = arrow::array::StringBuilder::new();
        for region in ["North", "South", "East", "West"] {
            x.append_value(region);
        }
        let y = Float64Array::from(vec![40.0, 80.0, 25.0, 55.0]);
        let batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x.finish()), Arc::new(y)])
            .expect("batch");
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let spec = SceneSpec {
            mark: Mark::Bar,
            columns: &columns,
            batches: &[batch],
            size: (400.0, 300.0),
            margins: Margins::default(),
            options: SceneOptions::default(),
        };
        build(&spec).expect("scene")
    }

    #[test]
    fn a_bar_scene_rasterises_to_a_nonempty_png() {
        let png = render_png(&bar_scene(), 2.0).expect("renders");
        assert!(png.len() > 1000, "a chart PNG should be more than a header");
        // PNG magic bytes.
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn the_pixmap_has_the_requested_size() {
        let pixmap = rasterize(&bar_scene(), 2.0);
        assert_eq!(pixmap.width(), 800);
        assert_eq!(pixmap.height(), 600);
    }

    #[test]
    fn bars_are_drawn_in_the_series_colour() {
        // The first Okabe–Ito palette colour, #0072b2, must appear: the bars were drawn.
        let pixmap = rasterize(&bar_scene(), 1.0);
        let found = pixmap.data().as_chunks::<4>().0.iter().any(|px| {
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            r < 30 && (g - 114).abs() < 30 && (b - 178).abs() < 30
        });
        assert!(found, "no pixels matched the series colour");
    }

    #[test]
    fn axis_labels_are_drawn() {
        let pixmap = rasterize(&bar_scene(), 1.0);
        let text_pixels = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| {
                let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
                (r - 61).abs() < 40 && (g - 71).abs() < 40 && (b - 84).abs() < 40
            })
            .count();
        assert!(
            text_pixels > 50,
            "axis labels should contribute text pixels"
        );
    }

    fn pie_scene() -> Scene {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
        ]);
        let mut x = arrow::array::StringBuilder::new();
        for region in ["North", "South", "East"] {
            x.append_value(region);
        }
        let y = Float64Array::from(vec![1.0, 2.0, 1.0]);
        let batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x.finish()), Arc::new(y)])
            .expect("batch");
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let spec = SceneSpec {
            mark: Mark::Arc,
            columns: &columns,
            batches: &[batch],
            size: (400.0, 300.0),
            margins: Margins::default(),
            options: SceneOptions::default(),
        };
        build(&spec).expect("scene")
    }

    #[test]
    fn a_pie_scene_rasterises_to_a_nonempty_png() {
        let png = render_png(&pie_scene(), 2.0).expect("renders");
        assert!(png.len() > 1000, "a pie PNG should be more than a header");
    }

    #[test]
    fn pie_slices_are_drawn_in_the_palette_colours() {
        // The first slice uses the first palette colour, #0072b2.
        let pixmap = rasterize(&pie_scene(), 1.0);
        let found = pixmap.data().as_chunks::<4>().0.iter().any(|px| {
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            r < 30 && (g - 114).abs() < 30 && (b - 178).abs() < 30
        });
        assert!(found, "no pixels matched the first slice colour");
    }

    #[test]
    fn a_preview_is_one_x_and_watermarked() {
        let scene = bar_scene();
        let plain = rasterize(&scene, 1.0);
        let preview = rasterize_preview(&scene);

        // A preview is 1×: the same pixels as a 1× render, plus the watermark.
        assert_eq!(preview.width(), plain.width());
        assert_eq!(preview.height(), plain.height());
        assert_ne!(
            preview.data(),
            plain.data(),
            "the watermark must change the image"
        );
    }

    #[test]
    fn a_preview_png_encodes() {
        let png = render_preview_png(&bar_scene()).expect("renders");
        assert!(png.len() > 1000);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn an_svg_document_has_bars_in_the_series_colour() {
        let svg = render_svg(&bar_scene());
        assert!(svg.starts_with("<svg"), "starts with an svg root");
        assert!(svg.ends_with("</svg>"));
        // Background, one rect per bar (four bars) and the plot border.
        assert!(svg.matches("<rect").count() >= 5, "expected bars as rects");
        assert!(svg.contains("#0072b2"), "bars use the first palette colour");
        assert!(svg.contains("North"), "x-axis labels are present");
    }

    #[test]
    fn an_svg_pie_draws_one_path_per_slice() {
        let svg = render_svg(&pie_scene());
        assert_eq!(svg.matches("<path").count(), 3, "one path per slice");
        assert!(svg.contains("North"));
        assert!(svg.contains("South"));
    }

    #[test]
    fn svg_text_is_escaped() {
        assert_eq!(
            escape_xml("a & b < c > \"d'"),
            "a &amp; b &lt; c &gt; &quot;d&apos;"
        );
    }
}
