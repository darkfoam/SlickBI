//! Reports: the composed document.
//!
//! A report holds **references** to worksheets and its own presentation. That asymmetry is the
//! product's core workflow promise: change a worksheet's fields and every report using it
//! updates, but a report can never change what a chart measures.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{ElementId, ReportId, WorksheetId};

/// A rectangle in points, with the origin at the top left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    /// Distance from the left edge.
    pub x: f64,
    /// Distance from the top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// The drawing surface of a screen report, in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    /// Width in points.
    pub width: f64,
    /// Height in points.
    pub height: f64,
}

impl Canvas {
    /// A canvas large enough for a two-column report.
    pub const DEFAULT_WIDTH: f64 = 1280.0;
    /// A canvas large enough for a header and two rows of charts.
    pub const DEFAULT_HEIGHT: f64 = 800.0;

    /// Returns the default canvas.
    pub fn standard() -> Self {
        Self {
            width: Self::DEFAULT_WIDTH,
            height: Self::DEFAULT_HEIGHT,
        }
    }
}

/// The paper size of a print report.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageSize {
    /// ISO A4.
    A4,
    /// US Letter.
    Letter,
    /// US Legal.
    Legal,
    /// A custom size in points.
    Custom {
        /// Width in points.
        width: f64,
        /// Height in points.
        height: f64,
    },
}

/// Which way a print page faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Taller than wide.
    Portrait,
    /// Wider than tall.
    Landscape,
}

/// Page margins in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Margins {
    /// Top margin.
    pub top: f64,
    /// Right margin.
    pub right: f64,
    /// Bottom margin.
    pub bottom: f64,
    /// Left margin.
    pub left: f64,
}

impl Margins {
    /// One inch all round.
    pub fn inch() -> Self {
        Self {
            top: 72.0,
            right: 72.0,
            bottom: 72.0,
            left: 72.0,
        }
    }
}

/// Everything needed to lay a report out on paper.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageSetup {
    /// The paper.
    pub size: PageSize,
    /// The orientation.
    #[serde(default = "default_orientation")]
    pub orientation: Orientation,
    /// The margins.
    pub margins: Margins,
}

fn default_orientation() -> Orientation {
    Orientation::Portrait
}

/// What a report is designed for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTarget {
    /// A screen document that scales to the window.
    Screen,
    /// A paginated document intended for paper or PDF.
    Print {
        /// The page layout.
        page: PageSetup,
    },
}

/// Presentation overrides applied to one chart in one report.
///
/// Every field is optional and only ever affects appearance. Anything that changes the numbers
/// belongs to the worksheet, not here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    /// Whether to draw the chart title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_title: Option<bool>,
    /// Whether to draw the legend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<bool>,
    /// Whether to draw the axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_axes: Option<bool>,
    /// Inner padding in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    /// Whether to draw a border around the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<bool>,
    /// A background colour, as a CSS colour string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

/// A worksheet placed in a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartElement {
    /// The worksheet this element displays. The values come from here, live.
    pub worksheet: WorksheetId,
    /// A title for this placement, overriding the worksheet name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Appearance overrides.
    #[serde(default)]
    pub overrides: Overrides,
}

/// How text is aligned inside its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// Flush left.
    #[default]
    Left,
    /// Centred.
    Center,
    /// Flush right.
    Right,
}

/// A block of prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextElement {
    /// Markdown source.
    pub markdown: String,
    /// Alignment.
    #[serde(default)]
    pub align: TextAlign,
    /// A named text style from the report theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// What an element draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementKind {
    /// A chart, referenced from a worksheet.
    Chart {
        /// The chart placement.
        chart: ChartElement,
    },
    /// A block of text.
    Text {
        /// The text.
        text: TextElement,
    },
}

/// One positioned item in a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Element {
    /// Identifier, unique within the report.
    pub id: ElementId,
    /// Where it sits and how large it is.
    pub frame: Rect,
    /// What it draws.
    pub kind: ElementKind,
}

impl Element {
    /// Returns the referenced worksheet, when this element is a chart.
    pub fn worksheet(&self) -> Option<&WorksheetId> {
        match &self.kind {
            ElementKind::Chart { chart } => Some(&chart.worksheet),
            ElementKind::Text { .. } => None,
        }
    }
}

/// A composed document built from worksheets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Identifier.
    pub id: ReportId,
    /// Name shown in the report list and as the document title.
    pub name: String,
    /// Whether this is a screen or print document.
    pub target: ReportTarget,
    /// The theme name, resolved against the workbook theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The drawing surface for screen reports.
    pub canvas: Canvas,
    /// The elements, in draw order.
    pub elements: Vec<Element>,
}

impl Report {
    /// Iterates over the worksheets this report references, once each.
    pub fn referenced_worksheets(&self) -> Vec<&WorksheetId> {
        let mut found: Vec<&WorksheetId> = Vec::new();
        for element in &self.elements {
            if let Some(id) = element.worksheet()
                && !found.contains(&id)
            {
                found.push(id);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart_element(id: &str, worksheet: &str) -> Element {
        Element {
            id: ElementId::new(id),
            frame: Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
            kind: ElementKind::Chart {
                chart: ChartElement {
                    worksheet: WorksheetId::new(worksheet),
                    title: None,
                    overrides: Overrides::default(),
                },
            },
        }
    }

    #[test]
    fn element_kind_is_tagged() {
        let element = chart_element("e1", "sales");
        let json = serde_json::to_string(&element).expect("serialize");
        assert!(json.contains(r#""kind":"chart""#));
        assert!(json.contains(r#""worksheet":"sales""#));
    }

    #[test]
    fn overrides_default_to_nothing() {
        let overrides = Overrides::default();
        assert!(overrides.show_title.is_none());
        assert!(overrides.padding.is_none());
        let json = serde_json::to_string(&overrides).expect("serialize");
        assert_eq!(json, "{}");
    }

    #[test]
    fn referenced_worksheets_are_deduplicated() {
        let report = Report {
            id: ReportId::new("r1"),
            name: "Annual".into(),
            target: ReportTarget::Screen,
            theme: None,
            canvas: Canvas::standard(),
            elements: vec![
                chart_element("e1", "sales"),
                chart_element("e2", "sales"),
                chart_element("e3", "costs"),
            ],
        };

        let ids: Vec<&str> = report
            .referenced_worksheets()
            .into_iter()
            .map(|id| id.as_str())
            .collect();
        assert_eq!(ids, vec!["sales", "costs"]);
    }

    #[test]
    fn text_elements_reference_no_worksheet() {
        let element = Element {
            id: ElementId::new("e1"),
            frame: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
            kind: ElementKind::Text {
                text: TextElement {
                    markdown: "# Title".into(),
                    align: TextAlign::Left,
                    style: None,
                },
            },
        };
        assert!(element.worksheet().is_none());
    }

    #[test]
    fn print_target_carries_page_setup() {
        let target = ReportTarget::Print {
            page: PageSetup {
                size: PageSize::A4,
                orientation: Orientation::Landscape,
                margins: Margins::inch(),
            },
        };
        let json = serde_json::to_string(&target).expect("serialize");
        let back: ReportTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, target);
    }
}
