#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! A software rasteriser for [`Scene`]s, used by `slickbi-cli render` and the image-diff harness.
//!
//! Shapes are drawn with `tiny-skia`; text is shaped and rasterised with `ab_glyph` using the
//! Inter font bundled in this crate, so the output does not depend on the host's installed fonts.
//! A scene is resolution-independent; [`rasterize`] renders it at a device scale into a pixmap,
//! and [`encode_png`] turns that into the bytes of a PNG file.

use ab_glyph::{Font, FontArc, Glyph, OutlinedGlyph, PxScale};
use slickbi_scene::{Bar, Scene, SeriesKind, Tick};
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

/// Rasterises and encodes in one step.
///
/// # Errors
///
/// Returns [`RenderError`] if the pixmap cannot be encoded.
pub fn render_png(scene: &Scene, scale: f32) -> Result<Vec<u8>, RenderError> {
    let pixmap = rasterize(scene, scale);
    encode_png(&pixmap)
}

struct Canvas {
    pixmap: Pixmap,
    scale: f32,
    font: FontArc,
}

impl Canvas {
    fn draw_axes(&mut self, scene: &Scene) {
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
                        self.circle(point.x, point.y, 2.5, series.color);
                    }
                }
            }
        }
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

    fn text(&mut self, text: &str, size: f32, x: f32, y: f32, color: &[f32; 4], anchor: Anchor) {
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Center,
    Right,
}

fn glyph_pixmap(outlined: &OutlinedGlyph, color: [f32; 4]) -> Option<Pixmap> {
    let bounds = outlined.px_bounds();
    let width = (bounds.width().ceil() as u32).max(1);
    let height = (bounds.height().ceil() as u32).max(1);
    let mut data = vec![0u8; (width * height * 4) as usize];

    // tiny-skia pixmaps are premultiplied RGBA, so scale each channel by the coverage.
    outlined.draw(|x, y, coverage| {
        if x < width && y < height {
            let index = ((y * width) + x) as usize * 4;
            data[index] = (color[0] * 255.0 * coverage) as u8;
            data[index + 1] = (color[1] * 255.0 * coverage) as u8;
            data[index + 2] = (color[2] * 255.0 * coverage) as u8;
            data[index + 3] = (coverage * 255.0) as u8;
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
    use slickbi_scene::{Margins, SceneSpec, build};
    use slickbi_spec::{DataType as SpecType, FieldId, Mark};
    use std::sync::Arc;

    fn column(kind: ChannelKind, name: &str) -> OutputColumn {
        OutputColumn {
            kind,
            name: name.to_owned(),
            field: FieldId::new(name),
            data_type: SpecType::Text,
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
        // The first palette colour, #0b6e99, must appear somewhere: the bars were actually drawn.
        let pixmap = rasterize(&bar_scene(), 1.0);
        let found = pixmap.data().chunks_exact(4).any(|px| {
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            (r - 11).abs() < 30 && (g - 110).abs() < 30 && (b - 153).abs() < 30
        });
        assert!(found, "no pixels matched the series colour");
    }

    #[test]
    fn axis_labels_are_drawn() {
        // Axis labels use a dark text colour; the plot area outside the bars must contain some.
        let pixmap = rasterize(&bar_scene(), 1.0);
        let text_pixels = pixmap
            .data()
            .chunks_exact(4)
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
}
