//! End-to-end: a `.scz` workbook opens, compiles to SQL and returns Arrow batches from DuckDB.
//!
//! This is the spine of 0.0.1 and the `planner ↔ engine`, `workbook ↔ spec` and `cli ↔ core`
//! boundaries in one test.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use slickbi_calc::ParamValue;
use slickbi_cli::{
    Entitlement, EntitlementError, Feature, OutputFormat, compile, inspect, query, render_preview,
    render_print, render_report_preview, render_svg, validate,
};
use slickbi_engine::arrow::array::{Array, Float64Array, StringArray};
use slickbi_engine::{Engine, SourceFormat};
use slickbi_model::Model;
use slickbi_planner::{PlanOptions, plan};
use slickbi_spec::{
    Agg, Canvas, Channel, ChartElement, Column, ColumnId, DataModel, DataModelId, DataType,
    Element, ElementId, ElementKind, Encoding, Field, FieldId, FieldSource, Mark, Overrides, Rect,
    Report, ReportId, ReportTarget, Role, SortKey, Table, TableId, TextAlign, TextElement,
    Workbook, WorkbookId, WorkbookMeta, Worksheet,
};
use slickbi_workbook::Scz;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_file(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("slickbi-e2e-{label}-{}-{n}", std::process::id()))
}

fn column(id: &str, name: &str, data_type: DataType) -> Column {
    Column {
        id: ColumnId::new(id),
        name: name.into(),
        data_type,
        nullable: true,
    }
}

fn field(id: &str, name: &str, column: &str, role: Role, data_type: DataType) -> Field {
    Field {
        id: FieldId::new(id),
        name: name.into(),
        table: TableId::new("sales"),
        source: FieldSource::Column {
            column: ColumnId::new(column),
        },
        role,
        data_type,
        format: None,
        hidden: false,
        description: None,
    }
}

fn channel(field: &str, agg: Agg) -> Channel {
    Channel {
        field: FieldId::new(field),
        agg,
        title: None,
        scale: None,
    }
}

fn sales_workbook() -> Workbook {
    let model = DataModel {
        id: DataModelId::new("m1"),
        name: "Sales".into(),
        tables: vec![Table {
            id: TableId::new("sales"),
            name: "sales".into(),
            columns: vec![
                column("region", "region", DataType::Text),
                column("amount", "amount", DataType::Number),
            ],
        }],
        fields: vec![
            field(
                "region",
                "Region",
                "region",
                Role::Dimension,
                DataType::Text,
            ),
            field(
                "revenue",
                "Revenue",
                "amount",
                Role::Measure,
                DataType::Number,
            ),
        ],
        relationships: vec![],
    };

    let worksheet = Worksheet {
        id: slickbi_spec::WorksheetId::new("bar"),
        name: "Revenue by region".into(),
        model: DataModelId::new("m1"),
        mark: Mark::Bar,
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
        transforms: vec![],
        filters: vec![],
        scales: vec![],
        sort: vec![SortKey {
            field: FieldId::new("region"),
            agg: Agg::None,
            descending: false,
        }],
        limit: None,
        description: None,
    };

    let mut workbook = Workbook::new(WorkbookId::new("w1"), WorkbookMeta::new("Sales"));
    workbook.models = vec![model];
    workbook.worksheets = vec![worksheet];
    workbook.reports = vec![Report {
        id: ReportId::new("board"),
        name: "Board".into(),
        target: ReportTarget::Screen,
        theme: None,
        canvas: Canvas::standard(),
        elements: vec![
            Element {
                id: ElementId::new("e1"),
                frame: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 300.0,
                },
                kind: ElementKind::Chart {
                    chart: ChartElement {
                        worksheet: slickbi_spec::WorksheetId::new("bar"),
                        title: None,
                        overrides: Overrides::default(),
                    },
                },
            },
            Element {
                id: ElementId::new("e2"),
                frame: Rect {
                    x: 0.0,
                    y: 310.0,
                    width: 640.0,
                    height: 80.0,
                },
                kind: ElementKind::Text {
                    text: TextElement {
                        markdown: "# Notes".into(),
                        align: TextAlign::Left,
                        style: None,
                    },
                },
            },
        ],
    }];
    workbook
}

/// Builds the Parquet extract for the `sales` table and a `.scz` holding it, returning the path.
fn fixture() -> PathBuf {
    let parquet = temp_file("sales.parquet");

    let engine = Engine::open().expect("opens");
    engine
        .execute(
            "COPY (SELECT * FROM (VALUES \
             ('EMEA', 10.5::DOUBLE), ('APAC', 4.0::DOUBLE), ('EMEA', 3.25::DOUBLE)) \
             t(region, amount)) TO ? (FORMAT PARQUET)",
            &[ParamValue::Text(parquet.to_string_lossy().into_owned())],
        )
        .expect("writes extract");

    let extract = std::fs::read(&parquet).expect("reads extract");
    let scz = Scz::new(sales_workbook()).with_extract("sales", extract);

    let path = temp_file("workbook.scz");
    scz.save(&path).expect("saves .scz");
    path
}

#[test]
fn a_scz_opens_compiles_and_returns_arrow_batches() {
    let path = fixture();

    let scz = Scz::open(&path).expect("opens .scz");
    let workbook = scz.workbook();
    let model = Model::resolve(&workbook.models[0]).expect("resolves");
    let worksheet = &workbook.worksheets[0];

    let query = plan(&model, worksheet, &PlanOptions::default()).expect("plans");
    assert!(query.aggregate);
    assert!(query.sql.contains("sum(\"sales\".\"amount\")"));

    // Register the extract embedded in the .scz and run the plan against real DuckDB.
    let extract = scz.extract("sales").expect("has extract");
    let extract_path = temp_file("extract.parquet");
    std::fs::write(&extract_path, extract).expect("writes extract");

    let engine = Engine::open().expect("opens engine");
    engine
        .register("sales", &extract_path, SourceFormat::Parquet)
        .expect("registers");

    let result = engine.execute(&query.sql, &query.params).expect("executes");
    assert_eq!(result.rows, 2);

    let batch = &result.batches[0];
    let regions = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("region is text");
    let totals = batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("total is double");

    assert_eq!(regions.value(0), "APAC");
    assert_eq!(regions.value(1), "EMEA");
    assert_eq!(totals.value(0), 4.0);
    assert_eq!(totals.value(1), 13.75);

    std::fs::remove_file(&extract_path).ok();
}

#[test]
fn the_cli_performs_the_same_work() {
    let path = fixture();

    validate(&path).expect("validates");

    let sql = compile(&path, Some("Revenue by region")).expect("compiles");
    assert!(sql.contains("sum(\"sales\".\"amount\")"));
    assert!(sql.contains("GROUP BY 1"));

    let summary = inspect(&path).expect("inspects");
    assert!(summary.contains("Revenue by region"));
    assert!(summary.contains("model 'Sales'"));

    let csv = query(&path, "Revenue by region", OutputFormat::Csv).expect("queries");
    assert!(csv.starts_with("x,y\n"), "unexpected header: {csv}");
    assert!(csv.contains("APAC"), "{csv}");
    assert!(csv.contains("EMEA"), "{csv}");
    assert!(csv.contains("13.75"), "{csv}");
}

#[test]
fn render_preview_writes_a_png() {
    let path = fixture();
    let out = std::env::temp_dir().join(format!(
        "slickbi-render-{}-{}.png",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    render_preview(&path, "Revenue by region", &out, (400.0, 300.0)).expect("renders");
    let bytes = std::fs::read(&out).expect("reads");
    assert!(
        bytes.len() > 1000,
        "a chart PNG should be more than a header"
    );
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    std::fs::remove_file(&out).ok();
}

#[test]
fn render_report_writes_a_png() {
    let path = fixture();
    let out = std::env::temp_dir().join(format!(
        "slickbi-report-{}-{}.png",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    render_report_preview(&path, "Board", &out, 1.0).expect("renders a report");
    let bytes = std::fs::read(&out).expect("reads");
    assert!(
        bytes.len() > 1000,
        "a report PNG should be more than a header"
    );
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    std::fs::remove_file(&out).ok();
}

#[test]
fn print_export_requires_pro() {
    let path = fixture();
    let out = std::env::temp_dir().join(format!(
        "slickbi-print-{}-{}.png",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    // The free tier is refused with a typed error naming the feature, and writes no file.
    let error = render_print(
        &path,
        "Revenue by region",
        &out,
        (400.0, 300.0),
        4.0,
        Entitlement::free(),
    )
    .expect_err("free must be refused");
    match error {
        slickbi_cli::CliError::Entitlement(EntitlementError::RequiresPro { feature }) => {
            assert_eq!(feature, Feature::PrintPng);
        }
        other => panic!("expected a requires-Pro error, got {other:?}"),
    }
    assert!(!out.exists(), "a refused export writes no file");

    // A Pro entitlement writes the 4× image.
    render_print(
        &path,
        "Revenue by region",
        &out,
        (400.0, 300.0),
        4.0,
        Entitlement::pro(),
    )
    .expect("pro renders");
    let bytes = std::fs::read(&out).expect("reads");
    assert!(bytes.len() > 1000);
    std::fs::remove_file(&out).ok();
}

#[test]
fn svg_export_requires_pro() {
    let path = fixture();
    let out = std::env::temp_dir().join(format!(
        "slickbi-svg-{}-{}.svg",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let error = render_svg(
        &path,
        "Revenue by region",
        &out,
        (400.0, 300.0),
        Entitlement::free(),
    )
    .expect_err("free must be refused");
    match error {
        slickbi_cli::CliError::Entitlement(EntitlementError::RequiresPro { feature }) => {
            assert_eq!(feature, Feature::Svg);
        }
        other => panic!("expected a requires-Pro error, got {other:?}"),
    }
    assert!(!out.exists(), "a refused export writes no file");

    render_svg(
        &path,
        "Revenue by region",
        &out,
        (400.0, 300.0),
        Entitlement::pro(),
    )
    .expect("pro renders");
    let svg = std::fs::read_to_string(&out).expect("reads");
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
    assert!(
        svg.contains("APAC"),
        "the category labels are in the document"
    );
    std::fs::remove_file(&out).ok();
}

#[test]
fn the_cli_refuses_a_worksheet_whose_table_has_no_extract() {
    let scz = Scz::new(sales_workbook()); // no extract
    let path = temp_file("empty.scz");
    scz.save(&path).expect("saves");

    let error = query(&path, "Revenue by region", OutputFormat::Table).expect_err("must fail");
    match error {
        slickbi_cli::CliError::TableHasNoExtract(table) => assert_eq!(table, "sales"),
        other => panic!("expected a missing-extract error, got {other}"),
    }
}
