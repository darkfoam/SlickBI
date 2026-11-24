//! Golden SQL tests for the DuckDB dialect.
//!
//! Each worksheet's planned SQL is compared byte-for-byte against a file under
//! `tests/sql/duckdb/`. Regenerate the files with `SLICKBI_UPDATE_GOLDEN=1 cargo test -p
//! slickbi-planner --test duckdb`, then review the diff before committing. The parameters and
//! column metadata are asserted in Rust, not in the files, because their formatting is incidental.

use std::path::Path;

use slickbi_calc::ParamValue;
use slickbi_model::Model;
use slickbi_planner::{ChannelKind, Dialect, PlanError, PlanOptions, plan};
use slickbi_spec::{
    Agg, Channel, Column, ColumnId, DataModel, DataModelId, DataType, Encoding, Field, FieldId,
    FieldSource, Filter, FilterOp, Mark, Role, Scalar, SortKey, Table, TableId, Transform,
    Worksheet, WorksheetId,
};

fn model() -> Model {
    let document = DataModel {
        id: DataModelId::new("m1"),
        name: "Sales".into(),
        tables: vec![Table {
            id: TableId::new("sales"),
            name: "sales".into(),
            columns: vec![
                column("region", "region", DataType::Text),
                column("month", "month", DataType::Date),
                column("amount", "amount", DataType::Number),
                column("units", "units", DataType::Integer),
                column("active", "active", DataType::Boolean),
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
            field("month", "Month", "month", Role::Dimension, DataType::Date),
            field(
                "revenue",
                "Revenue",
                "amount",
                Role::Measure,
                DataType::Number,
            ),
            field("units", "Units", "units", Role::Measure, DataType::Integer),
            field(
                "active",
                "Active",
                "active",
                Role::Dimension,
                DataType::Boolean,
            ),
            calculated(
                "margin",
                "Margin",
                "[Revenue] * 0.2",
                Role::Measure,
                DataType::Number,
            ),
            calculated(
                "revenue_per_unit",
                "Revenue per unit",
                "SUM([Revenue]) / COUNT([Units])",
                Role::Measure,
                DataType::Number,
            ),
        ],
        relationships: vec![],
    };
    Model::resolve(&document).expect("resolves")
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

fn calculated(id: &str, name: &str, expression: &str, role: Role, data_type: DataType) -> Field {
    Field {
        id: FieldId::new(id),
        name: name.into(),
        table: TableId::new("sales"),
        source: FieldSource::Expression {
            expression: expression.into(),
        },
        role,
        data_type,
        format: None,
        hidden: false,
        description: None,
    }
}

fn worksheet(name: &str) -> Worksheet {
    Worksheet {
        id: WorksheetId::new(name),
        name: name.into(),
        model: DataModelId::new("m1"),
        mark: Mark::Bar,
        encoding: Encoding::default(),
        transforms: vec![],
        filters: vec![],
        scales: vec![],
        sort: vec![],
        limit: None,
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

fn filter(field: &str, op: FilterOp, values: Vec<Scalar>) -> Filter {
    Filter {
        field: FieldId::new(field),
        op,
        values,
    }
}

fn sort(field: &str, agg: Agg, descending: bool) -> SortKey {
    SortKey {
        field: FieldId::new(field),
        agg,
        descending,
    }
}

fn text(value: &str) -> Scalar {
    Scalar::Text(value.into())
}

fn assert_golden(name: &str, sql: &str) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sql/duckdb");
    let path = directory.join(format!("{name}.sql"));

    if std::env::var_os("SLICKBI_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(&directory).expect("create golden directory");
        std::fs::write(&path, format!("{sql}\n")).expect("write golden");
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("missing golden file {path:?}: {error}\nrun with SLICKBI_UPDATE_GOLDEN=1")
    });
    assert_eq!(sql, expected.trim_end(), "golden mismatch for {name}");
}

#[test]
fn golden_sql_matches_the_files() {
    let model = model();
    let options = PlanOptions::default();

    let cases: Vec<(&str, Worksheet)> = vec![
        (
            "bar_region_sum",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                ..worksheet("bar_region_sum")
            },
        ),
        (
            "line_month_sum_ordered",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("month", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                sort: vec![sort("month", Agg::None, false)],
                ..worksheet("line_month_sum_ordered")
            },
        ),
        (
            "count_distinct_regions_per_month",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("month", Agg::None)),
                    y: Some(channel("region", Agg::CountDistinct)),
                    ..Encoding::default()
                },
                ..worksheet("count_distinct_regions_per_month")
            },
        ),
        (
            "ratio_calc_field",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("margin", Agg::Sum)),
                    ..Encoding::default()
                },
                ..worksheet("ratio_calc_field")
            },
        ),
        (
            "filtered_date",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                filters: vec![filter(
                    "month",
                    FilterOp::GreaterOrEqual,
                    vec![text("2025-01-01")],
                )],
                ..worksheet("filtered_date")
            },
        ),
        (
            "filtered_in",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                filters: vec![filter(
                    "region",
                    FilterOp::In,
                    vec![text("EMEA"), text("APAC")],
                )],
                ..worksheet("filtered_in")
            },
        ),
        (
            "top_five",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                sort: vec![sort("revenue", Agg::Sum, true)],
                limit: Some(5),
                ..worksheet("top_five")
            },
        ),
        (
            "table_raw",
            Worksheet {
                mark: Mark::Point,
                encoding: Encoding {
                    detail: vec![
                        channel("region", Agg::None),
                        channel("month", Agg::None),
                        channel("revenue", Agg::None),
                        channel("units", Agg::None),
                    ],
                    ..Encoding::default()
                },
                ..worksheet("table_raw")
            },
        ),
        (
            "scatter_raw",
            Worksheet {
                mark: Mark::Point,
                encoding: Encoding {
                    x: Some(channel("revenue", Agg::None)),
                    y: Some(channel("units", Agg::None)),
                    ..Encoding::default()
                },
                ..worksheet("scatter_raw")
            },
        ),
        (
            "multi_dimension_color",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    color: Some(channel("month", Agg::None)),
                    ..Encoding::default()
                },
                ..worksheet("multi_dimension_color")
            },
        ),
        (
            "having_aggregate_filter",
            Worksheet {
                encoding: Encoding {
                    x: Some(channel("region", Agg::None)),
                    y: Some(channel("revenue", Agg::Sum)),
                    ..Encoding::default()
                },
                filters: vec![filter(
                    "revenue_per_unit",
                    FilterOp::Greater,
                    vec![Scalar::Integer(1)],
                )],
                ..worksheet("having_aggregate_filter")
            },
        ),
        (
            "kpi_tile",
            Worksheet {
                mark: Mark::Text,
                encoding: Encoding {
                    text: Some(channel("revenue_per_unit", Agg::None)),
                    ..Encoding::default()
                },
                ..worksheet("kpi_tile")
            },
        ),
    ];

    for (name, worksheet) in cases {
        let query = plan(&model, &worksheet, &options).unwrap_or_else(|error| {
            panic!("{name} failed to plan: {error}");
        });
        assert_golden(name, &query.sql);
    }
}

#[test]
fn parameters_and_columns_are_reported() {
    let model = model();
    let options = PlanOptions::default();

    let ratio = Worksheet {
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("margin", Agg::Sum)),
            ..Encoding::default()
        },
        ..worksheet("ratio")
    };
    let query = plan(&model, &ratio, &options).expect("plans");
    assert_eq!(query.params, vec![ParamValue::Number(0.2)]);
    assert_eq!(query.columns.len(), 2);
    assert_eq!(query.columns[0].kind, ChannelKind::X);
    assert_eq!(query.columns[0].name, "x");
    assert_eq!(query.columns[0].data_type, DataType::Text);
    assert!(!query.columns[0].aggregate);
    assert_eq!(query.columns[1].kind, ChannelKind::Y);
    assert_eq!(query.columns[1].data_type, DataType::Number);
    assert!(query.columns[1].aggregate);
    assert!(query.aggregate);
    assert_eq!(query.dialect, Dialect::DuckDb);

    let raw = Worksheet {
        mark: Mark::Point,
        encoding: Encoding {
            detail: vec![channel("region", Agg::None), channel("units", Agg::None)],
            ..Encoding::default()
        },
        ..worksheet("raw")
    };
    let query = plan(&model, &raw, &options).expect("plans");
    assert!(!query.aggregate);
    assert_eq!(query.columns.len(), 2);
    assert_eq!(query.columns[0].name, "detail_0");
    assert_eq!(query.columns[1].name, "detail_1");
    assert_eq!(query.columns[1].data_type, DataType::Integer);
}

#[test]
fn a_filter_casts_temporal_values() {
    let model = model();
    let worksheet = Worksheet {
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
        filters: vec![filter("month", FilterOp::Equal, vec![text("2025-06-01")])],
        ..worksheet("cast")
    };
    let query = plan(&model, &worksheet, &PlanOptions::default()).expect("plans");
    assert_eq!(query.params, vec![ParamValue::Text("2025-06-01".into())]);
}

#[test]
fn unsupported_transforms_are_rejected() {
    let model = model();
    let worksheet = Worksheet {
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            ..Encoding::default()
        },
        transforms: vec![Transform::Sort {
            keys: vec![sort("region", Agg::None, false)],
        }],
        ..worksheet("transforms")
    };
    let error = plan(&model, &worksheet, &PlanOptions::default()).expect_err("must fail");
    assert!(matches!(
        error,
        PlanError::UnsupportedTransform { kind: "sort" }
    ));
}

#[test]
fn nested_aggregation_is_rejected() {
    let model = model();
    let worksheet = Worksheet {
        encoding: Encoding {
            y: Some(channel("revenue_per_unit", Agg::Sum)),
            ..Encoding::default()
        },
        ..worksheet("nested")
    };
    let error = plan(&model, &worksheet, &PlanOptions::default()).expect_err("must fail");
    assert!(matches!(error, PlanError::NestedAggregation { .. }));
}

#[test]
fn multiple_tables_are_rejected_until_relationships_land() {
    let mut document = DataModel {
        id: DataModelId::new("m2"),
        name: "Joined".into(),
        tables: vec![
            Table {
                id: TableId::new("sales"),
                name: "sales".into(),
                columns: vec![column("region", "region", DataType::Text)],
            },
            Table {
                id: TableId::new("customers"),
                name: "customers".into(),
                columns: vec![column("name", "name", DataType::Text)],
            },
        ],
        fields: vec![
            field(
                "region",
                "Region",
                "region",
                Role::Dimension,
                DataType::Text,
            ),
            Field {
                id: FieldId::new("customer"),
                name: "Customer".into(),
                table: TableId::new("customers"),
                source: FieldSource::Column {
                    column: ColumnId::new("name"),
                },
                role: Role::Dimension,
                data_type: DataType::Text,
                format: None,
                hidden: false,
                description: None,
            },
        ],
        relationships: vec![],
    };
    document.fields[0].table = TableId::new("sales");
    let model = Model::resolve(&document).expect("resolves");

    let worksheet = Worksheet {
        model: DataModelId::new("m2"),
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("customer", Agg::None)),
            ..Encoding::default()
        },
        ..worksheet("multi")
    };
    let error = plan(&model, &worksheet, &PlanOptions::default()).expect_err("must fail");
    assert!(matches!(error, PlanError::MultipleTables { .. }));
}

#[test]
fn an_empty_in_filter_is_rejected() {
    let model = model();
    let worksheet = Worksheet {
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
        filters: vec![filter("region", FilterOp::In, vec![])],
        ..worksheet("empty_in")
    };
    let error = plan(&model, &worksheet, &PlanOptions::default()).expect_err("must fail");
    assert!(matches!(error, PlanError::EmptyFilterValues { .. }));
}

#[test]
fn an_incompatible_filter_value_is_rejected() {
    let model = model();
    let worksheet = Worksheet {
        encoding: Encoding {
            x: Some(channel("region", Agg::None)),
            y: Some(channel("revenue", Agg::Sum)),
            ..Encoding::default()
        },
        filters: vec![filter("revenue", FilterOp::Equal, vec![text("lots")])],
        ..worksheet("bad_filter")
    };
    let error = plan(&model, &worksheet, &PlanOptions::default()).expect_err("must fail");
    assert!(matches!(error, PlanError::IncompatibleFilterValue { .. }));
}
