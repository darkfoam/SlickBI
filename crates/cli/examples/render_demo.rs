//! Renders the demo charts to PNGs so the output can be inspected by a human.
//!
//! ```sh
//! cargo run -p slickbi-cli --example render_demo -- /tmp
//! ```

use std::path::PathBuf;

use slickbi_calc::ParamValue;
use slickbi_cli::render;
use slickbi_engine::Engine;
use slickbi_spec::{
    Agg, Channel, Column, ColumnId, DataModel, DataModelId, DataType, Encoding, Field, FieldId,
    FieldSource, Mark, Role, Table, TableId, Workbook, WorkbookId, WorkbookMeta, Worksheet,
};
use slickbi_workbook::Scz;

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).expect("create output dir");

    // A synthetic dataset: monthly revenue by region.
    let parquet = out_dir.join("demo-sales.parquet");
    let engine = Engine::open().expect("opens");
    engine
        .execute(
            "COPY (SELECT r.region, d.m, (r.base + (d.m * 7)::DOUBLE % 40.0) AS amount \
             FROM (VALUES ('North', 20.0), ('South', 35.0), ('East', 15.0), ('West', 28.0)) r(region, base), \
                  range(1, 13) d(m)) TO ? (FORMAT PARQUET)",
            &[ParamValue::Text(parquet.to_string_lossy().into_owned())],
        )
        .expect("writes demo data");

    let extract = std::fs::read(&parquet).expect("reads demo data");

    let workbook = demo_workbook();
    let scz = Scz::new(workbook).with_extract("sales", extract);
    let scz_path = out_dir.join("demo.scz");
    scz.save(&scz_path).expect("saves .scz");

    for (worksheet, file) in [
        ("Revenue by region", "bar.png"),
        ("Monthly trend", "line.png"),
        ("Units vs revenue", "scatter.png"),
    ] {
        render(
            &scz_path,
            worksheet,
            &out_dir.join(file),
            (800.0, 500.0),
            2.0,
        )
        .unwrap_or_else(|e| panic!("render {worksheet}: {e}"));
        println!("wrote {}", out_dir.join(file).display());
    }
}

fn demo_workbook() -> Workbook {
    let model = DataModel {
        id: DataModelId::new("m1"),
        name: "Sales".into(),
        tables: vec![Table {
            id: TableId::new("sales"),
            name: "sales".into(),
            columns: vec![
                Column {
                    id: ColumnId::new("region"),
                    name: "region".into(),
                    data_type: DataType::Text,
                    nullable: true,
                },
                Column {
                    id: ColumnId::new("m"),
                    name: "m".into(),
                    data_type: DataType::Integer,
                    nullable: true,
                },
                Column {
                    id: ColumnId::new("amount"),
                    name: "amount".into(),
                    data_type: DataType::Number,
                    nullable: true,
                },
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
            field("month", "Month", "m", Role::Dimension, DataType::Integer),
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

    let bar = worksheet(
        "bar",
        "Revenue by region",
        Mark::Bar,
        Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
    );
    let line = worksheet(
        "line",
        "Monthly trend",
        Mark::Line,
        Encoding {
            x: Some(channel("month", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
    );
    let scatter = worksheet(
        "scatter",
        "Units vs revenue",
        Mark::Point,
        Encoding {
            x: Some(channel("month", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
    );

    let mut workbook = Workbook::new(WorkbookId::new("w1"), WorkbookMeta::new("Demo"));
    workbook.models = vec![model];
    workbook.worksheets = vec![bar, line, scatter];
    workbook
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

fn worksheet(id: &str, name: &str, mark: Mark, encoding: Encoding) -> Worksheet {
    Worksheet {
        id: slickbi_spec::WorksheetId::new(id),
        name: name.into(),
        model: DataModelId::new("m1"),
        mark,
        encoding,
        transforms: vec![],
        filters: vec![],
        scales: vec![],
        sort: vec![],
        limit: None,
        description: None,
    }
}
