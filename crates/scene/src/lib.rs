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

use std::collections::HashMap;

use arrow::array::{Array, Float64Array, StringArray};
use arrow::datatypes::DataType as ArrowDataType;
use arrow::record_batch::RecordBatch;
use slickbi_planner::{ChannelKind, OutputColumn};
use slickbi_spec::{FieldFormat, Mark};
use thiserror::Error;

/// The palette in `slickbi_spec::Theme::light`, as RGBA with full opacity.
///
/// This is the Okabe–Ito colourblind-safe categorical palette. It must stay identical to the
/// theme's palette; a test below fails if the two drift.
const PALETTE: [[f32; 4]; 8] = [
    [0.0, 0.4471, 0.6980, 1.0],    // #0072b2 blue
    [0.9020, 0.6235, 0.0, 1.0],    // #e69f00 orange
    [0.0, 0.6196, 0.4510, 1.0],    // #009e73 bluish green
    [0.8, 0.4745, 0.6549, 1.0],    // #cc79a7 reddish purple
    [0.8353, 0.3686, 0.0, 1.0],    // #d55e00 vermillion
    [0.3373, 0.7059, 0.9137, 1.0], // #56b4e9 sky blue
    [0.9412, 0.8941, 0.2588, 1.0], // #f0e442 yellow
    [0.6, 0.6, 0.6, 1.0],          // #999999 grey
];

/// The vertical space reserved above the plot for a legend, in points.
pub const LEGEND_HEIGHT: f32 = 24.0;

/// The vertical space reserved for an axis title, in points.
pub const TITLE_SPACE: f32 = 16.0;

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

/// A scatter point with its own radius, so a size shelf can vary it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatterPoint {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position.
    pub y: f32,
    /// The drawn radius.
    pub radius: f32,
}

/// A data label drawn at a mark.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// Horizontal position of the label's centre.
    pub x: f32,
    /// Vertical position of the label's baseline.
    pub y: f32,
    /// The text.
    pub text: String,
}

/// A hover target: where a mark sits, its hit radius, and the tooltip text.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipMark {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position.
    pub y: f32,
    /// How close the pointer must be, in points.
    pub radius: f32,
    /// The tooltip text, with one line per configured field.
    pub text: String,
}

/// A table of formatted cells, with a header row.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The column titles.
    pub headers: Vec<String>,
    /// The rows, one cell per column.
    pub rows: Vec<Vec<String>>,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Total width.
    pub width: f32,
    /// Width of one column.
    pub column_width: f32,
    /// Height of the header row.
    pub header_height: f32,
    /// Height of each body row.
    pub row_height: f32,
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

/// One cell of a heatmap: a rectangle with its own sequential colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatCell {
    /// The left edge of the cell.
    pub x: f32,
    /// The top edge of the cell.
    pub y: f32,
    /// The cell width.
    pub width: f32,
    /// The cell height.
    pub height: f32,
    /// The fill colour, RGBA, mapped from the value.
    pub color: [f32; 4],
}

/// One wedge of a pie chart, in points and radians.
///
/// Angles are measured clockwise from twelve o'clock: at `0` the point is directly above the
/// centre, at `π/2` directly to its right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcSlice {
    /// The centre of the pie.
    pub center: Point,
    /// The outer radius.
    pub radius: f32,
    /// The inner radius of a donut, or `0.0` for a full pie slice.
    pub inner_radius: f32,
    /// The start angle, in radians.
    pub start: f32,
    /// The end angle, in radians. Greater than `start`.
    pub end: f32,
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

/// One plotted series: a set of marks sharing a colour and a legend label.
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
    /// One coloured cell per category pair, for a heatmap.
    Heat(Vec<HeatCell>),
    /// A connected line through the points.
    Line(Vec<Point>),
    /// A filled area down to the baseline, then the line on top.
    Area(Vec<Point>),
    /// One point per value.
    Points(Vec<ScatterPoint>),
    /// One wedge of a pie chart.
    Arc(ArcSlice),
    /// A single formatted value, for a KPI tile.
    Text(String),
}

/// One entry in a legend: the swatch colour and the series it names.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    /// The series label.
    pub label: String,
    /// The swatch colour, RGBA.
    pub color: [f32; 4],
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
    /// Legend entries, when the chart splits into several series. Empty for a single series.
    pub legend: Vec<LegendEntry>,
    /// The horizontal axis title, when the chart shows axes.
    pub x_title: Option<String>,
    /// The vertical axis title, when the chart shows axes.
    pub y_title: Option<String>,
    /// Data labels drawn at marks.
    pub labels: Vec<Label>,
    /// Hover targets with their tooltip text.
    pub tooltips: Vec<TooltipMark>,
    /// A table, when the mark is a table.
    pub table: Option<Table>,
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
    /// Presentation options that do not change what the chart measures.
    pub options: SceneOptions,
}

/// Presentation options a report element may override without changing the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneOptions {
    /// Whether to draw axes and reserve space for their labels.
    pub show_axes: bool,
    /// Whether to draw a legend when the chart splits into several series.
    pub show_legend: bool,
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions {
            show_axes: true,
            show_legend: true,
        }
    }
}

/// Builds a scene from planned, aggregated rows.
///
/// # Errors
///
/// Returns [`SceneError`] when the encoding has no x or y shelf, the mark is not one the builder
/// supports, a column is missing, or a column's values are not of a drawable type.
pub fn build(spec: &SceneSpec<'_>) -> Result<Scene, SceneError> {
    // A KPI tile is a single value: it needs no x or y shelf.
    if spec.mark == Mark::Text {
        return text_scene(spec);
    }
    // A table lists whatever columns the shelves produced: it needs no x or y shelf either.
    if spec.mark == Mark::Table {
        return table_scene(spec);
    }

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
    let color_index = spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::Color);

    // A pie colours by category and ignores the colour shelf, but still needs legend space.
    let pie = matches!(spec.mark, Mark::Arc | Mark::Donut);
    let has_color = !pie && spec.mark != Mark::Rect && color_index.is_some();
    let show_axes = spec.options.show_axes && !pie;
    let show_legend = spec.options.show_legend && (has_color || pie);

    let x_title = if show_axes {
        column_title(spec.columns, ChannelKind::X)
    } else {
        None
    };
    let y_title = if show_axes {
        column_title(spec.columns, ChannelKind::Y)
    } else {
        None
    };

    // A pie has no axes; a chart with its axes hidden gets uniform, tighter margins. Axis titles
    // and the legend each reserve a line in the surrounding margins.
    let mut margins = if show_axes {
        spec.margins
    } else {
        Margins {
            left: 16.0,
            right: 16.0,
            bottom: 16.0,
            top: spec.margins.top.max(16.0),
        }
    };
    if y_title.is_some() {
        margins.top += TITLE_SPACE;
    }
    if show_legend {
        margins.top += LEGEND_HEIGHT;
    }
    if x_title.is_some() {
        margins.bottom += TITLE_SPACE;
    }
    let plot = plot_rect(spec.size, margins);

    let x = read_axis(
        spec.batches,
        x_index,
        false,
        column_format(spec.columns, ChannelKind::X),
    )?;
    let y = read_axis(
        spec.batches,
        y_index,
        true,
        column_format(spec.columns, ChannelKind::Y),
    )?;
    let color = color_index
        .map(|index| read_labels(spec.batches, index))
        .transpose()?;

    let size_values = match spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::Size)
    {
        Some(index) => Some(
            read_axis(spec.batches, index, true, None)?
                .into_numeric()
                .ok_or(SceneError::UndrawableYType)?,
        ),
        None => None,
    };
    let text_values = spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::Text)
        .map(|index| read_labels(spec.batches, index))
        .transpose()?;
    let tooltip_values: Vec<(String, Vec<Option<String>>)> = spec
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| matches!(column.kind, ChannelKind::Tooltip(_)))
        .map(|(index, column)| Ok((column.title.clone(), read_labels(spec.batches, index)?)))
        .collect::<Result<_, SceneError>>()?;
    let size_domain = size_values.as_deref().map(numeric_domain);
    let marks = Marks {
        size: size_values.as_deref(),
        text: text_values.as_deref(),
        tooltip: &tooltip_values,
        x_title: x_title.as_deref().unwrap_or(""),
        y_title: y_title.as_deref().unwrap_or(""),
        y_format: column_format(spec.columns, ChannelKind::Y),
        size_domain,
    };

    // Bars read either axis as the categories; the other marks need a numeric y.
    let numeric_y = |y: XData| y.into_numeric().ok_or(SceneError::UndrawableYType);

    let mut scene = match spec.mark {
        Mark::Bar => bar_scene(spec, plot, x, y, color, &marks),
        Mark::Line => line_scene(spec, plot, x, numeric_y(y)?, false, color),
        Mark::Area => line_scene(spec, plot, x, numeric_y(y)?, true, color),
        Mark::Point => point_scene(spec, plot, x, numeric_y(y)?, color, &marks),
        Mark::Rect => heat_scene(spec, plot, x, y, &marks),
        Mark::Arc => arc_scene(spec, plot, x, numeric_y(y)?, false),
        Mark::Donut => arc_scene(spec, plot, x, numeric_y(y)?, true),
        other => Err(SceneError::UnsupportedMark { mark: other }),
    }?;

    if !show_axes {
        scene.x_axis = Axis {
            ticks: Vec::new(),
            categorical: scene.x_axis.categorical,
        };
        scene.y_axis = Axis {
            ticks: Vec::new(),
            categorical: scene.y_axis.categorical,
        };
    }

    if show_legend {
        scene.legend = scene
            .series
            .iter()
            .map(|series| LegendEntry {
                label: series.label.clone(),
                color: series.color,
            })
            .collect();
    }

    scene.x_title = x_title;
    scene.y_title = y_title;

    Ok(scene)
}

fn plot_rect(size: (f32, f32), margins: Margins) -> Rect {
    Rect {
        x: margins.left,
        y: margins.top,
        width: (size.0 - margins.left - margins.right).max(1.0),
        height: (size.1 - margins.top - margins.bottom).max(1.0),
    }
}

/// The display format declared on the column from a given shelf, if any.
fn column_format(columns: &[OutputColumn], kind: ChannelKind) -> Option<&FieldFormat> {
    columns
        .iter()
        .find(|column| column.kind == kind)
        .and_then(|column| column.format.as_ref())
}

/// The display title of the column from a given shelf, if it has one.
fn column_title(columns: &[OutputColumn], kind: ChannelKind) -> Option<String> {
    columns
        .iter()
        .find(|column| column.kind == kind)
        .map(|column| column.title.clone())
        .filter(|title| !title.is_empty())
}

/// The x values: categorical labels, numeric values, or a temporal column that is both.
///
/// A temporal column carries its numeric ticks (days or milliseconds since the epoch) so a line,
/// area or point chart can put it on a continuous axis, and the formatted labels so a bar, pie or
/// table can treat it as categories.
enum XData {
    Categorical(Vec<Option<String>>),
    Numeric(Vec<Option<f64>>),
    Temporal {
        values: Vec<Option<f64>>,
        labels: Vec<Option<String>>,
        unit: TemporalUnit,
    },
}

/// Extra encodings that decorate marks: size, data labels and tooltips.
struct Marks<'a> {
    size: Option<&'a [Option<f64>]>,
    text: Option<&'a [Option<String>]>,
    tooltip: &'a [(String, Vec<Option<String>>)],
    x_title: &'a str,
    y_title: &'a str,
    y_format: Option<&'a FieldFormat>,
    size_domain: Option<(f64, f64)>,
}

impl Marks<'_> {
    /// The drawn radius for one row: 2.5 by default, scaled 3–12 by the size shelf.
    fn radius(&self, row: usize) -> f32 {
        let Some(values) = self.size else {
            return 2.5;
        };
        let Some(value) = values.get(row).copied().flatten() else {
            return 2.5;
        };
        let Some((low, high)) = self.size_domain else {
            return 2.5;
        };
        let span = (high - low).max(f64::EPSILON);
        let t = ((value - low) / span).clamp(0.0, 1.0) as f32;
        3.0 + 9.0 * t
    }

    /// The data label for one row, if the text shelf has a value.
    fn label(&self, row: usize) -> Option<String> {
        self.text
            .and_then(|values| values.get(row).cloned().flatten())
            .filter(|text| !text.is_empty())
    }

    /// The tooltip text for one row: the axis values plus every configured tooltip field.
    fn tooltip_text(&self, row: usize, x: Option<String>, y: Option<f64>) -> String {
        let mut lines = Vec::new();
        if let Some(x) = x.filter(|x| !x.is_empty()) {
            lines.push(format!("{}: {x}", self.x_title));
        }
        if let Some(y) = y {
            lines.push(format!(
                "{}: {}",
                self.y_title,
                format_number(y, self.y_format)
            ));
        }
        for (title, values) in self.tooltip {
            if let Some(value) = values.get(row).cloned().flatten()
                && !value.is_empty()
            {
                lines.push(format!("{title}: {value}"));
            }
        }
        lines.join("\n")
    }
}

fn bar_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: XData,
    color: Option<Vec<Option<String>>>,
    marks: &Marks,
) -> Result<Scene, SceneError> {
    // One axis is categorical (the categories) and the other numeric (the values). Categories on
    // x give vertical columns; categories on y give horizontal bars. A temporal column is a set of
    // categories for a bar; a date is never a bar's value axis.
    let (vertical, categories, values) = if let Some(categories) = x.as_categories() {
        let values = y.as_numeric().ok_or(SceneError::NumericYExpected)?.to_vec();
        (true, categories.to_vec(), values)
    } else if let Some(categories) = y.as_categories() {
        if x.temporal_unit().is_some() {
            return Err(SceneError::NumericXExpected);
        }
        let values = x
            .as_numeric()
            .ok_or(SceneError::CategoricalXExpected)?
            .to_vec();
        (false, categories.to_vec(), values)
    } else {
        return Err(SceneError::CategoricalXExpected);
    };

    let (distinct, category_of) = dedup(&categories);
    let groups = series_groups(&color, values.len());

    let band = if vertical {
        plot.width / distinct.len().max(1) as f32
    } else {
        plot.height / distinct.len().max(1) as f32
    };
    let sub_band = band / groups.len().max(1) as f32;
    let thickness = sub_band * 0.7;

    let scale = LinearScale::with_zero(numeric_domain(&values));

    let mut series = Vec::with_capacity(groups.len());
    let mut labels = Vec::new();
    let mut tooltips = Vec::new();
    for (index, (label, rows)) in groups.iter().enumerate() {
        let mut bars = Vec::new();
        for &row in rows {
            let Some(value) = values.get(row).copied().flatten() else {
                continue;
            };
            let category = category_of[row];
            let category_text = distinct[category].clone().unwrap_or_default();
            let (bar, label_x, label_y, tip_x, tip_y) = if vertical {
                let center =
                    plot.x + band * category as f32 + sub_band * index as f32 + sub_band / 2.0;
                let top = scale.map_y(value, plot.y, plot.height);
                let bottom = scale.map_y(0.0, plot.y, plot.height);
                (
                    Bar {
                        x: center - thickness / 2.0,
                        y: top,
                        width: thickness,
                        height: bottom - top,
                    },
                    center,
                    top - 4.0,
                    center,
                    top + (bottom - top) / 2.0,
                )
            } else {
                let center =
                    plot.y + band * category as f32 + sub_band * index as f32 + sub_band / 2.0;
                let start = scale.map(0.0, plot.x, plot.width);
                let end = scale.map(value, plot.x, plot.width);
                let bar = Bar {
                    x: start.min(end),
                    y: center - thickness / 2.0,
                    width: (end - start).abs(),
                    height: thickness,
                };
                (
                    bar,
                    bar.x + bar.width / 2.0,
                    center - thickness / 2.0 - 4.0,
                    bar.x + bar.width / 2.0,
                    center,
                )
            };
            bars.push(bar);
            if let Some(text) = marks.label(row) {
                labels.push(Label {
                    x: label_x,
                    y: label_y,
                    text,
                });
            }
            tooltips.push(TooltipMark {
                x: tip_x,
                y: tip_y,
                radius: (thickness / 2.0).max(7.0),
                text: marks.tooltip_text(row, Some(category_text), Some(value)),
            });
        }
        series.push(Series {
            label: label.clone(),
            color: PALETTE[index % PALETTE.len()],
            kind: SeriesKind::Bars(bars),
        });
    }

    let (x_axis, y_axis) = if vertical {
        let value_format = column_format(spec.columns, ChannelKind::Y);
        let ticks = distinct
            .iter()
            .enumerate()
            .map(|(index, category)| Tick {
                position: plot.x + band * index as f32 + band / 2.0,
                text: category.as_ref().cloned().unwrap_or_default(),
            })
            .collect();
        (
            Axis {
                ticks,
                categorical: true,
            },
            numeric_axis(&scale, plot, value_format),
        )
    } else {
        let value_format = column_format(spec.columns, ChannelKind::X);
        let ticks = distinct
            .iter()
            .enumerate()
            .map(|(index, category)| Tick {
                position: plot.y + band * index as f32 + band / 2.0,
                text: category.as_ref().cloned().unwrap_or_default(),
            })
            .collect();
        (
            Axis {
                ticks: linear_ticks(&scale, plot, false, value_format),
                categorical: false,
            },
            Axis {
                ticks,
                categorical: true,
            },
        )
    };

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis,
        y_axis,
        series,
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels,
        tooltips,
        table: None,
    })
}

fn line_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
    area: bool,
    color: Option<Vec<Option<String>>>,
) -> Result<Scene, SceneError> {
    let labels = Vec::new();
    let tooltips = Vec::new();
    let x_numeric = x.as_numeric().ok_or(SceneError::NumericXExpected)?;
    let x_unit = x.temporal_unit();

    let groups = series_groups(&color, y.len());

    // One domain across every series so grouped lines and areas stay comparable.
    let x_domain = numeric_domain(x_numeric);
    let y_domain = numeric_domain_padded(&y);
    let x_scale = LinearScale::new(x_domain);
    let y_scale = if area {
        LinearScale::with_zero(y_domain)
    } else {
        LinearScale::new(y_domain)
    };

    let mut series = Vec::with_capacity(groups.len());
    for (index, (label, rows)) in groups.iter().enumerate() {
        let mut data: Vec<(f64, f64)> = rows
            .iter()
            .filter_map(|&row| Some((x_numeric[row]?, y[row]?)))
            .collect();
        data.sort_by(|a, b| a.0.total_cmp(&b.0));

        let points: Vec<Point> = data
            .iter()
            .map(|(x, y)| Point {
                x: x_scale.map(*x, plot.x, plot.width),
                y: y_scale.map_y(*y, plot.y, plot.height),
            })
            .collect();

        let kind = if area {
            SeriesKind::Area(points)
        } else {
            SeriesKind::Line(points)
        };
        series.push(Series {
            label: label.clone(),
            color: PALETTE[index % PALETTE.len()],
            kind,
        });
    }

    let x_axis = Axis {
        ticks: match x_unit {
            Some(unit) => temporal_ticks(
                &x_scale,
                plot,
                unit,
                column_format(spec.columns, ChannelKind::X),
            ),
            None => linear_ticks(
                &x_scale,
                plot,
                false,
                column_format(spec.columns, ChannelKind::X),
            ),
        },
        categorical: false,
    };
    let y_axis = numeric_axis(&y_scale, plot, column_format(spec.columns, ChannelKind::Y));

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis,
        y_axis,
        series,
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels,
        tooltips,
        table: None,
    })
}

fn point_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
    color: Option<Vec<Option<String>>>,
    marks: &Marks,
) -> Result<Scene, SceneError> {
    let x_numeric = x.as_numeric().ok_or(SceneError::NumericXExpected)?;
    let x_unit = x.temporal_unit();

    let groups = series_groups(&color, y.len());

    let x_domain = numeric_domain(x_numeric);
    let y_domain = numeric_domain_padded(&y);
    let x_scale = LinearScale::new(x_domain);
    let y_scale = LinearScale::new(y_domain);

    let mut series = Vec::with_capacity(groups.len());
    let mut labels = Vec::new();
    let mut tooltips = Vec::new();
    let x_format = column_format(spec.columns, ChannelKind::X);
    let x_text = |value: f64| match x_unit {
        Some(unit) => format_temporal(value.round() as i64, unit, x_format),
        None => format_number(value, x_format),
    };
    for (index, (label, rows)) in groups.iter().enumerate() {
        let mut points = Vec::new();
        for &row in rows {
            let Some(x) = x_numeric[row] else {
                continue;
            };
            let Some(y) = y[row] else {
                continue;
            };
            let position = ScatterPoint {
                x: x_scale.map(x, plot.x, plot.width),
                y: y_scale.map_y(y, plot.y, plot.height),
                radius: marks.radius(row),
            };
            if let Some(text) = marks.label(row) {
                labels.push(Label {
                    x: position.x,
                    y: position.y - position.radius - 2.0,
                    text,
                });
            }
            tooltips.push(TooltipMark {
                x: position.x,
                y: position.y,
                radius: position.radius.max(6.0),
                text: marks.tooltip_text(row, Some(x_text(x)), Some(y)),
            });
            points.push(position);
        }
        series.push(Series {
            label: label.clone(),
            color: PALETTE[index % PALETTE.len()],
            kind: SeriesKind::Points(points),
        });
    }

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis: Axis {
            ticks: match x_unit {
                Some(unit) => temporal_ticks(
                    &x_scale,
                    plot,
                    unit,
                    column_format(spec.columns, ChannelKind::X),
                ),
                None => linear_ticks(
                    &x_scale,
                    plot,
                    false,
                    column_format(spec.columns, ChannelKind::X),
                ),
            },
            categorical: false,
        },
        y_axis: numeric_axis(&y_scale, plot, column_format(spec.columns, ChannelKind::Y)),
        series,
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels,
        tooltips,
        table: None,
    })
}

/// A pie or donut chart: one wedge per category with a positive value, sweeping clockwise from the
/// top. A donut leaves a hole of 55% of the radius.
///
/// Categories with a null, zero or negative value are left out, because a pie has no way to show
/// them. Negative values are common in financial data, so this is a documented choice rather than
/// an error: the caller sees the remaining slices and no wedge for the excluded category.
/// A heatmap: one cell per (x, y) pair, coloured by a numeric measure on the colour shelf.
fn heat_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: XData,
    marks: &Marks,
) -> Result<Scene, SceneError> {
    let xs = x
        .as_categories()
        .ok_or(SceneError::CategoricalXExpected)?
        .to_vec();
    let ys = y
        .as_categories()
        .ok_or(SceneError::CategoricalYExpected)?
        .to_vec();
    let color_index = spec
        .columns
        .iter()
        .position(|column| column.kind == ChannelKind::Color)
        .ok_or(SceneError::MissingShelf("color"))?;
    let format = column_format(spec.columns, ChannelKind::Color);
    let values = read_axis(spec.batches, color_index, true, format)?
        .into_numeric()
        .ok_or(SceneError::NumericColorExpected)?;

    let (distinct_x, x_of) = dedup(&xs);
    let (distinct_y, y_of) = dedup(&ys);
    let (low, high) = numeric_domain(&values);
    let band_width = plot.width / distinct_x.len().max(1) as f32;
    let band_height = plot.height / distinct_y.len().max(1) as f32;
    let inset = if band_width.min(band_height) > 6.0 {
        0.5
    } else {
        0.0
    };
    let color_title = column_title(spec.columns, ChannelKind::Color)
        .unwrap_or_else(|| "Value".to_owned());

    let mut cells = Vec::new();
    let mut tooltips = Vec::new();
    for (row, value) in values.iter().enumerate() {
        let (Some(value), Some(x_position), Some(y_position)) =
            (value, x_of.get(row), y_of.get(row))
        else {
            continue;
        };
        let t = if (high - low).abs() < f64::EPSILON {
            0.5
        } else {
            ((value - low) / (high - low)).clamp(0.0, 1.0) as f32
        };
        let cell = HeatCell {
            x: plot.x + *x_position as f32 * band_width + inset,
            y: plot.y + *y_position as f32 * band_height + inset,
            width: (band_width - inset * 2.0).max(0.0),
            height: (band_height - inset * 2.0).max(0.0),
            color: heat_color(t),
        };
        let text = format!(
            "{}: {}\n{}: {}\n{}: {}",
            marks.x_title,
            xs.get(row).cloned().flatten().unwrap_or_default(),
            marks.y_title,
            ys.get(row).cloned().flatten().unwrap_or_default(),
            color_title,
            format_number(*value, format)
        );
        tooltips.push(TooltipMark {
            x: cell.x + cell.width / 2.0,
            y: cell.y + cell.height / 2.0,
            radius: band_width.min(band_height) / 2.0,
            text,
        });
        cells.push(cell);
    }

    let x_axis = Axis {
        ticks: distinct_x
            .iter()
            .enumerate()
            .map(|(index, category)| Tick {
                position: plot.x + (index as f32 + 0.5) * band_width,
                text: category.clone().unwrap_or_default(),
            })
            .collect(),
        categorical: true,
    };
    let y_axis = Axis {
        ticks: distinct_y
            .iter()
            .enumerate()
            .map(|(index, category)| Tick {
                position: plot.y + (index as f32 + 0.5) * band_height,
                text: category.clone().unwrap_or_default(),
            })
            .collect(),
        categorical: true,
    };

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        x_axis,
        y_axis,
        series: vec![Series {
            label: String::new(),
            color: [0.0; 4],
            kind: SeriesKind::Heat(cells),
        }],
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels: Vec::new(),
        tooltips,
        table: None,
    })
}

/// Maps a value in `0..=1` onto a five-stop blue ramp, light for low values and dark for high.
fn heat_color(t: f32) -> [f32; 4] {
    const STOPS: [[u8; 3]; 5] = [
        [247, 251, 255],
        [198, 219, 239],
        [107, 174, 214],
        [33, 113, 181],
        [8, 48, 107],
    ];
    let t = t.clamp(0.0, 1.0);
    let scaled = t * (STOPS.len() - 1) as f32;
    let low = scaled.floor() as usize;
    let high = (low + 1).min(STOPS.len() - 1);
    let fraction = scaled - low as f32;
    let mix = |a: u8, b: u8| a as f32 + (b as f32 - a as f32) * fraction;
    [
        mix(STOPS[low][0], STOPS[high][0]) / 255.0,
        mix(STOPS[low][1], STOPS[high][1]) / 255.0,
        mix(STOPS[low][2], STOPS[high][2]) / 255.0,
        1.0,
    ]
}

fn arc_scene(
    spec: &SceneSpec<'_>,
    plot: Rect,
    x: XData,
    y: Vec<Option<f64>>,
    donut: bool,
) -> Result<Scene, SceneError> {
    let labels = Vec::new();
    let tooltips = Vec::new();
    let categories = x.as_categories().ok_or(SceneError::CategoricalXExpected)?;

    let center = Point {
        x: plot.x + plot.width / 2.0,
        y: plot.y + plot.height / 2.0,
    };
    let radius = plot.width.min(plot.height) / 2.0 * 0.95;
    let inner_radius = if donut { radius * 0.55 } else { 0.0 };

    let total: f64 = y
        .iter()
        .flatten()
        .filter(|value| value.is_finite() && **value > 0.0)
        .sum();

    let mut series = Vec::new();
    let mut angle = 0.0f32;
    if total > 0.0 {
        for (index, category) in categories.iter().enumerate() {
            let Some(value) = y.get(index).copied().flatten() else {
                continue;
            };
            if !value.is_finite() || value <= 0.0 {
                continue;
            }
            let span = (value / total * std::f64::consts::TAU) as f32;
            let start = angle;
            angle += span;
            series.push(Series {
                label: category.clone().unwrap_or_default(),
                color: PALETTE[series.len() % PALETTE.len()],
                kind: SeriesKind::Arc(ArcSlice {
                    center,
                    radius,
                    inner_radius,
                    start,
                    end: angle,
                }),
            });
        }
    }

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot,
        // A pie has no axes: the categories are named in the legend.
        x_axis: Axis {
            ticks: Vec::new(),
            categorical: true,
        },
        y_axis: Axis {
            ticks: Vec::new(),
            categorical: false,
        },
        series,
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels,
        tooltips,
        table: None,
    })
}

/// A KPI tile: one formatted value, from the `text` shelf, else the y value, else the x value.
fn text_scene(spec: &SceneSpec<'_>) -> Result<Scene, SceneError> {
    let labels = Vec::new();
    let tooltips = Vec::new();
    let index = [ChannelKind::Text, ChannelKind::Y, ChannelKind::X]
        .into_iter()
        .find_map(|kind| spec.columns.iter().position(|column| column.kind == kind))
        .ok_or(SceneError::MissingShelf("text"))?;
    let column = &spec.columns[index];
    let value = read_text_value(spec.batches, index, column)?;

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot: Rect {
            x: 12.0,
            y: 12.0,
            width: (spec.size.0 - 24.0).max(1.0),
            height: (spec.size.1 - 24.0).max(1.0),
        },
        x_axis: Axis {
            ticks: Vec::new(),
            categorical: false,
        },
        y_axis: Axis {
            ticks: Vec::new(),
            categorical: false,
        },
        series: vec![Series {
            label: String::new(),
            color: PALETTE[0],
            kind: SeriesKind::Text(value),
        }],
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels,
        tooltips,
        table: None,
    })
}

/// Reads the first non-null value of a column as display text, applying its number format.
fn read_text_value(
    batches: &[RecordBatch],
    index: usize,
    column: &OutputColumn,
) -> Result<String, SceneError> {
    if let Some(values) = batches.iter().find_map(|batch| to_f64(batch.column(index)))
        && let Some(value) = values.into_iter().flatten().next()
    {
        return Ok(format_number(value, column.format.as_ref()));
    }
    for batch in batches {
        if let Some(array) = batch.column(index).as_any().downcast_ref::<StringArray>() {
            for i in 0..array.len() {
                if !array.is_null(i) {
                    return Ok(array.value(i).to_owned());
                }
            }
        }
    }
    Ok(String::new())
}

/// A table: the result's columns and rows as formatted text, truncated to fit the panel.
fn table_scene(spec: &SceneSpec<'_>) -> Result<Scene, SceneError> {
    let columns = spec.columns;
    if columns.is_empty() {
        return Err(SceneError::MissingShelf("column"));
    }

    let margin = 12.0;
    let x = margin;
    let y = margin;
    let width = (spec.size.0 - margin * 2.0).max(1.0);
    let header_height = 22.0;
    let row_height = 18.0;
    let available = (spec.size.1 - margin * 2.0 - header_height).max(0.0);
    let max_rows = (available / row_height).floor() as usize;

    let headers = columns.iter().map(|column| column.title.clone()).collect();
    let numeric: Vec<Option<Vec<Option<f64>>>> = columns
        .iter()
        .enumerate()
        .map(|(index, _)| {
            read_axis(spec.batches, index, true, None)
                .ok()
                .and_then(XData::into_numeric)
        })
        .collect();
    let temporal: Vec<Option<Vec<Option<String>>>> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| read_temporal_labels(spec.batches, index, column.format.as_ref()))
        .collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut flat_row = 0usize;
    'batches: for batch in spec.batches {
        for row in 0..batch.num_rows() {
            if rows.len() >= max_rows {
                break 'batches;
            }
            let mut cells = Vec::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                let text = if let Some(labels) = &temporal[index] {
                    labels.get(flat_row).cloned().flatten().unwrap_or_default()
                } else if let Some(values) = &numeric[index] {
                    match values.get(flat_row).copied().flatten() {
                        Some(value) => format_number(value, column.format.as_ref()),
                        None => String::new(),
                    }
                } else {
                    arrow::util::display::array_value_to_string(batch.column(index), row)
                        .unwrap_or_default()
                };
                cells.push(text);
            }
            rows.push(cells);
            flat_row += 1;
        }
    }

    Ok(Scene {
        width: spec.size.0,
        height: spec.size.1,
        plot: Rect {
            x,
            y,
            width,
            height: (spec.size.1 - margin * 2.0).max(1.0),
        },
        x_axis: Axis {
            ticks: Vec::new(),
            categorical: false,
        },
        y_axis: Axis {
            ticks: Vec::new(),
            categorical: false,
        },
        series: Vec::new(),
        legend: Vec::new(),
        x_title: None,
        y_title: None,
        labels: Vec::new(),
        tooltips: Vec::new(),
        table: Some(Table {
            headers,
            rows,
            x,
            y,
            width,
            column_width: width / columns.len() as f32,
            header_height,
            row_height,
        }),
    })
}

impl XData {
    /// The values as category labels: categorical labels, or a temporal column's formatted dates.
    fn as_categories(&self) -> Option<&[Option<String>]> {
        match self {
            XData::Categorical(values) => Some(values),
            XData::Temporal { labels, .. } => Some(labels),
            XData::Numeric(_) => None,
        }
    }

    /// The values as numbers: numeric values, or a temporal column's ticks. A temporal column is
    /// numeric only for a value axis (a line, area or point); it is never a bar's value axis.
    fn as_numeric(&self) -> Option<&[Option<f64>]> {
        match self {
            XData::Numeric(values) => Some(values),
            XData::Temporal { values, .. } => Some(values),
            XData::Categorical(_) => None,
        }
    }

    /// The time unit when the column is temporal, so ticks can be formatted as dates.
    fn temporal_unit(&self) -> Option<TemporalUnit> {
        match self {
            XData::Temporal { unit, .. } => Some(*unit),
            _ => None,
        }
    }

    /// Consumes the column as numeric values, or returns `None` when it is categorical.
    fn into_numeric(self) -> Option<Vec<Option<f64>>> {
        match self {
            XData::Numeric(values) => Some(values),
            XData::Temporal { values, .. } => Some(values),
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

fn numeric_axis(scale: &LinearScale, plot: Rect, format: Option<&FieldFormat>) -> Axis {
    Axis {
        ticks: linear_ticks(scale, plot, true, format),
        categorical: false,
    }
}

/// Builds ticks for a linear scale. When `vertical`, positions are y coordinates (top-down).
fn linear_ticks(
    scale: &LinearScale,
    plot: Rect,
    vertical: bool,
    format: Option<&FieldFormat>,
) -> Vec<Tick> {
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
                text: format_number(value, format),
            }
        })
        .collect()
}

/// Builds ticks for a temporal scale: numeric positions with date-formatted labels.
fn temporal_ticks(
    scale: &LinearScale,
    plot: Rect,
    unit: TemporalUnit,
    format: Option<&FieldFormat>,
) -> Vec<Tick> {
    scale
        .ticks(6)
        .into_iter()
        .map(|value| Tick {
            position: scale.map(value, plot.x, plot.width),
            text: format_temporal(value.round() as i64, unit, format),
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
fn format_plain(value: f64) -> String {
    let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
    if rounded.fract().abs() < f64::EPSILON {
        format!("{rounded:.0}")
    } else {
        let text = format!("{rounded:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Formats a value using a field's number format, or the compact default when it has none.
fn format_number(value: f64, format: Option<&FieldFormat>) -> String {
    let Some(FieldFormat::Number {
        decimals,
        thousands,
        currency,
    }) = format
    else {
        return format_plain(value);
    };

    let decimals = *decimals as usize;
    let negative = value < 0.0;
    let text = format!("{:.decimals$}", value.abs());
    let (integer, fraction) = match text.split_once('.') {
        Some((integer, fraction)) => (integer.to_owned(), Some(fraction.to_owned())),
        None => (text, None),
    };
    let integer = if *thousands {
        group_thousands(&integer)
    } else {
        integer
    };

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if let Some(currency) = currency {
        out.push_str(currency);
    }
    out.push_str(&integer);
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(&fraction);
    }
    out
}

/// Inserts thousands separators into a run of digits.
fn group_thousands(digits: &str) -> String {
    let length = digits.len();
    let mut out = String::with_capacity(length + length / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (length - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

/// Reads one axis as either categorical (text) or numeric, depending on the column's type.
/// How a temporal Arrow column counts time.
#[derive(Debug, Clone, Copy)]
enum TemporalUnit {
    Day,
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

/// Reads a temporal column as raw integer ticks, or `None` when the column is not temporal.
fn temporal_raw(column: &dyn Array) -> Option<(Vec<Option<i64>>, TemporalUnit)> {
    use arrow::datatypes::TimeUnit;
    let unit = match column.data_type() {
        ArrowDataType::Date32 => TemporalUnit::Day,
        ArrowDataType::Date64 => TemporalUnit::Millisecond,
        ArrowDataType::Timestamp(unit, _) => match unit {
            TimeUnit::Second => TemporalUnit::Second,
            TimeUnit::Millisecond => TemporalUnit::Millisecond,
            TimeUnit::Microsecond => TemporalUnit::Microsecond,
            TimeUnit::Nanosecond => TemporalUnit::Nanosecond,
        },
        _ => return None,
    };
    let cast = arrow::compute::cast(column, &ArrowDataType::Int64).ok()?;
    let array = cast.as_any().downcast_ref::<arrow::array::Int64Array>()?;
    Some((
        (0..array.len())
            .map(|i| (!array.is_null(i)).then(|| array.value(i)))
            .collect(),
        unit,
    ))
}

/// The numeric ticks, formatted labels and unit of a temporal column.
type TemporalAxis = (Vec<Option<f64>>, Vec<Option<String>>, TemporalUnit);

/// Reads a temporal column as numeric ticks plus formatted labels, or `None` when the column is not
/// temporal (or there are no batches to inspect).
fn read_temporal_axis(
    batches: &[RecordBatch],
    index: usize,
    format: Option<&FieldFormat>,
) -> Option<TemporalAxis> {
    let mut values = Vec::new();
    let mut labels = Vec::new();
    let mut unit = None;
    for batch in batches {
        let (raw, batch_unit) = temporal_raw(batch.column(index))?;
        let unit = *unit.get_or_insert(batch_unit);
        for value in raw {
            values.push(value.map(|ticks| ticks as f64));
            labels.push(value.map(|ticks| format_temporal(ticks, unit, format)));
        }
    }
    Some((values, labels, unit?))
}

/// Reads a temporal column as formatted labels, or `None` when the column is not temporal.
fn read_temporal_labels(
    batches: &[RecordBatch],
    index: usize,
    format: Option<&FieldFormat>,
) -> Option<Vec<Option<String>>> {
    read_temporal_axis(batches, index, format).map(|(_, labels, _)| labels)
}

/// Formats one temporal tick value with a `%Y-%m-%d %H:%M:%S`-style pattern.
fn format_temporal(value: i64, unit: TemporalUnit, format: Option<&FieldFormat>) -> String {
    let (days, seconds) = match unit {
        TemporalUnit::Day => (value, 0),
        TemporalUnit::Second => split_ticks(value, 1),
        TemporalUnit::Millisecond => split_ticks(value, 1_000),
        TemporalUnit::Microsecond => split_ticks(value, 1_000_000),
        TemporalUnit::Nanosecond => split_ticks(value, 1_000_000_000),
    };
    let (year, month, day) = civil_from_days(days);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let pattern = match format {
        Some(FieldFormat::Date { pattern }) if !pattern.is_empty() => pattern.as_str(),
        _ if matches!(unit, TemporalUnit::Day) => "%Y-%m-%d",
        _ => "%Y-%m-%d %H:%M",
    };
    apply_date_pattern(pattern, year, month, day, hour, minute, second)
}

fn split_ticks(value: i64, per_second: i64) -> (i64, i64) {
    let ticks_per_day = per_second * 86_400;
    let days = value.div_euclid(ticks_per_day);
    let seconds = value.rem_euclid(ticks_per_day) / per_second;
    (days, seconds)
}

/// Converts days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn apply_date_pattern(
    pattern: &str,
    year: i64,
    month: u32,
    day: u32,
    hour: i64,
    minute: i64,
    second: i64,
) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&year.to_string()),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn read_axis(
    batches: &[RecordBatch],
    index: usize,
    y: bool,
    format: Option<&FieldFormat>,
) -> Result<XData, SceneError> {
    if let Some((values, labels, unit)) = read_temporal_axis(batches, index, format) {
        return Ok(XData::Temporal {
            values,
            labels,
            unit,
        });
    }

    let mixed = || {
        if y {
            SceneError::MixedYTypes
        } else {
            SceneError::MixedXTypes
        }
    };
    let undrawable = || {
        if y {
            SceneError::UndrawableYType
        } else {
            SceneError::UndrawableXType
        }
    };

    let mut categorical = Vec::new();
    let mut numeric = Vec::new();
    let mut is_categorical = None;

    for batch in batches {
        let column = batch.column(index);
        if let Some(array) = column.as_any().downcast_ref::<StringArray>() {
            if is_categorical == Some(false) {
                return Err(mixed());
            }
            is_categorical = Some(true);
            for i in 0..array.len() {
                categorical.push((!array.is_null(i)).then(|| array.value(i).to_owned()));
            }
        } else if let Some(array) = column.as_any().downcast_ref::<Float64Array>() {
            if is_categorical == Some(true) {
                return Err(mixed());
            }
            is_categorical = Some(false);
            for i in 0..array.len() {
                numeric.push((!array.is_null(i)).then(|| array.value(i)));
            }
        } else if let Some(values) = to_f64(column) {
            if is_categorical == Some(true) {
                return Err(mixed());
            }
            is_categorical = Some(false);
            numeric.extend(values);
        } else {
            return Err(undrawable());
        }
    }

    match is_categorical {
        Some(true) => Ok(XData::Categorical(categorical)),
        Some(false) => Ok(XData::Numeric(numeric)),
        None => Ok(XData::Categorical(Vec::new())),
    }
}

/// Reads a column as categorical labels, one per row: text as-is, numbers formatted as text.
fn read_labels(batches: &[RecordBatch], index: usize) -> Result<Vec<Option<String>>, SceneError> {
    if let Some(labels) = read_temporal_labels(batches, index, None) {
        return Ok(labels);
    }
    let mut labels = Vec::new();
    for batch in batches {
        let column = batch.column(index);
        if let Some(array) = column.as_any().downcast_ref::<StringArray>() {
            for i in 0..array.len() {
                labels.push((!array.is_null(i)).then(|| array.value(i).to_owned()));
            }
        } else if let Some(values) = to_f64(column) {
            labels.extend(values.into_iter().map(|value| value.map(format_plain)));
        } else {
            return Err(SceneError::UndrawableColorType);
        }
    }
    Ok(labels)
}

/// The series to draw: one entry per colour value, or a single unnamed series when there is no
/// colour shelf. Each series is a list of row indices, in first-appearance order.
fn series_groups(
    color: &Option<Vec<Option<String>>>,
    row_count: usize,
) -> Vec<(String, Vec<usize>)> {
    match color {
        Some(labels) => group_rows(labels),
        None => vec![(String::new(), (0..row_count).collect())],
    }
}

/// Groups row indices by their label, preserving first-appearance order. A null label becomes the
/// empty string, so nulls group together rather than one group per row.
fn group_rows(labels: &[Option<String>]) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (row, label) in labels.iter().enumerate() {
        let text = label.clone().unwrap_or_default();
        let group = match index.get(&text) {
            Some(&group) => group,
            None => {
                let group = groups.len();
                index.insert(text.clone(), group);
                groups.push((text, Vec::new()));
                group
            }
        };
        groups[group].1.push(row);
    }
    groups
}

/// Maps each value to its first-appearance index and returns the distinct values in that order.
fn dedup(values: &[Option<String>]) -> (Vec<Option<String>>, Vec<usize>) {
    let mut distinct: Vec<Option<String>> = Vec::new();
    let mut index: HashMap<Option<String>, usize> = HashMap::new();
    let mut indices = Vec::with_capacity(values.len());
    for value in values {
        let key = value.clone();
        let position = match index.get(&key) {
            Some(&position) => position,
            None => {
                let position = distinct.len();
                index.insert(key, position);
                distinct.push(value.clone());
                position
            }
        };
        indices.push(position);
    }
    (distinct, indices)
}

/// Converts a numeric Arrow column to `f64`, whatever its integer, decimal or float type.
///
/// DuckDB returns `HUGEINT` (an Arrow `Decimal128`) for `SUM` and `COUNT` of integer columns, so
/// charts must accept more than just `Float64` and `Int64`.
fn to_f64(column: &dyn Array) -> Option<Vec<Option<f64>>> {
    if !matches!(
        column.data_type(),
        ArrowDataType::Int8
            | ArrowDataType::Int16
            | ArrowDataType::Int32
            | ArrowDataType::Int64
            | ArrowDataType::UInt8
            | ArrowDataType::UInt16
            | ArrowDataType::UInt32
            | ArrowDataType::UInt64
            | ArrowDataType::Float16
            | ArrowDataType::Float32
            | ArrowDataType::Float64
            | ArrowDataType::Decimal128(_, _)
            | ArrowDataType::Decimal256(_, _)
    ) {
        return None;
    }
    let cast = arrow::compute::cast(column, &ArrowDataType::Float64).ok()?;
    let array = cast.as_any().downcast_ref::<Float64Array>()?;
    Some(
        (0..array.len())
            .map(|i| (!array.is_null(i)).then(|| array.value(i)))
            .collect(),
    )
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
    /// A bar chart with categorical x values also needs numeric y values.
    #[error("a bar chart with a categorical x axis needs numeric y values")]
    NumericYExpected,
    /// A heatmap needs categorical y values.
    #[error("a heatmap needs categorical y values, but the y shelf is numeric")]
    CategoricalYExpected,
    /// A heatmap needs a numeric colour value.
    #[error("a heatmap needs a numeric colour value")]
    NumericColorExpected,
    /// A line, area or scatter chart needs numeric x values.
    #[error("this mark needs numeric x values, but the x shelf is categorical")]
    NumericXExpected,
    /// The x column mixes types across batches.
    #[error("the x column mixes text and numeric values")]
    MixedXTypes,
    /// The y column mixes types across batches.
    #[error("the y column mixes text and numeric values")]
    MixedYTypes,
    /// The x column's type cannot be drawn.
    #[error("the x column has a type the scene builder cannot draw")]
    UndrawableXType,
    /// The y column's type cannot be drawn.
    #[error("the y column has a type the scene builder cannot draw")]
    UndrawableYType,
    /// The colour column's type cannot be drawn.
    #[error("the colour column has a type the scene builder cannot draw")]
    UndrawableColorType,
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
            title: name.to_owned(),
            data_type: SpecType::Text,
            format: None,
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
            options: SceneOptions::default(),
        }
    }

    use std::sync::Arc;

    #[test]
    fn integer_values_are_drawn() {
        // A raw integer column (no aggregation) must still produce a bar per category.
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Int32, true),
        ]);
        let mut x = StringBuilder::new();
        x.append_value("A");
        x.append_value("B");
        let y = arrow::array::Int32Array::from(vec![10, 20]);
        let batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x.finish()), Arc::new(y)])
            .expect("batch");

        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("integer bars build");
        let SeriesKind::Bars(bars) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        assert_eq!(bars.len(), 2);
        assert!(bars[1].height > bars[0].height);
    }

    #[test]
    fn hugeint_sums_are_drawn() {
        // DuckDB returns HUGEINT (an Arrow Decimal128) for SUM over an integer column.
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Decimal128(38, 0), true),
        ]);
        let mut x = StringBuilder::new();
        x.append_value("A");
        let y = arrow::array::Decimal128Array::from(vec![Some(123i128)])
            .with_precision_and_scale(38, 0)
            .expect("decimal");
        let batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x.finish()), Arc::new(y)])
            .expect("batch");

        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("hugeint bars build");
        let SeriesKind::Bars(bars) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        assert_eq!(bars.len(), 1);
        assert!(bars[0].height > 0.0);
    }

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
    fn a_heatmap_colours_one_cell_per_category_pair() {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Utf8, true),
            Field::new("value", DataType::Float64, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["A", "B", "A", "B"])),
                Arc::new(StringArray::from(vec!["One", "One", "Two", "Two"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0])),
            ],
        )
        .expect("batch");
        let columns = vec![
            column(ChannelKind::X, "x"),
            column(ChannelKind::Y, "y"),
            column(ChannelKind::Color, "value"),
        ];

        let scene = build(&spec(Mark::Rect, &columns, &[batch])).expect("builds");
        let SeriesKind::Heat(cells) = &scene.series[0].kind else {
            panic!("heat cells");
        };
        assert_eq!(cells.len(), 4);
        assert!(scene.x_axis.categorical && scene.y_axis.categorical);
        let x_labels: Vec<&str> = scene.x_axis.ticks.iter().map(|tick| tick.text.as_str()).collect();
        let y_labels: Vec<&str> = scene.y_axis.ticks.iter().map(|tick| tick.text.as_str()).collect();
        assert_eq!(x_labels, vec!["A", "B"]);
        assert_eq!(y_labels, vec!["One", "Two"]);
        assert!(cells[0].x < cells[1].x, "the second column is further right");
        assert!(cells[0].y < cells[2].y, "the second row is lower");
        assert!(
            cells[0].color[2] > cells[3].color[2],
            "the lowest value is the lightest cell"
        );
        assert!(scene.tooltips[0].text.contains('A'));
        assert!(scene.tooltips[0].text.contains("10"));
    }

    #[test]
    fn nice_ticks_cover_the_domain_with_round_values() {
        let ticks = nice_ticks(0.0, 10.0, 6);
        assert_eq!(ticks, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    }

    fn batch_categorical_color(rows: &[(&str, f64, &str)]) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
            Field::new("color", DataType::Utf8, true),
        ]);
        let mut x = StringBuilder::new();
        let mut color = StringBuilder::new();
        let mut ys = Vec::new();
        for (x_value, y, color_value) in rows {
            x.append_value(x_value);
            ys.push(*y);
            color.append_value(color_value);
        }
        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(x.finish()),
                Arc::new(Float64Array::from(ys)),
                Arc::new(color.finish()),
            ],
        )
        .expect("batch")
    }

    fn color_columns() -> Vec<OutputColumn> {
        vec![
            column(ChannelKind::X, "x"),
            column(ChannelKind::Y, "y"),
            column(ChannelKind::Color, "color"),
        ]
    }

    #[test]
    fn grouped_bars_produce_one_series_per_color() {
        let batch = batch_categorical_color(&[
            ("A", 3.0, "red"),
            ("A", 5.0, "blue"),
            ("B", 2.0, "red"),
            ("B", 4.0, "blue"),
        ]);
        let scene = build(&spec(Mark::Bar, &color_columns(), &[batch])).expect("builds");

        assert_eq!(scene.series.len(), 2);
        assert_eq!(scene.series[0].label, "red");
        assert_eq!(scene.series[1].label, "blue");
        assert_ne!(scene.series[0].color, scene.series[1].color);
        assert_eq!(scene.legend.len(), 2, "a grouped chart carries a legend");
        assert_eq!(scene.legend[0].label, "red");
        assert_eq!(scene.legend[1].label, "blue");

        // Two x categories, so each series carries two bars.
        for series in &scene.series {
            let SeriesKind::Bars(bars) = &series.kind else {
                panic!("expected bars");
            };
            assert_eq!(bars.len(), 2);
        }
        assert_eq!(scene.x_axis.ticks.len(), 2);
    }

    #[test]
    fn grouped_bars_in_one_category_are_side_by_side() {
        let batch = batch_categorical_color(&[("A", 3.0, "red"), ("A", 5.0, "blue")]);
        let scene = build(&spec(Mark::Bar, &color_columns(), &[batch])).expect("builds");

        let SeriesKind::Bars(red) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        let SeriesKind::Bars(blue) = &scene.series[1].kind else {
            panic!("expected bars");
        };
        assert_eq!(red.len(), 1);
        assert_eq!(blue.len(), 1);
        // Same category, so the bars sit beside each other rather than overlapping.
        assert!(red[0].x < blue[0].x);
        assert!(red[0].x >= scene.plot.x - 0.01);
        assert!(blue[0].x + blue[0].width <= scene.plot.x + scene.plot.width + 0.01);
    }

    #[test]
    fn a_line_chart_splits_into_one_series_per_color() {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Float64, true),
            Field::new("y", DataType::Float64, true),
            Field::new("color", DataType::Utf8, true),
        ]);
        let x = Float64Array::from(vec![1.0, 1.0, 2.0, 2.0]);
        let y = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0]);
        let mut color = StringBuilder::new();
        color.append_value("red");
        color.append_value("blue");
        color.append_value("red");
        color.append_value("blue");
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(x), Arc::new(y), Arc::new(color.finish())],
        )
        .expect("batch");

        let scene = build(&spec(Mark::Line, &color_columns(), &[batch])).expect("builds");
        assert_eq!(scene.series.len(), 2);
        assert_eq!(scene.series[0].label, "red");
        assert_eq!(scene.series[1].label, "blue");
        for series in &scene.series {
            let SeriesKind::Line(points) = &series.kind else {
                panic!("expected a line");
            };
            assert_eq!(points.len(), 2);
            for window in points.windows(2) {
                assert!(window[0].x <= window[1].x);
            }
        }
    }

    #[test]
    fn a_pie_chart_has_one_slice_per_positive_value() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A", "B", "C"], &[1.0, 2.0, 1.0]);
        let scene = build(&spec(Mark::Arc, &columns, &[batch])).expect("builds");

        assert_eq!(scene.series.len(), 3);
        assert_eq!(scene.legend.len(), 3);
        assert!(scene.x_axis.ticks.is_empty() && scene.y_axis.ticks.is_empty());

        let mut total = 0.0;
        let mut angles = Vec::new();
        for series in &scene.series {
            let SeriesKind::Arc(slice) = &series.kind else {
                panic!("expected an arc");
            };
            assert!(slice.radius > 0.0);
            assert_eq!(slice.inner_radius, 0.0, "a pie has no hole");
            assert!(slice.end >= slice.start);
            angles.push(slice.end - slice.start);
            total += slice.end - slice.start;
        }
        assert!((total - std::f32::consts::TAU).abs() < 1e-4);
        // B carries twice the value of A, so its slice is twice as wide.
        assert!((angles[1] - angles[0] * 2.0).abs() < 1e-4);
    }

    #[test]
    fn pie_drops_non_positive_values() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A", "B", "C"], &[4.0, -1.0, 0.0]);
        let scene = build(&spec(Mark::Arc, &columns, &[batch])).expect("builds");
        assert_eq!(scene.series.len(), 1);
        assert_eq!(scene.series[0].label, "A");
    }

    #[test]
    fn a_pie_slice_stays_inside_the_plot() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A", "B"], &[1.0, 1.0]);
        let scene = build(&spec(Mark::Arc, &columns, &[batch])).expect("builds");

        let SeriesKind::Arc(slice) = &scene.series[0].kind else {
            panic!("expected an arc");
        };
        assert!(slice.center.x - slice.radius >= scene.plot.x - 0.01);
        assert!(slice.center.x + slice.radius <= scene.plot.x + scene.plot.width + 0.01);
        assert!(slice.center.y - slice.radius >= scene.plot.y - 0.01);
        assert!(slice.center.y + slice.radius <= scene.plot.y + scene.plot.height + 0.01);
    }

    #[test]
    fn a_horizontal_bar_scene_puts_categories_on_the_y_axis() {
        // Categories on y and values on x: the "top ten products as a horizontal bar" shape.
        let schema = Schema::new(vec![
            Field::new("x", DataType::Float64, true),
            Field::new("y", DataType::Utf8, true),
        ]);
        let x = Float64Array::from(vec![10.0, 30.0, 20.0]);
        let mut y = StringBuilder::new();
        for category in ["Low", "High", "Mid"] {
            y.append_value(category);
        }
        let batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(x), Arc::new(y.finish())])
            .expect("batch");

        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");

        assert!(!scene.x_axis.categorical, "the value axis is numeric");
        assert!(scene.y_axis.categorical, "the category axis is on y");
        assert_eq!(scene.y_axis.ticks.len(), 3);

        let SeriesKind::Bars(bars) = &scene.series[0].kind else {
            panic!("expected bars");
        };
        assert_eq!(bars.len(), 3);

        // The bar for 30 is three times as wide as the bar for 10.
        let mut widths: Vec<f32> = bars.iter().map(|bar| bar.width).collect();
        widths.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        assert!(
            (widths[2] - widths[0] * 3.0).abs() < 1.0,
            "bar widths should be proportional: {widths:?}"
        );

        for bar in bars {
            assert!(bar.x >= scene.plot.x - 0.01);
            assert!(bar.x + bar.width <= scene.plot.x + scene.plot.width + 0.01);
            assert!(bar.y >= scene.plot.y - 0.01);
            assert!(bar.y + bar.height <= scene.plot.y + scene.plot.height + 0.01);
        }
    }

    #[test]
    fn hiding_axes_and_legend_clears_them_but_keeps_the_marks() {
        let batch = batch_categorical_color(&[("A", 3.0, "red"), ("A", 5.0, "blue")]);
        let batches = [batch];
        let columns = color_columns();
        let mut scene_spec = spec(Mark::Bar, &columns, &batches);
        scene_spec.options = SceneOptions {
            show_axes: false,
            show_legend: false,
        };

        let scene = build(&scene_spec).expect("builds");
        assert!(scene.x_axis.ticks.is_empty());
        assert!(scene.y_axis.ticks.is_empty());
        assert!(scene.legend.is_empty());
        assert_eq!(scene.series.len(), 2, "the marks are still drawn");
    }

    #[test]
    fn hiding_axes_keeps_a_requested_legend() {
        let batch = batch_categorical_color(&[("A", 3.0, "red"), ("A", 5.0, "blue")]);
        let batches = [batch];
        let columns = color_columns();
        let mut scene_spec = spec(Mark::Bar, &columns, &batches);
        scene_spec.options = SceneOptions {
            show_axes: false,
            show_legend: true,
        };

        let scene = build(&scene_spec).expect("builds");
        assert!(scene.x_axis.ticks.is_empty());
        assert_eq!(scene.legend.len(), 2);
    }

    #[test]
    fn a_number_format_is_applied_to_the_value_axis() {
        let mut y_column = column(ChannelKind::Y, "y");
        y_column.format = Some(FieldFormat::Number {
            decimals: 0,
            thousands: true,
            currency: Some("$".to_owned()),
        });
        let columns = vec![column(ChannelKind::X, "x"), y_column];
        let batch = batch_categorical_y(&["A", "B"], &[1_200.0, 3_400.0]);

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        let texts: Vec<&str> = scene.y_axis.ticks.iter().map(|t| t.text.as_str()).collect();
        assert!(
            texts.iter().any(|text| text.starts_with('$')),
            "ticks carry the currency: {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains(',')),
            "ticks group thousands: {texts:?}"
        );
    }

    #[test]
    fn the_palette_matches_the_light_theme() {
        let theme = slickbi_spec::Theme::light();
        assert_eq!(theme.palette.len(), PALETTE.len());
        for (colour, hex) in PALETTE.iter().zip(&theme.palette) {
            let digits = hex.trim_start_matches('#');
            let channel = |range: std::ops::Range<usize>| {
                u8::from_str_radix(&digits[range], 16).expect("hex digit") as f32
            };
            let expected = [channel(0..2), channel(2..4), channel(4..6)];
            for (index, want) in expected.iter().enumerate() {
                assert!(
                    (colour[index] * 255.0 - want).abs() < 1.5,
                    "{hex} channel {index} drifted from the theme palette"
                );
            }
        }
    }

    #[test]
    fn a_donut_has_an_inner_radius() {
        let columns = vec![column(ChannelKind::X, "x"), column(ChannelKind::Y, "y")];
        let batch = batch_categorical_y(&["A", "B"], &[1.0, 1.0]);
        let scene = build(&spec(Mark::Donut, &columns, &[batch])).expect("builds");

        let SeriesKind::Arc(slice) = &scene.series[0].kind else {
            panic!("expected an arc");
        };
        assert!(slice.inner_radius > 0.0, "a donut has a hole");
        assert!(slice.inner_radius < slice.radius);
    }

    #[test]
    fn axes_carry_titles_from_the_field_names() {
        let columns = vec![
            column(ChannelKind::X, "region"),
            column(ChannelKind::Y, "amount"),
        ];
        let batch = batch_categorical_y(&["A"], &[1.0]);
        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        assert_eq!(scene.x_title.as_deref(), Some("region"));
        assert_eq!(scene.y_title.as_deref(), Some("amount"));
    }

    #[test]
    fn a_pie_has_no_axis_titles() {
        let columns = vec![
            column(ChannelKind::X, "region"),
            column(ChannelKind::Y, "amount"),
        ];
        let batch = batch_categorical_y(&["A"], &[1.0]);
        let scene = build(&spec(Mark::Arc, &columns, &[batch])).expect("builds");
        assert!(scene.x_title.is_none());
        assert!(scene.y_title.is_none());
    }

    #[test]
    fn a_kpi_tile_formats_its_value() {
        let schema = Schema::new(vec![Field::new("total", DataType::Float64, true)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(Float64Array::from(vec![1_234.0]))],
        )
        .expect("batch");
        let mut total = column(ChannelKind::Text, "total");
        total.format = Some(FieldFormat::Number {
            decimals: 0,
            thousands: true,
            currency: Some("$".to_owned()),
        });

        let scene = build(&spec(Mark::Text, &[total], &[batch])).expect("builds");
        let SeriesKind::Text(text) = &scene.series[0].kind else {
            panic!("expected a text tile");
        };
        assert!(text.starts_with('$'), "currency: {text}");
        assert!(text.contains(','), "thousands: {text}");
    }

    #[test]
    fn a_text_shelf_labels_the_marks() {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
            Field::new("label", DataType::Utf8, true),
        ]);
        let mut x = StringBuilder::new();
        x.append_value("A");
        let mut label = StringBuilder::new();
        label.append_value("hello");
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(x.finish()),
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(label.finish()),
            ],
        )
        .expect("batch");
        let columns = vec![
            column(ChannelKind::X, "x"),
            column(ChannelKind::Y, "y"),
            column(ChannelKind::Text, "label"),
        ];

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        assert_eq!(scene.labels.len(), 1);
        assert_eq!(scene.labels[0].text, "hello");
    }

    #[test]
    fn a_tooltip_shelf_feeds_the_tooltip() {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Utf8, true),
            Field::new("y", DataType::Float64, true),
            Field::new("note", DataType::Utf8, true),
        ]);
        let mut x = StringBuilder::new();
        x.append_value("A");
        let mut note = StringBuilder::new();
        note.append_value("detail");
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(x.finish()),
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(note.finish()),
            ],
        )
        .expect("batch");
        let columns = vec![
            column(ChannelKind::X, "x"),
            column(ChannelKind::Y, "y"),
            column(ChannelKind::Tooltip(0), "note"),
        ];

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        assert_eq!(scene.tooltips.len(), 1);
        assert!(
            scene.tooltips[0].text.contains("note: detail"),
            "tooltip: {:?}",
            scene.tooltips[0].text
        );
    }

    #[test]
    fn a_size_shelf_varies_point_radii() {
        let schema = Schema::new(vec![
            Field::new("x", DataType::Float64, true),
            Field::new("y", DataType::Float64, true),
            Field::new("size", DataType::Float64, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 10.0])),
            ],
        )
        .expect("batch");
        let columns = vec![
            column(ChannelKind::X, "x"),
            column(ChannelKind::Y, "y"),
            column(ChannelKind::Size, "size"),
        ];

        let scene = build(&spec(Mark::Point, &columns, &[batch])).expect("builds");
        let SeriesKind::Points(points) = &scene.series[0].kind else {
            panic!("expected points");
        };
        assert!(points[0].radius < points[1].radius, "{points:?}");
        assert!(
            points
                .iter()
                .all(|point| (3.0..=12.0).contains(&point.radius))
        );
    }

    #[test]
    fn a_table_lists_the_columns_and_rows() {
        let columns = vec![
            column(ChannelKind::X, "region"),
            column(ChannelKind::Y, "amount"),
        ];
        let batch = batch_categorical_y(&["A", "B"], &[1.0, 2.0]);

        let scene = build(&spec(Mark::Table, &columns, &[batch])).expect("builds");
        let table = scene.table.expect("a table");
        assert_eq!(
            table.headers,
            vec!["region".to_owned(), "amount".to_owned()]
        );
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0], "A");
        assert_eq!(table.rows[0][1], "1");
    }

    #[test]
    fn a_date_axis_is_labelled_and_formatted() {
        let schema = Schema::new(vec![
            Field::new("day", DataType::Date32, true),
            Field::new("y", DataType::Float64, true),
        ]);
        let days = arrow::array::Date32Array::from(vec![0, 31]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(days), Arc::new(Float64Array::from(vec![1.0, 2.0]))],
        )
        .expect("batch");
        let mut day = column(ChannelKind::X, "day");
        day.format = Some(FieldFormat::Date {
            pattern: "%d/%m/%Y".to_owned(),
        });
        let columns = vec![day, column(ChannelKind::Y, "y")];

        let scene = build(&spec(Mark::Bar, &columns, &[batch])).expect("builds");
        assert!(scene.x_axis.categorical, "a date on a bar stays a category");
        let texts: Vec<&str> = scene.x_axis.ticks.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"01/01/1970"), "labels: {texts:?}");
        assert!(texts.contains(&"01/02/1970"), "labels: {texts:?}");
    }

    #[test]
    fn a_date_on_a_line_is_a_continuous_time_axis() {
        let schema = Schema::new(vec![
            Field::new("day", DataType::Date32, true),
            Field::new("balance", DataType::Float64, true),
        ]);
        // A month of daily balances that drain linearly, the motivating bank-account case.
        let days = arrow::array::Date32Array::from((0..31).collect::<Vec<i32>>());
        let balances: Vec<f64> = (0..31).map(|day| 2_000.0 - day as f64 * 50.0).collect();
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(days), Arc::new(Float64Array::from(balances))],
        )
        .expect("batch");
        let columns = vec![
            column(ChannelKind::X, "day"),
            column(ChannelKind::Y, "balance"),
        ];

        let scene = build(&spec(Mark::Line, &columns, &[batch])).expect("a date on a line builds");
        assert!(!scene.x_axis.categorical, "the time axis is continuous");
        let texts: Vec<&str> = scene.x_axis.ticks.iter().map(|t| t.text.as_str()).collect();
        assert!(
            !texts.is_empty()
                && texts
                    .iter()
                    .all(|text| text.len() == 10 && text.as_bytes()[4] == b'-'),
            "date tick labels: {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.starts_with("1970-01")),
            "date tick labels: {texts:?}"
        );

        let SeriesKind::Line(points) = &scene.series[0].kind else {
            panic!("a line series");
        };
        assert_eq!(points.len(), 31, "every day is drawn");
        assert!(points[0].x < points[30].x, "later dates are further right");
        assert!(
            points[0].y < points[30].y,
            "the drained balance is drawn lower at the end of the month"
        );
    }
}
