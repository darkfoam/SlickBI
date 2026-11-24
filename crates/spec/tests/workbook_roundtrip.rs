//! Fixture construction and the contract the workbook format must satisfy.

use std::collections::BTreeMap;

use slickbi_spec::{
    Agg, Canvas, Channel, ChartElement, Column, Connection, ConnectionKind, DataModel, DataType,
    Element, ElementId, ElementKind, Encoding, Field, FieldSource, Mark, Overrides, Rect, Report,
    ReportId, ReportTarget, Role, Table, TextAlign, TextElement, Theme, ValidationError, Workbook,
    WorkbookId, WorkbookMeta, Worksheet, WorksheetId,
};

fn column(id: &str, data_type: DataType) -> Column {
    Column {
        id: id.into(),
        name: id.into(),
        data_type,
        nullable: false,
    }
}

fn field(id: &str, name: &str, column: &str, role: Role, data_type: DataType) -> Field {
    Field {
        id: id.into(),
        name: name.into(),
        table: "sales".into(),
        source: FieldSource::Column {
            column: column.into(),
        },
        role,
        data_type,
        format: None,
        hidden: false,
        description: None,
    }
}

fn worksheet(id: &str, name: &str, mark: Mark) -> Worksheet {
    Worksheet {
        id: id.into(),
        name: name.into(),
        model: "model-sales".into(),
        mark,
        encoding: Encoding {
            x: Some(Channel {
                field: "f-region".into(),
                agg: Agg::None,
                title: None,
                scale: None,
            }),
            y: Some(Channel {
                field: "f-amount".into(),
                agg: Agg::Sum,
                title: None,
                scale: None,
            }),
            ..Encoding::default()
        },
        transforms: Vec::new(),
        filters: Vec::new(),
        scales: Vec::new(),
        sort: Vec::new(),
        limit: None,
        description: None,
    }
}

fn chart_element(id: &str, x: f64, worksheet: &str) -> Element {
    Element {
        id: ElementId::new(id),
        frame: Rect {
            x,
            y: 80.0,
            width: 560.0,
            height: 320.0,
        },
        kind: ElementKind::Chart {
            chart: ChartElement {
                worksheet: WorksheetId::new(worksheet),
                title: Some("Revenue".into()),
                overrides: Overrides {
                    show_legend: Some(false),
                    ..Overrides::default()
                },
            },
        },
    }
}

/// Builds a workbook that exercises every part of the document model.
pub fn sample_workbook() -> Workbook {
    let connection = Connection {
        id: "conn-sales".into(),
        name: "Sales file".into(),
        kind: ConnectionKind::Csv,
        location: "sales.csv".into(),
        read_only: false,
        options: BTreeMap::new(),
    };

    let table = Table {
        id: "sales".into(),
        name: "sales".into(),
        columns: vec![
            column("region", DataType::Text),
            column("month", DataType::Date),
            column("amount", DataType::Number),
        ],
    };

    let model = DataModel {
        id: "model-sales".into(),
        name: "Sales".into(),
        tables: vec![table],
        fields: vec![
            field(
                "f-region",
                "Region",
                "region",
                Role::Dimension,
                DataType::Text,
            ),
            field("f-month", "Month", "month", Role::Dimension, DataType::Date),
            field(
                "f-amount",
                "Amount",
                "amount",
                Role::Measure,
                DataType::Number,
            ),
        ],
        relationships: Vec::new(),
    };

    let worksheets = vec![
        worksheet("ws-revenue", "Revenue by region", Mark::Bar),
        worksheet("ws-trend", "Monthly trend", Mark::Line),
    ];

    let report = Report {
        id: ReportId::new("report-annual"),
        name: "Annual review".into(),
        target: ReportTarget::Screen,
        theme: None,
        canvas: Canvas::standard(),
        elements: vec![
            Element {
                id: ElementId::new("el-title"),
                frame: Rect {
                    x: 40.0,
                    y: 20.0,
                    width: 1200.0,
                    height: 48.0,
                },
                kind: ElementKind::Text {
                    text: TextElement {
                        markdown: "# Annual review".into(),
                        align: TextAlign::Left,
                        style: None,
                    },
                },
            },
            chart_element("el-revenue", 40.0, "ws-revenue"),
            chart_element("el-trend", 640.0, "ws-trend"),
        ],
    };

    Workbook {
        format_version: slickbi_spec::FORMAT_VERSION,
        id: WorkbookId::new("book-annual"),
        meta: WorkbookMeta::new("Annual review"),
        connections: vec![connection],
        models: vec![model],
        worksheets,
        reports: vec![report],
        theme: Some(Theme::light()),
    }
}

#[test]
fn sample_workbook_is_valid() {
    sample_workbook().validate().expect("sample must be valid");
}

#[test]
fn workbook_round_trips_through_json_with_identical_semantics() {
    let workbook = sample_workbook();
    let json = workbook.to_json_string().expect("serialise");
    let loaded = Workbook::load_str(&json).expect("load");

    assert_eq!(loaded, workbook);
    assert_eq!(loaded.worksheets.len(), 2);
    assert_eq!(loaded.reports.len(), 1);
    assert_eq!(loaded.reports[0].elements.len(), 3);
}

#[test]
fn report_references_resolve_to_worksheets() {
    let workbook = sample_workbook();
    let referenced = workbook.reports[0].referenced_worksheets();
    let ids: Vec<&str> = referenced.iter().map(|id| id.as_str()).collect();
    assert_eq!(ids, vec!["ws-revenue", "ws-trend"]);
}

#[test]
fn referencing_reports_finds_every_user_of_a_worksheet() {
    let workbook = sample_workbook();
    let reports = workbook.referencing_reports(&WorksheetId::new("ws-trend"));
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].id.as_str(), "report-annual");

    let unused = workbook.referencing_reports(&WorksheetId::new("ws-missing"));
    assert!(unused.is_empty());
}

#[test]
fn a_report_that_loses_its_worksheet_is_invalid() {
    let mut workbook = sample_workbook();
    workbook
        .worksheets
        .retain(|ws| ws.id.as_str() != "ws-trend");

    let error = workbook.validate().expect_err("must fail");
    assert_eq!(
        error,
        ValidationError::UnknownWorksheet {
            report: "report-annual".into(),
            worksheet: "ws-trend".into(),
        }
    );
}

#[test]
fn duplicate_worksheet_ids_are_rejected() {
    let mut workbook = sample_workbook();
    let mut duplicate = workbook.worksheets[0].clone();
    duplicate.name = "Duplicate".into();
    workbook.worksheets.push(duplicate);

    let error = workbook.validate().expect_err("must fail");
    assert_eq!(
        error,
        ValidationError::DuplicateId {
            entity: "worksheet",
            id: "ws-revenue".into(),
        }
    );
}

#[test]
fn a_field_without_its_table_is_rejected() {
    let mut workbook = sample_workbook();
    workbook.models[0].tables.clear();

    let error = workbook.validate().expect_err("must fail");
    match error {
        ValidationError::UnknownTable { field, table, .. } => {
            assert_eq!(field, "f-region");
            assert_eq!(table, "sales");
        }
        other => panic!("expected an unknown table, got {other:?}"),
    }
}

#[test]
fn a_worksheet_using_an_unknown_field_is_rejected() {
    let mut workbook = sample_workbook();
    workbook.worksheets[0].encoding.y = Some(Channel {
        field: "f-nope".into(),
        agg: Agg::Sum,
        title: None,
        scale: None,
    });

    let error = workbook.validate().expect_err("must fail");
    assert_eq!(
        error,
        ValidationError::UnknownField {
            worksheet: "ws-revenue".into(),
            field: "f-nope".into(),
        }
    );
}

#[test]
fn load_rejects_a_document_that_breaks_invariants() {
    let mut workbook = sample_workbook();
    workbook.worksheets.clear();

    let json = workbook.to_json_string().expect("serialise");
    let error = Workbook::load_str(&json).expect_err("must fail");
    assert!(matches!(
        error,
        slickbi_spec::FormatError::Invalid(ValidationError::UnknownWorksheet { .. })
    ));
}

#[test]
fn unknown_fields_are_rejected_so_typos_never_load_silently() {
    let workbook = sample_workbook();
    let json = workbook.to_json_string().expect("serialise");
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    value["worksheets"][0]["encodings"] = serde_json::json!({});

    let error = Workbook::from_json_str(&value.to_string()).expect_err("must fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn the_schema_describes_the_document_it_serialises() {
    let schema = slickbi_spec::workbook_schema();
    assert_eq!(schema["title"], "Workbook");
    assert!(schema["properties"]["worksheets"].is_object());
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );

    let defs = &schema["$defs"];
    for definition in ["Worksheet", "Report", "DataModel", "Connection"] {
        assert!(
            defs.get(definition).is_some_and(|value| value.is_object()),
            "schema is missing a definition for {definition}"
        );
    }
}
