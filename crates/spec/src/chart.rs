//! Worksheets: one chart, one tab.
//!
//! A worksheet owns everything that determines *what the numbers are*: the model, the fields,
//! the aggregations, the filters, the transforms, the sort and the row limit. A report may
//! restyle a worksheet but can never change any of those, which is why editing a worksheet is
//! safe and why reports stay current.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{DataModelId, FieldId, ParamId, ScaleId, WorksheetId};

/// The shape drawn for each group of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// A bar or column.
    Bar,
    /// A line.
    Line,
    /// A filled area.
    Area,
    /// A point.
    Point,
    /// A rectangle, including heatmap cells.
    Rect,
    /// Text, including KPI tiles.
    Text,
    /// An arc, used by pie and donut charts.
    Arc,
    /// A donut: an arc with a hole in the middle.
    Donut,
    /// A table of the result's columns and rows.
    Table,
    /// A horizontal or vertical rule.
    Rule,
}

/// How values are aggregated before being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Agg {
    /// No aggregation; the value is used as it arrives.
    #[default]
    None,
    /// Arithmetic sum.
    Sum,
    /// Arithmetic mean.
    Average,
    /// Median.
    Median,
    /// Smallest value.
    Min,
    /// Largest value.
    Max,
    /// Row count.
    Count,
    /// Count of distinct values.
    CountDistinct,
    /// Sample standard deviation.
    StdDev,
    /// Sample variance.
    Variance,
    /// First value in the current ordering.
    First,
    /// Last value in the current ordering.
    Last,
}

/// One field placed on one shelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    /// The field to read.
    pub field: FieldId,
    /// How to aggregate it. Dimensions normally use [`Agg::None`].
    #[serde(default)]
    pub agg: Agg,
    /// A title that overrides the field name for this worksheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The scale to use, when the worksheet defines several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleId>,
}

/// The shelves of a worksheet.
///
/// Any non-aggregated channel becomes part of the grouping. The planner derives the `GROUP BY`
/// from this structure rather than from a hand-written list, so the chart and the query cannot
/// disagree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Encoding {
    /// The horizontal shelf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<Channel>,
    /// The vertical shelf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Channel>,
    /// The end of a horizontal range, for bars and areas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x2: Option<Channel>,
    /// The end of a vertical range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<Channel>,
    /// Colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Channel>,
    /// Size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Channel>,
    /// Opacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Channel>,
    /// Shape, for point marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Channel>,
    /// The text of a label or a KPI tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Channel>,
    /// Fields that split marks without encoding a visual property.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<Channel>,
    /// Fields shown in the tooltip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tooltip: Vec<Channel>,
}

impl Encoding {
    /// Iterates over every channel that carries a field, in a stable order.
    ///
    /// Used by validation, by the planner and by the "used in" index. Keeping one ordering in
    /// one place means none of them can drift.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        [
            self.x.as_ref(),
            self.y.as_ref(),
            self.x2.as_ref(),
            self.y2.as_ref(),
            self.color.as_ref(),
            self.size.as_ref(),
            self.opacity.as_ref(),
            self.shape.as_ref(),
            self.text.as_ref(),
        ]
        .into_iter()
        .flatten()
        .chain(self.detail.iter())
        .chain(self.tooltip.iter())
    }

    /// Reports whether the encoding has neither a horizontal nor a vertical shelf.
    pub fn is_empty_plot(&self) -> bool {
        self.x.is_none() && self.y.is_none()
    }
}

/// How a filter compares a field to its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Equal to the single value.
    Equal,
    /// Not equal to the single value.
    NotEqual,
    /// Less than the single value.
    Less,
    /// Less than or equal to the single value.
    LessOrEqual,
    /// Greater than the single value.
    Greater,
    /// Greater than or equal to the single value.
    GreaterOrEqual,
    /// One of the listed values.
    In,
    /// None of the listed values.
    NotIn,
    /// Between the first and second values, inclusive.
    Between,
    /// Contains the text.
    Contains,
    /// Starts with the text.
    StartsWith,
    /// Ends with the text.
    EndsWith,
    /// Is null. Takes no values.
    IsNull,
    /// Is not null. Takes no values.
    IsNotNull,
}

/// A literal value used by a filter or a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Scalar {
    /// A boolean.
    Boolean(bool),
    /// A whole number.
    Integer(i64),
    /// A number with a fraction.
    Number(f64),
    /// Text.
    Text(String),
}

impl Scalar {
    /// The scalar as text, when it is text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Scalar::Text(value) => Some(value),
            _ => None,
        }
    }
}

/// A predicate applied to a worksheet before aggregation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// The field to test.
    pub field: FieldId,
    /// How to test it.
    pub op: FilterOp,
    /// The comparison values. Interpretation depends on [`FilterOp`]: none for null tests, one
    /// for comparison, two for `between`, many for `in` and `not_in`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Scalar>,
    /// A parameter whose current value is used instead of `values`, for a one-value operator.
    ///
    /// Binding a filter to a parameter is how an author offers a control that re-renders the
    /// chart without editing the filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<ParamId>,
}

/// How a numeric or temporal axis is mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScaleKind {
    /// Continuous linear.
    Linear,
    /// Continuous logarithmic.
    Logarithmic,
    /// Continuous time.
    Time,
    /// Discrete with a band per category.
    Band,
    /// Discrete with a point per category.
    Point,
    /// Discrete with no implied order.
    Ordinal,
}

/// Configuration for one scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaleSpec {
    /// Identifier referenced by channels.
    pub id: ScaleId,
    /// The mapping.
    pub kind: ScaleKind,
    /// A palette name, for colour scales.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    /// Whether to extend the domain to round numbers.
    #[serde(default = "default_true")]
    pub nice: bool,
}

fn default_true() -> bool {
    true
}

/// One key in a sort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SortKey {
    /// The field to sort by.
    pub field: FieldId,
    /// The aggregation to apply first, when sorting an aggregated result.
    #[serde(default)]
    pub agg: Agg,
    /// Whether the order is descending.
    #[serde(default)]
    pub descending: bool,
}

/// One aggregate in an explicit aggregation step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AggSpec {
    /// The field to aggregate.
    pub field: FieldId,
    /// How to aggregate it.
    pub agg: Agg,
    /// An optional new name for the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<FieldId>,
}

/// Which side of a window a frame covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowFrameKind {
    /// Count rows.
    Rows,
    /// Compare values.
    Range,
}

/// The frame a window function operates over, expressed as offsets from the current row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowFrame {
    /// Whether offsets count rows or values.
    pub kind: WindowFrameKind,
    /// Offset of the first row, negative for preceding.
    pub start: i64,
    /// Offset of the last row, zero for the current row.
    pub end: i64,
}

/// The function applied across a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowFunction {
    /// Position of the row in the partition.
    RowNumber,
    /// Rank with gaps.
    Rank,
    /// Rank without gaps.
    DenseRank,
    /// Running sum.
    Sum,
    /// Running mean.
    Average,
    /// Running minimum.
    Min,
    /// Running maximum.
    Max,
    /// Running count.
    Count,
    /// The value `n` rows back.
    Lag,
    /// The value `n` rows forward.
    Lead,
    /// The first value in the frame.
    FirstValue,
    /// The last value in the frame.
    LastValue,
}

/// A window or table calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowSpec {
    /// The function.
    pub function: WindowFunction,
    /// Its arguments, when it takes more than the current row's value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<FieldId>,
    /// The fields that define each group the window restarts over.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partition: Vec<FieldId>,
    /// The ordering inside each partition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<SortKey>,
    /// The frame, when the function is frame-sensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<WindowFrame>,
}

/// A step applied to the data after it is read and before it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transform {
    /// Group a numeric field into fixed-width bins.
    Bin {
        /// The field to bin.
        field: FieldId,
        /// The width of each bin.
        size: f64,
        /// The field id the binned values are exposed as.
        alias: FieldId,
    },
    /// An explicit aggregation step.
    Aggregate {
        /// The fields to group by.
        group_by: Vec<FieldId>,
        /// The aggregates to compute.
        aggregates: Vec<AggSpec>,
    },
    /// A calculated expression evaluated over the rows.
    Calculate {
        /// The field id the result is exposed as.
        alias: FieldId,
        /// The expression, owned by the calculation language.
        expression: String,
    },
    /// A window or table calculation.
    Window {
        /// The field id the result is exposed as.
        alias: FieldId,
        /// The window definition.
        spec: WindowSpec,
    },
    /// An ordering.
    Sort {
        /// The keys, applied in order.
        keys: Vec<SortKey>,
    },
    /// A row limit, applied after sorting.
    Limit {
        /// How many rows to keep.
        rows: u32,
    },
}

/// One chart, on one tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Worksheet {
    /// Identifier, referenced by report elements.
    pub id: WorksheetId,
    /// The tab label and the title used in reports unless overridden.
    pub name: String,
    /// The model this worksheet reads.
    pub model: DataModelId,
    /// The mark drawn.
    pub mark: Mark,
    /// The shelves.
    pub encoding: Encoding,
    /// Steps applied between reading and drawing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transforms: Vec<Transform>,
    /// Predicates applied before aggregation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    /// Scale configurations referenced by channels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scales: Vec<ScaleSpec>,
    /// The ordering of the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<SortKey>,
    /// A row limit applied after sorting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// A note for the author, not shown in reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(field: &str) -> Channel {
        Channel {
            field: FieldId::new(field),
            agg: Agg::Sum,
            title: None,
            scale: None,
        }
    }

    #[test]
    fn channels_lists_every_shelf_in_order() {
        let encoding = Encoding {
            x: Some(channel("region")),
            y: Some(channel("revenue")),
            color: Some(channel("segment")),
            detail: vec![channel("order")],
            tooltip: vec![channel("note")],
            ..Encoding::default()
        };

        let fields: Vec<&str> = encoding.channels().map(|c| c.field.as_str()).collect();
        assert_eq!(
            fields,
            vec!["region", "revenue", "segment", "order", "note"]
        );
    }

    #[test]
    fn empty_encoding_has_no_channels() {
        assert_eq!(Encoding::default().channels().count(), 0);
        assert!(Encoding::default().is_empty_plot());
    }

    #[test]
    fn encoding_round_trips() {
        let encoding = Encoding {
            x: Some(channel("region")),
            y: Some(channel("revenue")),
            ..Encoding::default()
        };
        let json = serde_json::to_string(&encoding).expect("serialize");
        let back: Encoding = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, encoding);
    }

    #[test]
    fn transform_is_tagged_by_kind() {
        let transform = Transform::Limit { rows: 10 };
        let json = serde_json::to_string(&transform).expect("serialize");
        assert_eq!(json, r#"{"kind":"limit","rows":10}"#);
    }

    #[test]
    fn scalar_is_untagged() {
        assert_eq!(
            serde_json::to_string(&Scalar::Integer(3)).expect("serialize"),
            "3"
        );
        assert_eq!(
            serde_json::to_string(&Scalar::Text("north".into())).expect("serialize"),
            "\"north\""
        );
        let parsed: Scalar = serde_json::from_str("true").expect("deserialize");
        assert_eq!(parsed, Scalar::Boolean(true));
    }

    #[test]
    fn filter_values_default_to_empty() {
        let json = r#"{"field": "region", "op": "is_null"}"#;
        let filter: Filter = serde_json::from_str(json).expect("deserialize");
        assert!(filter.values.is_empty());
    }
}
