#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The query planner: turns a worksheet into one SQL statement.
//!
//! A worksheet names fields and shelves; a [`Model`] has already resolved those fields to SQL.
//! The planner's job is to assemble that SQL in the right order — `SELECT`, `FROM`, `WHERE`,
//! `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT` — while tracking every bound parameter so that the
//! engine can execute the statement against DuckDB with its values supplied as parameters, never
//! as text.
//!
//! The planner owns one rule the rest of the application relies on: **any non-aggregated shelf
//! becomes part of the grouping**. The `GROUP BY` is derived from the shelves, not stored, so a
//! chart and its query cannot disagree.
//!
//! Only DuckDB is emitted today, but the dialect is a first-class value so a second dialect is an
//! extension, not a rewrite.

use std::collections::BTreeSet;

use slickbi_calc::{ParamValue, ValueType};
use slickbi_model::{Model, ResolvedField, ResolvedTable};
use slickbi_spec::{Agg, Channel, Encoding, Filter, FilterOp, Scalar, SortKey, Transform};
use slickbi_spec::{DataType, FieldFormat, FieldId, ParamId, TableId, Worksheet};
use thiserror::Error;

/// The SQL dialect a query is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// DuckDB's dialect.
    DuckDb,
}

/// Which shelf a column came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// The horizontal shelf.
    X,
    /// The vertical shelf.
    Y,
    /// The end of a horizontal range.
    X2,
    /// The end of a vertical range.
    Y2,
    /// Colour.
    Color,
    /// Size.
    Size,
    /// Opacity.
    Opacity,
    /// Shape.
    Shape,
    /// Text.
    Text,
    /// A detail field, indexed within the detail shelf.
    Detail(usize),
    /// A tooltip field, indexed within the tooltip shelf.
    Tooltip(usize),
}

/// One column of the query result, so the scene knows what each value is.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputColumn {
    /// The shelf the value came from.
    pub kind: ChannelKind,
    /// The SQL alias, which is also the column's key in the result.
    pub name: String,
    /// The field the column reads.
    pub field: FieldId,
    /// The field's display name, for an axis title.
    pub title: String,
    /// The type of the value.
    pub data_type: DataType,
    /// How the value should be displayed, when the field declares a format.
    pub format: Option<FieldFormat>,
    /// Whether the value is aggregated over the group.
    pub aggregate: bool,
}

/// A complete, executable query.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// The SQL statement, with `?` placeholders.
    pub sql: String,
    /// The values for the placeholders, in order.
    pub params: Vec<ParamValue>,
    /// The columns, in `SELECT` order.
    pub columns: Vec<OutputColumn>,
    /// Whether the query aggregates rows into groups.
    pub aggregate: bool,
    /// The dialect the SQL is written in.
    pub dialect: Dialect,
}

/// Options that do not change what the worksheet means, only how much is read and which parameter
/// values are bound.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanOptions {
    /// The most rows an unaggregated query returns when the worksheet sets no limit of its own.
    ///
    /// A raw dump of a large table would otherwise hand every row to the renderer. Aggregated
    /// queries are already bounded by their number of groups and are not capped here.
    pub preview_row_cap: u32,
    /// The workbook's parameters, so a filter bound to one uses its current value.
    pub parameters: Vec<slickbi_spec::Parameter>,
}

impl Default for PlanOptions {
    fn default() -> Self {
        PlanOptions {
            preview_row_cap: 10_000,
            parameters: Vec::new(),
        }
    }
}

/// Plans a worksheet against a resolved model.
///
/// # Errors
///
/// Returns [`PlanError`] when the worksheet names a field the model lacks, mixes fields from
/// several tables (relationships arrive in 0.2.0), nests an aggregation, applies a numeric
/// aggregation to a non-numeric field, uses a transform this release does not support, or gives
/// a filter values it cannot use.
pub fn plan(
    model: &Model,
    worksheet: &Worksheet,
    options: &PlanOptions,
) -> Result<Query, PlanError> {
    let shelves = shelved_channels(&worksheet.encoding);
    if shelves.is_empty() {
        return Err(PlanError::EmptyEncoding);
    }

    if let Some(transform) = worksheet.transforms.first() {
        return Err(PlanError::UnsupportedTransform {
            kind: transform_kind(transform),
        });
    }

    let mut select = Select::default();

    for (kind, channel) in &shelves {
        let field = resolve(model, &channel.field)?;
        let alias = alias_for(*kind);

        let (expression, data_type, aggregate) = select_expression(channel, field)?;
        for value in field.params() {
            select.params.push(value.clone());
        }

        select.items.push(SelectItem {
            kind: *kind,
            alias: alias.clone(),
            expression,
            field: field.id.clone(),
            title: field.name().to_owned(),
            agg: channel.agg,
            data_type,
            format: field.format.clone(),
            aggregate,
        });
    }

    select.aggregate = select.items.iter().any(|item| item.aggregate);

    // Every referenced table, to pick the single FROM.
    let mut tables: BTreeSet<TableId> = BTreeSet::new();
    for item in &select.items {
        collect_tables(model, &item.field, &mut tables)?;
    }
    for filter in &worksheet.filters {
        collect_tables(model, &filter.field, &mut tables)?;
    }
    for key in &worksheet.sort {
        collect_tables(model, &key.field, &mut tables)?;
    }
    let table = single_table(model, &tables)?;

    // Partition filters so parameters stay in SQL-text order: WHERE before HAVING.
    let mut where_filters = Vec::new();
    let mut having_filters = Vec::new();
    for filter in &worksheet.filters {
        let field = resolve(model, &filter.field)?;
        if field.aggregate {
            having_filters.push((filter, field));
        } else {
            where_filters.push((filter, field));
        }
    }

    let mut where_predicates = Vec::new();
    for (filter, field) in &where_filters {
        where_predicates.push(filter_predicate(
            filter,
            field,
            &options.parameters,
            &mut select.params,
        )?);
    }
    let mut having_predicates = Vec::new();
    for (filter, field) in &having_filters {
        having_predicates.push(filter_predicate(
            filter,
            field,
            &options.parameters,
            &mut select.params,
        )?);
    }

    let order = order_by(model, worksheet, &select.items, &mut select.params)?;

    let mut sql = String::new();
    sql.push_str("SELECT\n");
    sql.push_str(
        &select
            .items
            .iter()
            .map(|item| format!("  {} AS {}", item.expression, quote_ident(&item.alias)))
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    sql.push_str(&format!("\nFROM {}", table.sql_name));

    if !where_predicates.is_empty() {
        sql.push_str("\nWHERE ");
        sql.push_str(&where_predicates.join("\n  AND "));
    }

    if select.aggregate {
        let positions: Vec<String> = select
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.aggregate)
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        if !positions.is_empty() {
            sql.push_str("\nGROUP BY ");
            sql.push_str(&positions.join(", "));
        }
    }

    if !having_predicates.is_empty() {
        sql.push_str("\nHAVING ");
        sql.push_str(&having_predicates.join("\n  AND "));
    }

    if !order.is_empty() {
        sql.push_str("\nORDER BY ");
        sql.push_str(
            &order
                .iter()
                .map(|term| match &term.target {
                    OrderTarget::Alias(alias) => {
                        format!("{} {}", quote_ident(alias), direction(term.descending))
                    }
                    OrderTarget::Expression(expression) => {
                        format!("{} {}", expression, direction(term.descending))
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    match worksheet.limit {
        Some(rows) => sql.push_str(&format!("\nLIMIT {rows}")),
        None if !select.aggregate => sql.push_str(&format!("\nLIMIT {}", options.preview_row_cap)),
        None => {}
    }

    Ok(Query {
        sql,
        params: select.params,
        columns: select
            .items
            .into_iter()
            .map(|item| OutputColumn {
                kind: item.kind,
                name: item.alias,
                field: item.field,
                title: item.title,
                data_type: item.data_type,
                format: item.format,
                aggregate: item.aggregate,
            })
            .collect(),
        aggregate: select.aggregate,
        dialect: Dialect::DuckDb,
    })
}

/// One shelf the planner reads, in a stable order that matches the SELECT list.
fn shelved_channels(encoding: &Encoding) -> Vec<(ChannelKind, &Channel)> {
    let mut shelves = Vec::new();

    if let Some(channel) = &encoding.x {
        shelves.push((ChannelKind::X, channel));
    }
    if let Some(channel) = &encoding.y {
        shelves.push((ChannelKind::Y, channel));
    }
    if let Some(channel) = &encoding.x2 {
        shelves.push((ChannelKind::X2, channel));
    }
    if let Some(channel) = &encoding.y2 {
        shelves.push((ChannelKind::Y2, channel));
    }
    if let Some(channel) = &encoding.color {
        shelves.push((ChannelKind::Color, channel));
    }
    if let Some(channel) = &encoding.size {
        shelves.push((ChannelKind::Size, channel));
    }
    if let Some(channel) = &encoding.opacity {
        shelves.push((ChannelKind::Opacity, channel));
    }
    if let Some(channel) = &encoding.shape {
        shelves.push((ChannelKind::Shape, channel));
    }
    if let Some(channel) = &encoding.text {
        shelves.push((ChannelKind::Text, channel));
    }

    for (index, channel) in encoding.detail.iter().enumerate() {
        shelves.push((ChannelKind::Detail(index), channel));
    }
    for (index, channel) in encoding.tooltip.iter().enumerate() {
        shelves.push((ChannelKind::Tooltip(index), channel));
    }

    shelves
}

fn alias_for(kind: ChannelKind) -> String {
    match kind {
        ChannelKind::X => "x".to_owned(),
        ChannelKind::Y => "y".to_owned(),
        ChannelKind::X2 => "x2".to_owned(),
        ChannelKind::Y2 => "y2".to_owned(),
        ChannelKind::Color => "color".to_owned(),
        ChannelKind::Size => "size".to_owned(),
        ChannelKind::Opacity => "opacity".to_owned(),
        ChannelKind::Shape => "shape".to_owned(),
        ChannelKind::Text => "text".to_owned(),
        ChannelKind::Detail(index) => format!("detail_{index}"),
        ChannelKind::Tooltip(index) => format!("tooltip_{index}"),
    }
}

/// The SQL and type of one shelf.
fn select_expression(
    channel: &Channel,
    field: &ResolvedField,
) -> Result<(String, DataType, bool), PlanError> {
    let aggregate = channel.agg != Agg::None || field.aggregate;

    if channel.agg != Agg::None && field.aggregate {
        return Err(PlanError::NestedAggregation {
            field: field.id.clone(),
        });
    }

    if channel.agg == Agg::None {
        // Either a dimension or a raw measure; the SQL is just the field.
        return Ok((field.sql().to_owned(), field.data_type, aggregate));
    }

    let (function, data_type) = aggregate_sql(channel.agg, field)?;
    Ok((function, data_type, true))
}

/// Maps an aggregation to its DuckDB function and result type.
fn aggregate_sql(agg: Agg, field: &ResolvedField) -> Result<(String, DataType), PlanError> {
    let sql = field.sql();

    match agg {
        Agg::Sum => {
            require_numeric(field, agg)?;
            let data_type = if field.value_type() == ValueType::Integer {
                DataType::Integer
            } else {
                DataType::Number
            };
            Ok((format!("sum({sql})"), data_type))
        }
        Agg::Average => {
            require_numeric(field, agg)?;
            Ok((format!("avg({sql})"), DataType::Number))
        }
        Agg::Median => {
            require_numeric(field, agg)?;
            Ok((format!("median({sql})"), DataType::Number))
        }
        Agg::Min => Ok((format!("min({sql})"), field.data_type)),
        Agg::Max => Ok((format!("max({sql})"), field.data_type)),
        Agg::Count => Ok((format!("count({sql})"), DataType::Integer)),
        Agg::CountDistinct => Ok((format!("count(DISTINCT {sql})"), DataType::Integer)),
        Agg::StdDev => {
            require_numeric(field, agg)?;
            Ok((format!("stddev_samp({sql})"), DataType::Number))
        }
        Agg::Variance => {
            require_numeric(field, agg)?;
            Ok((format!("var_samp({sql})"), DataType::Number))
        }
        Agg::First | Agg::Last => Err(PlanError::UnsupportedAggregation { agg }),
        Agg::None => unreachable!("handled by the caller"),
    }
}

fn require_numeric(field: &ResolvedField, agg: Agg) -> Result<(), PlanError> {
    if field.value_type().is_numeric() {
        Ok(())
    } else {
        Err(PlanError::NonNumericAggregation {
            field: field.id.clone(),
            agg,
            found: field.value_type(),
        })
    }
}

fn collect_tables(
    model: &Model,
    field: &FieldId,
    tables: &mut BTreeSet<TableId>,
) -> Result<(), PlanError> {
    let field = resolve(model, field)?;
    tables.extend(field.tables.iter().cloned());
    Ok(())
}

fn single_table<'a>(
    model: &'a Model,
    tables: &BTreeSet<TableId>,
) -> Result<&'a ResolvedTable, PlanError> {
    match tables.len() {
        0 => Err(PlanError::EmptyEncoding),
        1 => {
            let id = tables.iter().next().expect("one table");
            model.table(id).ok_or(PlanError::EmptyEncoding)
        }
        _ => Err(PlanError::MultipleTables {
            tables: tables
                .iter()
                .map(|id| {
                    model
                        .table(id)
                        .map(|table| table.name.clone())
                        .unwrap_or_else(|| id.as_str().to_owned())
                })
                .collect(),
        }),
    }
}

fn resolve<'a>(model: &'a Model, field: &FieldId) -> Result<&'a ResolvedField, PlanError> {
    model.field(field).ok_or_else(|| PlanError::UnknownField {
        field: field.clone(),
    })
}

fn transform_kind(transform: &Transform) -> &'static str {
    match transform {
        Transform::Bin { .. } => "bin",
        Transform::Aggregate { .. } => "aggregate",
        Transform::Calculate { .. } => "calculate",
        Transform::Window { .. } => "window",
        Transform::Sort { .. } => "sort",
        Transform::Limit { .. } => "limit",
    }
}

fn direction(descending: bool) -> &'static str {
    if descending { "DESC" } else { "ASC" }
}

fn quote_ident(name: &str) -> String {
    slickbi_model::quote_ident(name)
}

#[derive(Default)]
struct Select {
    items: Vec<SelectItem>,
    params: Vec<ParamValue>,
    aggregate: bool,
}

struct SelectItem {
    kind: ChannelKind,
    alias: String,
    expression: String,
    field: FieldId,
    title: String,
    agg: Agg,
    data_type: DataType,
    format: Option<FieldFormat>,
    aggregate: bool,
}

/// Builds one filter's SQL predicate, appending its parameters in placeholder order.
fn filter_predicate(
    filter: &Filter,
    field: &ResolvedField,
    parameters: &[slickbi_spec::Parameter],
    params: &mut Vec<ParamValue>,
) -> Result<String, PlanError> {
    let parameter = match &filter.parameter {
        Some(id) => Some(
            parameters
                .iter()
                .find(|parameter| &parameter.id == id)
                .ok_or_else(|| PlanError::UnknownParameter {
                    parameter: id.clone(),
                })?,
        ),
        None => None,
    };
    let sql = field.sql();
    let value_type = field.value_type();

    let expect = |expected: usize| -> Result<(), PlanError> {
        if filter.values.len() == expected {
            Ok(())
        } else {
            Err(PlanError::WrongFilterValueCount {
                field: filter.field.clone(),
                op: filter.op,
                expected: expected_count(expected),
                found: filter.values.len(),
            })
        }
    };

    // A filter bound to a parameter carries no literal values; its one value is the parameter's.
    let single_value = |expect: &dyn Fn(usize) -> Result<(), PlanError>| -> Result<&slickbi_spec::Scalar, PlanError> {
        match parameter {
            Some(parameter) => {
                if !filter.values.is_empty() {
                    return Err(PlanError::ParameterWithValues {
                        field: filter.field.clone(),
                        op: filter.op,
                    });
                }
                Ok(&parameter.value)
            }
            None => {
                expect(1)?;
                Ok(&filter.values[0])
            }
        }
    };

    let multi_value_refusal =
        |parameter: &slickbi_spec::Parameter| PlanError::UnsupportedParameterFilter {
            parameter: parameter.id.clone(),
            op: filter.op,
        };

    let predicate = match filter.op {
        FilterOp::Equal
        | FilterOp::NotEqual
        | FilterOp::Less
        | FilterOp::LessOrEqual
        | FilterOp::Greater
        | FilterOp::GreaterOrEqual => {
            let value = single_value(&expect)?;
            let (value, bound) = bound_value(field, value, filter)?;
            let operator = match filter.op {
                FilterOp::Equal => "=",
                FilterOp::NotEqual => "<>",
                FilterOp::Less => "<",
                FilterOp::LessOrEqual => "<=",
                FilterOp::Greater => ">",
                _ => ">=",
            };
            params.push(value);
            format!("({sql} {operator} {bound})")
        }
        FilterOp::In | FilterOp::NotIn => {
            if let Some(parameter) = parameter {
                return Err(multi_value_refusal(parameter));
            }
            if filter.values.is_empty() {
                return Err(PlanError::EmptyFilterValues {
                    field: filter.field.clone(),
                    op: filter.op,
                });
            }
            let mut bound = Vec::with_capacity(filter.values.len());
            for value in &filter.values {
                let (value, text) = bound_value(field, value, filter)?;
                params.push(value);
                bound.push(text);
            }
            let keyword = if filter.op == FilterOp::In {
                "IN"
            } else {
                "NOT IN"
            };
            format!("({sql} {keyword} ({}))", bound.join(", "))
        }
        FilterOp::Between => {
            if let Some(parameter) = parameter {
                return Err(multi_value_refusal(parameter));
            }
            expect(2)?;
            let (low, low_text) = bound_value(field, &filter.values[0], filter)?;
            let (high, high_text) = bound_value(field, &filter.values[1], filter)?;
            params.push(low);
            params.push(high);
            format!("({sql} BETWEEN {low_text} AND {high_text})")
        }
        FilterOp::Contains | FilterOp::StartsWith | FilterOp::EndsWith => {
            let value = single_value(&expect)?;
            if value_type != ValueType::Text {
                return Err(PlanError::IncompatibleFilterValue {
                    field: filter.field.clone(),
                    op: filter.op,
                    field_type: value_type,
                    value_type: scalar_type(value),
                });
            }
            let slickbi_spec::Scalar::Text(text) = value else {
                return Err(PlanError::IncompatibleFilterValue {
                    field: filter.field.clone(),
                    op: filter.op,
                    field_type: value_type,
                    value_type: scalar_type(value),
                });
            };
            let function = match filter.op {
                FilterOp::Contains => "contains",
                FilterOp::StartsWith => "starts_with",
                _ => "ends_with",
            };
            params.push(ParamValue::Text(text.clone()));
            format!("{function}({sql}, ?)")
        }
        FilterOp::IsNull | FilterOp::IsNotNull => {
            if let Some(parameter) = parameter {
                return Err(multi_value_refusal(parameter));
            }
            expect(0)?;
            let operator = if filter.op == FilterOp::IsNull {
                "IS NULL"
            } else {
                "IS NOT NULL"
            };
            format!("({sql} {operator})")
        }
    };

    Ok(predicate)
}

/// Binds a filter value, casting temporal text to a date or timestamp.
fn bound_value(
    field: &ResolvedField,
    value: &Scalar,
    filter: &Filter,
) -> Result<(ParamValue, String), PlanError> {
    let value_type = field.value_type();

    if value_type.is_temporal() {
        let Scalar::Text(text) = value else {
            return Err(PlanError::IncompatibleFilterValue {
                field: filter.field.clone(),
                op: filter.op,
                field_type: value_type,
                value_type: scalar_type(value),
            });
        };
        let cast = if value_type == ValueType::Date {
            "DATE"
        } else {
            "TIMESTAMP"
        };
        return Ok((ParamValue::Text(text.clone()), format!("CAST(? AS {cast})")));
    }

    let (param, scalar_type) = scalar_parts(value);
    if !value_type.comparable_with(scalar_type) {
        return Err(PlanError::IncompatibleFilterValue {
            field: filter.field.clone(),
            op: filter.op,
            field_type: value_type,
            value_type: scalar_type,
        });
    }
    Ok((param, "?".to_owned()))
}

fn scalar_parts(value: &Scalar) -> (ParamValue, ValueType) {
    match value {
        Scalar::Boolean(value) => (ParamValue::Boolean(*value), ValueType::Boolean),
        Scalar::Integer(value) => (ParamValue::Integer(*value), ValueType::Integer),
        Scalar::Number(value) => (ParamValue::Number(*value), ValueType::Number),
        Scalar::Text(value) => (ParamValue::Text(value.clone()), ValueType::Text),
    }
}

fn scalar_type(value: &Scalar) -> ValueType {
    scalar_parts(value).1
}

fn expected_count(expected: usize) -> &'static str {
    match expected {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "many",
    }
}

struct OrderTerm {
    target: OrderTarget,
    descending: bool,
}

enum OrderTarget {
    Alias(String),
    Expression(String),
}

fn order_by(
    model: &Model,
    worksheet: &Worksheet,
    items: &[SelectItem],
    params: &mut Vec<ParamValue>,
) -> Result<Vec<OrderTerm>, PlanError> {
    let mut terms = Vec::new();

    for key in &worksheet.sort {
        let field = resolve(model, &key.field)?;

        // A sort key that matches a shelf reuses its alias; anything else is emitted in full.
        if let Some(item) = items
            .iter()
            .find(|item| item.field == key.field && item.agg == key.agg)
        {
            terms.push(OrderTerm {
                target: OrderTarget::Alias(item.alias.clone()),
                descending: key.descending,
            });
            continue;
        }

        let (expression, _, _) = sort_expression(key, field)?;
        for value in field.params() {
            params.push(value.clone());
        }
        terms.push(OrderTerm {
            target: OrderTarget::Expression(expression),
            descending: key.descending,
        });
    }

    Ok(terms)
}

fn sort_expression(
    key: &SortKey,
    field: &ResolvedField,
) -> Result<(String, DataType, bool), PlanError> {
    if key.agg != Agg::None {
        if field.aggregate {
            return Err(PlanError::NestedAggregation {
                field: field.id.clone(),
            });
        }
        let (sql, data_type) = aggregate_sql(key.agg, field)?;
        return Ok((sql, data_type, true));
    }
    Ok((field.sql().to_owned(), field.data_type, field.aggregate))
}

/// Anything that can go wrong while planning a worksheet.
#[derive(Debug, Error)]
pub enum PlanError {
    /// A worksheet names a field its model does not have.
    #[error("the worksheet uses field '{field}', which is not in the model")]
    UnknownField {
        /// The field as referenced.
        field: FieldId,
    },
    /// The worksheet has no shelves, so there is nothing to query.
    #[error("the worksheet has no fields on its shelves")]
    EmptyEncoding,
    /// Fields come from more than one table; relationships arrive in 0.2.0.
    #[error(
        "this worksheet reads more than one table ({}); relationships are not supported until 0.2.0",
        tables.join(", ")
    )]
    MultipleTables {
        /// The table names.
        tables: Vec<String>,
    },
    /// An aggregation is applied to a field that is already an aggregate.
    #[error("field '{field}' is already an aggregate and cannot be aggregated again")]
    NestedAggregation {
        /// The field.
        field: FieldId,
    },
    /// A numeric aggregation is applied to a non-numeric field.
    #[error("field '{field}' is {found:?}, so it cannot be aggregated with {agg:?}")]
    NonNumericAggregation {
        /// The field.
        field: FieldId,
        /// The aggregation.
        agg: Agg,
        /// The field's type.
        found: ValueType,
    },
    /// An aggregation this release does not support.
    #[error("{agg:?} is not supported yet; it needs an ordering the planner does not emit")]
    UnsupportedAggregation {
        /// The aggregation.
        agg: Agg,
    },
    /// A transform this release does not support.
    #[error("the {kind} transform is not supported in 0.0.1")]
    UnsupportedTransform {
        /// The transform.
        kind: &'static str,
    },
    /// An `in` or `not_in` filter has no values.
    #[error("filter on field '{field}' uses {op:?} but lists no values")]
    EmptyFilterValues {
        /// The field.
        field: FieldId,
        /// The operator.
        op: FilterOp,
    },
    /// A filter has the wrong number of values for its operator.
    #[error(
        "filter on field '{field}' uses {op:?}, which expects {expected} value(s), but {found} were given"
    )]
    WrongFilterValueCount {
        /// The field.
        field: FieldId,
        /// The operator.
        op: FilterOp,
        /// A description of the expected count.
        expected: &'static str,
        /// The count supplied.
        found: usize,
    },
    /// A filter value cannot be compared with the field.
    #[error("filter on field '{field}' of type {field_type:?} cannot use a {value_type:?} value")]
    IncompatibleFilterValue {
        /// The field.
        field: FieldId,
        /// The operator.
        op: FilterOp,
        /// The field's type.
        field_type: ValueType,
        /// The value's type.
        value_type: ValueType,
    },
    /// A filter names a parameter the workbook does not contain.
    #[error(
        "the worksheet's filter uses parameter '{parameter}', which the workbook does not have"
    )]
    UnknownParameter {
        /// The parameter as referenced.
        parameter: ParamId,
    },
    /// A filter binds a parameter to an operator that needs more than one value.
    #[error("filter operator {op:?} cannot be bound to parameter '{parameter}'")]
    UnsupportedParameterFilter {
        /// The parameter.
        parameter: ParamId,
        /// The operator.
        op: FilterOp,
    },
    /// A filter bound to a parameter also holds literal values.
    #[error("filter on field '{field}' is bound to a parameter and must not also list values")]
    ParameterWithValues {
        /// The field.
        field: FieldId,
        /// The operator.
        op: FilterOp,
    },
}
