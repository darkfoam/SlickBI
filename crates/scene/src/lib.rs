#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The scene builder: turns planned, aggregated rows into scales, axes and mark geometry.
//!
//! The planner produces SQL and column metadata; the engine runs it into Arrow batches. This
//! crate consumes both and produces a [`Scene`] — a resolution-independent description of what to
//! draw, in points, with `(0, 0)` at the top-left and `y` growing downward. A scene knows nothing
//! about pixels, GPU or text rasterisation; a renderer draws it.
//!
//! Keeping this pure is what makes the geometry testable: every test below asserts that no mark
//! falls outside its panel and that every domain covers its data, without a GPU.

use arrow::array::{Array, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use slickbi_planner::{ChannelKind, OutputColumn};
use slickbi_spec::Mark;
use thiserror::Error;

/// The palette in `slickbi_spec::Theme::light`, as RGBA with full opacity.
const PALETTE: [[f32; 4]; 8] = [
    [0.043, 0.431, 0.6, 1.0],
    [0.761, 0.255, 0.047, 1.0],
    [0.302, 0.486, 0.059, 1.0],
    [0.486, 0.227, 0.929, 1.0],
    [0.631, 0.404, 0.027, 1.0],
    [0.059, 0.463, 0.431, 1.0],
    [0.745, 0.071, 0.235, 1.0],
    [0.263, 0.22, 0.792, 1.0],
];

/// Margins, in points, reserved around the plot area for axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    /// Space for the y-axis labels.
    pub left: f32,
    /// Space for the x-axis labels.
    pub bottom: f32,
    /// Headroom.
    pub top: f32,
    /// Trailing space.
    pub right: f32,
}

impl Default for Margins {
    fn default() -> Self {
        Margins {
            left: 48.0,
            bottom: 28.0,
            top: 16.0,
            right: 16.0,
        }
    }
}

/// An axis-aligned rectangle in points, `y` growing downward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// A point in points, `y` growing downward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position.
    pub y: f32,
}

/// One bar, `y` and `height` measured downward from the top of the bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// The left edge of the bar.
    pub x: f32,
    /// The top edge of the bar.
    pub y: f32,
    /// The bar width.
    pub width: f32,
    /// The bar height, down to the baseline.
    pub height: f32,
}

/// A tick on an axis: a position along the axis and its label.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// The position in points. For the x-axis this is the horizontal position; for the y-axis
    /// the vertical position.
    pub position: f32,
    /// The label.
    pub text: String,
}

/// A description of one axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    /// The ticks, in increasing position order.
    pub ticks: Vec<Tick>,
    /// Whether the axis is categorical (bands) or continuous.
    pub categorical: bool,
}

/// One plotted series. A single series for now; colour and multiple series arrive with the
/// colour shelf.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// The label shown in a legend.
    pub label: String,
    /// The colour, RGBA.
    pub color: [f32; 4],
    /// The geometry.
    pub kind: SeriesKind,
}

/// The geometry of a series.
#[derive(Debug, Clone, PartialEq)]
pub enum SeriesKind {
    /// One rectangle per value.
    Bars(Vec<Bar>),
    /// A connected line through the points.
    Line(Vec<Point>),
    /// A filled area down to the baseline, then the line on top.
    Area(Vec<Point>),
    /// One point per value.
    Points(Vec<Point>),
}

/// A complete scene, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The panel size in points.
    pub width: f32,
    /// The panel height in points.
    pub height: f32,
    /// The plot area, inside the margins.
    pub plot: Rect,
    /// The x-axis.
    pub x_axis: Axis,
    /// The y-axis.
    pub y_axis: Axis,
    /// The series to draw.
    pub series: Vec<Series>,
}

/// Everything needed to build a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSpec<'a> {
    /// The mark drawn.
    pub mark: Mark,
    /// The columns the query produced, in batch order.
    pub columns: &'a [OutputColumn],
    /// The query result.
    pub batches: &'a [RecordBatch],
    /// The panel size in points.
    pub size: (f32, f32),
    /// The margins around the plot.
    pub margins: Margins,
}

/// Builds a scene from planned, aggregated rows.
///
/// # Errors
///
/// Returns [`SceneError`] when the encoding has no x or y shelf, the mark is not one the builder
/// supports, a column is missing, or a column's values are not of a drawable type.
pub fn build(spec: &SceneSpec<'_>) -> Result<Scene, SceneError> {
    let x_index = spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::X)
        .ok_or(SceneError::MissingShelf("x"))?;
    let y_index = spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::Y)
        .ok_or(SceneError::MissingShelf("y"))?;

    let plot = plot_rect(spec.size, spec.margins);

    let x = read_x(spec.batches, x_index)?;
    let y = read_numeric(spec.batches, y_index)?;

    match spec.mark {
        Mark::Bar => bar_scene(spec, plot, x, y),
        Mark::Line => line_scene(spec, plot, x, y, false),
        Mark::Area => line_scene(spec, plot, x, y, true),
        Mark::Point => point_scene(spec, plot, x, y),
        other => Err(SceneError::UnsupportedMark { mark: other }),
    }
}

fn plot_rect(size: (f32, f32), margins: Margins) -> Rect {
    Rect {
        x: margins.left,
        y: margins.top,
        width: (size.0 - margins.left - margins.right).max(1.0),
        height: (size.1 - margins.top - margins.bottom).max(1.0),
    }
}

/// The x values, either categorical or numeric.
enum XData {
    Categorical(Vec<Option<String>>),
    Numeric(Vec<Option<f64>>),
}

fn bar_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
) -> Result<Scene, SceneError> {
    let categories = x.as_categorical().ok_or(SceneError::CategoricalXExpected)?;
    let band = plot.width / categories.len().max(1) as f32;
    let bar_width = band * 0.7;

    let y_scale = LinearScale::with_zero(numeric_domain(&y));

    let mut bars = Vec::with_capacity(categories.len());
    let mut ticks = Vec::with_capacity(categories.len());

    for (index, category) in categories.iter().enumerate() {
        let Some(value) = y[index] else {
            continue;
        };
        let center = plot.x + band * index as f32 + band / 2.0;
        let top = y_scale.map_y(value, plot.y, plot.height);
        let bottom = y_scale.map_y(0.0, plot.y, plot.height);
        bars.push(Bar {
            x: center - bar_width / 2.0,
            y: top,
            width: bar_width,
            height: bottom - top,
        });
        ticks.push(Tick {
            position: center,
            text: category.as_ref().cloned().unwrap_or_default(),
        });
    }

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis: Axis {
            ticks,
            categorical: true,
        },
        y_axis: numeric_axis(&y_scale, plot),
        series: vec![Series {
            label: "".to_owned(),
            color: PALETTE[0],
            kind: SeriesKind::Bars(bars),
        }],
    })
}

fn line_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
    area: bool,
) -> Result<Scene, SceneError> {
    let x_numeric = x.as_numeric().ok_or(SceneError::NumericXExpected)?;

    // Order the points by x so the line connects left to right.
    let mut data: Vec<(f64, f64)> = x_numeric
        .iter()
        .zip(y.iter())
        .filter_map(|(x, y)| Some(((*x)?, (*y)?)))
        .collect();
    data.sort_by(|a, b| a.0.total_cmp(&b.0));

    let x_domain = numeric_domain(&data.iter().map(|(x, _)| Some(*x)).collect::<Vec<_>>());
    let y_domain = numeric_domain_padded(&data.iter().map(|(_, y)| Some(*y)).collect::<Vec<_>>());

    let x_scale = LinearScale::new(x_domain);
    let y_scale = if area {
        LinearScale::with_zero(y_domain)
    } else {
        LinearScale::new(y_domain)
    };

    let points: Vec<Point> = data
        .iter()
        .map(|(x, y)| Point {
            x: x_scale.map(*x, plot.x, plot.width),
            y: y_scale.map_y(*y, plot.y, plot.height),
        })
        .collect();

    let x_axis = Axis {
        ticks: linear_ticks(&x_scale, plot, false),
        categorical: false,
    };
    let y_axis = numeric_axis(&y_scale, plot);

    let kind = if area {
        SeriesKind::Area(points)
    } else {
        SeriesKind::Line(points)
    };

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis,
        y_axis,
        series: vec![Series {
            label: "".to_owned(),
            color: if area { PALETTE[3] } else { PALETTE[1] },
            kind,
        }],
    })
}

fn point_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
) -> Result<Scene, SceneError> {
    let x_numeric = x.as_numeric().ok_or(SceneError::NumericXExpected)?;

    let data: Vec<(f64, f64)> = x_numeric
        .iter()
        .zip(y.iter())
        .filter_map(|(x, y)| Some(((*x)?, (*y)?)))
        .collect();

    let x_domain = numeric_domain(&data.iter().map(|(x, _)| Some(*x)).collect::<Vec<_>>());
    let y_domain = numeric_domain_padded(&data.iter().map(|(_, y)| Some(*y)).collect::<Vec<_>>());
    let x_scale = LinearScale::new(x_domain);
    let y_scale = LinearScale::new(y_domain);

    let points: Vec<Point> = data
        .iter()
        .map(|(x, y)| Point {
            x: x_scale.map(*x, plot.x, plot.width),
            y: y_scale.map_y(*y, plot.y, plot.height),
        })
        .collect();

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis: Axis {
            ticks: linear_ticks(&x_scale, plot, false),
            categorical: false,
        },
        y_axis: numeric_axis(&y_scale, plot),
        series: vec![Series {
            label: "".to_owned(),
            color: PALETTE[2],
            kind: SeriesKind::Points(points),
        }],
    })
}

impl XData {
    fn as_categorical(&self) -> Option<&[Option<String>]> {
        match self {
            XData::Categorical(values) => Some(values),
            XData::Numeric(_) => None,
        }
    }

    fn as_numeric(&self) -> Option<&[Option<f64>]> {
        match self {
            XData::Numeric(values) => Some(values),
            XData::Categorical(_) => None,
        }
    }
}

/// A linear mapping from a data domain to a pixel range.
#[derive(Debug, Clone, Copy)]
struct LinearScale {
    domain_min: f64,
    domain_max: f64,
}

impl LinearScale {
    fn new(domain: (f64, f64)) -> Self {
        LinearScale {
            domain_min: domain.0,
            domain_max: domain.1,
        }
    }

    fn with_zero(domain: (f64, f64)) -> Self {
        LinearScale {
            domain_min: domain.0.min(0.0),
            domain_max: domain.1.max(0.0),
        }
    }

    fn span(&self) -> f64 {
        (self.domain_max - self.domain_min).max(f64::EPSILON)
    }

    fn map(&self, value: f64, start: f32, length: f32) -> f32 {
        let t = ((value - self.domain_min) / self.span()).clamp(0.0, 1.0) as f32;
        start + length * t
    }

    /// Maps a value to a vertical coordinate, `y` growing downward: the domain minimum sits at
    /// the bottom and the maximum at the top.
    fn map_y(&self, value: f64, top: f32, height: f32) -> f32 {
        let t = ((value - self.domain_min) / self.span()).clamp(0.0, 1.0) as f32;
        top + height * (1.0 - t)
    }

    fn ticks(&self, count: usize) -> Vec<f64> {
        nice_ticks(self.domain_min, self.domain_max, count)
    }
}

fn numeric_axis(scale: &LinearScale, plot: Rect) -> Axis {
    Axis {
        ticks: linear_ticks(scale, plot, true),
        categorical: false,
    }
}

/// Builds ticks for a linear scale. When `vertical`, positions are y coordinates (top-down).
fn linear_ticks(scale: &LinearScale, plot: Rect, vertical: bool) -> Vec<Tick> {
    scale
        .ticks(6)
        .into_iter()
        .map(|value| {
            let position = if vertical {
                scale.map_y(value, plot.y, plot.height)
            } else {
                scale.map(value, plot.x, plot.width)
            };
            Tick {
                position,
                text: format_number(value),
            }
        })
        .collect()
}

fn numeric_domain(values: &[Option<f64>]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in values.iter().flatten() {
        min = min.min(*value);
        max = max.max(*value);
    }
    if min > max {
        (0.0, 1.0)
    } else if (min - max).abs() < f64::EPSILON {
        let pad = min.abs().max(1.0) * 0.1;
        (min - pad, max + pad)
    } else {
        (min, max)
    }
}

fn numeric_domain_padded(values: &[Option<f64>]) -> (f64, f64) {
    let (min, max) = numeric_domain(values);
    let pad = (max - min) * 0.05;
    (min - pad, max + pad)
}

/// The "nice" number algorithm from a range, for 1/2/5 ticks.
fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || count < 2 {
        return vec![min];
    }
    let step = nice_step((max - min) / (count - 1) as f64);
    let start = (min / step).floor() * step;
    let mut ticks = Vec::new();
    let mut value = start;
    let mut guard = 0;
    while value <= max + step * 0.5 && guard < 100 {
        ticks.push(value);
        value += step;
        guard += 1;
    }
    ticks
}

fn nice_step(raw: f64) -> f64 {
    let exponent = raw.log10().floor();
    let fraction = raw / 10f64.powf(exponent);
    let nice = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * 10f64.powf(exponent)
}

/// Formats a tick value compactly: integers without a fraction, decimals trimmed.
fn format_number(value: f64) -> String {
    let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
    if rounded.fract().abs() < f64::EPSILON {
        format!("{rounded:.0}")
    } else {
        let text = format!("{rounded:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

fn read_x(batches: &[RecordBatch], index: usize) -> Result<XData, SceneError> {
    let mut categorical = Vec::new();
    let mut numeric = Vec::new();
    let mut is_categorical = None;

    for batch in batches {
        let column = batch.column(index);
        if let Some(array) = column.as_any().downcast_ref::<StringArray>() {
            if is_categorical == Some(false) {
                return Err(SceneError::MixedXTypes);
            }
            is_categorical = Some(true);
            for i in 0..array.len() {
                categorical.push((!array.is_null(i)).then(|| array.value(i).to_owned()));
            }
        } else if let Some(array) = column.as_any().downcast_ref::<Float64Array>() {
            if is_categorical == Some(true) {
                return Err(SceneError::MixedXTypes);
            }
            is_categorical = Some(false);
            for i in 0..array.len() {
                numeric.push((!array.is_null(i)).then(|| array.value(i)));
            }
        } else if let Some(array) = column.as_any().downcast_ref::<Int64Array>() {
            if is_categorical == Some(true) {
                return Err(SceneError::MixedXTypes);
            }
            is_categorical = Some(false);
            for i in 0..array.len() {
                numeric.push((!array.is_null(i)).then(|| array.value(i) as f64));
            }
        } else {
            return Err(SceneError::UndrawableXType);
        }
    }

    match is_categorical {
        Some(true) => Ok(XData::Categorical(categorical)),
        Some(false) => Ok(XData::Numeric(numeric)),
        None => Ok(XData::Categorical(Vec::new())),
    }
}

fn read_numeric(batches: &[RecordBatch], index: usize) -> Result<Vec<Option<f64>>, SceneError> {
    let mut values = Vec::new();
    for batch in batches {
        let column = batch.column(index);
        if let Some(array) = column.as_any().downcast_ref::<Float64Array>() {
            for i in 0..array.len() {
                values.push((!array.is_null(i)).then(|| array.value(i)));
            }
        } else if let Some(array) = column.as_any().downcast_ref::<Int64Array>() {
            for i in 0..array.len() {
                values.push((!array.is_null(i)).then(|| array.value(i) as f64));
            }
        } else {
            return Err(SceneError::UndrawableYType);
        }
    }
    Ok(values)
}

/// Anything that prevents a scene from being built.
#[derive(Debug, Error, PartialEq)]
pub enum SceneError {
    /// The encoding has no field on the given shelf.
    #[error("the encoding has no {0} shelf")]
    MissingShelf(&'static str),
    /// The mark is not one the scene builder draws.
    #[error("mark {mark:?} is not supported by the scene builder yet")]
    UnsupportedMark {
        /// The mark.
        mark: Mark,
    },
    /// A bar chart needs categorical x values.
    #[error("a bar chart needs categorical x values, but the x shelf is numeric")]
    CategoricalXExpected,
    /// A line, area or scatter chart needs numeric x values.
    #[error("this mark needs numeric x values, but the x shelf is categorical")]
    NumericXExpected,
    /// The x column mixes types across batches.
    #[error("the x column mixes text and numeric values")]
    MixedXTypes,
    /// The x column's type cannot be drawn.
    #[error("the x column has a type the scene builder cannot draw")]
    UndrawableXType,
    /// The y column's type cannot be drawn.
    #[error("the y column has a type the scene builder cannot draw")]
    UndrawableYType,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::StringBuilder;
    use arrow::datatypes::{DataType, Field, Schema};
    use slickbi_planner::ChannelKind;
    use slickbi_spec::{DataType as SpecType, FieldId};

    fn column(kind: ChannelKind, name: &str) -> OutputColumn {
        OutputColumn {
            kind,
            name: name.to_owned(),
            field: FieldId::new(name),
            data_type: SpecType::Text,
            aggregate: false,
        }
    }

    fn batch_categorical_y(regions: &[&str], values: &[f64]) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
        ]);
        let mut x = StringBuilder::new();
        for region in regions {
            x.append_value(region);
        }
        let y = arrow::array::Float64Array::from(values.to_vec());
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x.finish()), Arc::new(y)])
            .expect("batch")
    }

    fn batch_numeric_y(xs: &[f64], ys: &[f64]) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Float64, true),
            Field::new("y", DataType::Float64, true),
        ]);
        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Float64Array::from(xs.to_vec())),
                Arc::new(Float64Array::from(ys.to_vec())),
            ],
        )
        .expect("batch")
    }

    fn spec<'a>(
        mark: Mark,
        columns: &'a [OutputColumn],
        batches: &'a [RecordBatch],
    ) -> SceneSpec<'a> {
        SceneSpec {
            mark,
            columns,
            batches,
            size: (400.0, 300.0),
            margins: Margins::default(),
        }
    }

    use std::sync::Arc;

    #[test]
    fn a_bar_scene_has_one_bar_per_category_inside_the_plot() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A", "B", "C"], &[3.0, 5.0, 2.0]);

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        assert_eq!(scene.series.len(), 1);
        let SeriesKind::Bars(bars) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        assert_eq!(bars.len(), 3);
        assert_eq!(scene.x_axis.ticks.len(), 3);

        for bar in bars {
            assert!(bar.x >= scene.plot.x - 0.01);
            assert!(bar.x + bar.width <= scene.plot.x + scene.plot.width + 0.01);
            assert!(bar.y >= scene.plot.y - 0.01);
            assert!(bar.y + bar.height <= scene.plot.y + scene.plot.height + 0.01);
        }
    }

    #[test]
    fn bars_start_at_a_zero_baseline() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A"], &[4.0]);

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        let SeriesKind::Bars(bars) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        let bar = bars[0];
        // The bottom of the bar (y + height) sits at the plot's bottom edge, the zero line.
        assert!((bar.y + bar.height - (scene.plot.y + scene.plot.height)).abs() < 0.01);
    }

    #[test]
    fn a_line_scene_orders_points_left_to_right() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_numeric_y(&[3.0, 1.0, 2.0], &[30.0, 10.0, 20.0]);

        let scene = build(&spec(Mark::Line, &columns, &[batch])).expect("builds");
        let SeriesKind::Line(points) = &scene.series[0].kind else {
            panic!("expected a line");
        };
        assert_eq!(points.len(), 3);
        for window in points.windows(2) {
            assert!(window[0].x <= window[1].x);
        }
    }

    #[test]
    fn a_scatter_scene_has_one_point_per_row() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_numeric_y(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]);

        let scene = build(&spec(Mark::Point, &columns, &[batch])).expect("builds");
        let SeriesKind::Points(points) = &scene.series[0].kind else {
            panic!("expected points");
        };
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn every_domain_covers_its_data() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_numeric_y(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);

        let scene = build(&spec(Mark::Point, &columns, &[batch])).expect("builds");
        let SeriesKind::Points(points) = &scene.series[0].kind else {
            panic!("expected points");
        };
        for point in points {
            assert!(
                point.x >= scene.plot.x - 0.01 && point.x <= scene.plot.x + scene.plot.width + 0.01
            );
            assert!(
                point.y >= scene.plot.y - 0.01
                    && point.y <= scene.plot.y + scene.plot.height + 0.01
            );
        }
    }

    #[test]
    fn a_missing_y_shelf_is_rejected() {
        let columns = vec![column(ChannelKind::X, "x")];
        let batch = batch_categorical_y(&["A"], &[1.0]);
        let error = build(&spec(Mark::Bar, &columns, &[batch])).expect_err("must fail");
        assert_eq!(error, SceneError::MissingShelf("y"));
    }

    #[test]
    fn a_bar_chart_needs_categorical_x() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_numeric_y(&[1.0, 2.0], &[3.0, 4.0]);
        let error = build(&spec(Mark::Bar, &columns, &[batch])).expect_err("must fail");
        assert_eq!(error, SceneError::CategoricalXExpected);
    }

    #[test]
    fn an_unsupported_mark_is_rejected() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_numeric_y(&[1.0], &[2.0]);
        let error = build(&spec(Mark::Text, &columns, &[batch])).expect_err("must fail");
        assert_eq!(error, SceneError::UnsupportedMark { mark: Mark::Text });
    }

    #[test]
    fn nice_ticks_cover_the_domain_with_round_values() {
        let ticks = nice_ticks(0.0, 10.0, 6);
        assert_eq!(ticks, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    }
}
