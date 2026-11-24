#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Resolution of a document model into the bindings a query planner needs.
//!
//! A [`DataModel`] in the document says where data comes from and what the user calls it. This
//! crate turns that into one [`ResolvedField`] per field: the SQL expression it stands for, its
//! type, whether it aggregates, and every parameter the expression binds. Column fields resolve
//! directly; calculated fields are compiled by [`slickbi_calc`] against the fields already
//! resolved, in whatever order the document happens to list them.
//!
//! Resolution is the only place a calculated field is compiled, so a broken expression is
//! reported once, against the field that owns it, instead of leaking into every worksheet.

use std::collections::{BTreeMap, BTreeSet};

use slickbi_calc::{CalcError, Compiled, FieldBinding, FieldResolver, ParamValue, ValueType};
use slickbi_spec::{DataModel, DataType, Field, FieldSource, Role, Table};
use slickbi_spec::{DataModelId, FieldId, TableId};
use thiserror::Error;

/// A table, ready to appear in a `FROM` clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTable {
    /// Identifier of the source table.
    pub id: TableId,
    /// The name as the source presents it.
    pub name: String,
    /// The identifier quoted for SQL, with an alias equal to the table name.
    pub sql_name: String,
    /// The columns it exposes.
    pub columns: Vec<ResolvedColumn>,
}

/// A column of a resolved table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedColumn {
    /// Identifier of the source column.
    pub id: slickbi_spec::ColumnId,
    /// The name as the source presents it.
    pub name: String,
    /// The classified type.
    pub data_type: DataType,
}

/// A field, ready to appear in a query.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedField {
    /// Identifier of the source field.
    pub id: FieldId,
    /// The name and SQL the calculation language resolves this field to.
    pub binding: FieldBinding,
    /// The role the model gives the field.
    pub role: Role,
    /// The type shown to the user. Declared where the model declares one, derived otherwise.
    pub data_type: DataType,
    /// The table the field is declared against.
    pub table: TableId,
    /// Whether the field's SQL contains an aggregate function.
    pub aggregate: bool,
    /// The tables the field reads, including through other calculated fields.
    pub tables: Vec<TableId>,
    /// The expression source when the field is calculated.
    pub expression: Option<String>,
}

impl ResolvedField {
    /// The name used inside brackets.
    pub fn name(&self) -> &str {
        &self.binding.name
    }

    /// The SQL expression, with `?` placeholders for parameter values.
    pub fn sql(&self) -> &str {
        &self.binding.sql
    }

    /// The type of the field's values.
    pub fn value_type(&self) -> ValueType {
        self.binding.value_type
    }

    /// The values for the field's placeholders, in order.
    ///
    /// Every time a field's SQL is emitted into a query, these values must be appended in the
    /// same order, because each emission repeats the placeholders.
    pub fn params(&self) -> &[ParamValue] {
        &self.binding.params
    }
}

/// A model whose fields have all been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    id: DataModelId,
    name: String,
    tables: Vec<ResolvedTable>,
    fields: Vec<ResolvedField>,
    by_name: BTreeMap<String, usize>,
}

impl Model {
    /// Resolves every field of a document model.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] when a field names a table or column that does not exist, when two
    /// fields share a name, when a calculation is invalid, when calculated fields refer to each
    /// other in a cycle, or when a calculation's type contradicts its declaration.
    pub fn resolve(model: &DataModel) -> Result<Model, ModelError> {
        let tables = resolve_tables(model)?;
        let table_index: BTreeMap<&str, &ResolvedTable> = tables
            .iter()
            .map(|table| (table.id.as_str(), table))
            .collect();

        let mut seen = BTreeSet::new();
        for field in &model.fields {
            if !seen.insert(field.name.as_str()) {
                return Err(ModelError::DuplicateFieldName {
                    name: field.name.clone(),
                });
            }
        }

        let mut resolved: Vec<Option<ResolvedField>> = vec![None; model.fields.len()];

        for (index, field) in model.fields.iter().enumerate() {
            match &field.source {
                FieldSource::Column { column } => {
                    let table = table_index.get(field.table.as_str()).ok_or_else(|| {
                        ModelError::UnknownTable {
                            field: field.name.clone(),
                            table: field.table.as_str().to_owned(),
                        }
                    })?;
                    let column = table
                        .columns
                        .iter()
                        .find(|candidate| &candidate.id == column)
                        .ok_or_else(|| ModelError::UnknownColumn {
                            field: field.name.clone(),
                            table: table.name.clone(),
                            column: column.as_str().to_owned(),
                        })?;

                    resolved[index] = Some(ResolvedField {
                        id: field.id.clone(),
                        binding: FieldBinding::new(
                            field.name.clone(),
                            format!("{}.{}", table.sql_name, quote_ident(&column.name)),
                            value_type_of(column.data_type),
                        ),
                        role: field.role,
                        data_type: column.data_type,
                        table: field.table.clone(),
                        aggregate: false,
                        tables: vec![field.table.clone()],
                        expression: None,
                    });
                }
                FieldSource::Expression { .. } => {}
            }
        }

        resolve_expressions(model, &table_index, &mut resolved)?;

        let fields: Vec<ResolvedField> = resolved
            .into_iter()
            .map(|field| field.expect("every field is resolved"))
            .collect();

        let by_name = fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.binding.name.clone(), index))
            .collect();

        Ok(Model {
            id: model.id.clone(),
            name: model.name.clone(),
            tables,
            fields,
            by_name,
        })
    }

    /// Identifier of the source model.
    pub fn id(&self) -> &DataModelId {
        &self.id
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The model's tables, in document order.
    pub fn tables(&self) -> &[ResolvedTable] {
        &self.tables
    }

    /// A table by identifier.
    pub fn table(&self, id: &TableId) -> Option<&ResolvedTable> {
        self.tables.iter().find(|table| &table.id == id)
    }

    /// The model's fields, in document order.
    pub fn fields(&self) -> &[ResolvedField] {
        &self.fields
    }

    /// A field by identifier.
    pub fn field(&self, id: &FieldId) -> Option<&ResolvedField> {
        self.fields.iter().find(|field| &field.id == id)
    }

    /// A field by the name used inside brackets.
    pub fn field_by_name(&self, name: &str) -> Option<&ResolvedField> {
        self.by_name.get(name).map(|index| &self.fields[*index])
    }
}

impl FieldResolver for Model {
    fn resolve_field(&self, name: &str) -> Option<&FieldBinding> {
        self.field_by_name(name).map(|field| &field.binding)
    }
}

/// Resolves calculated fields, allowing them to refer to each other in any document order.
fn resolve_expressions(
    model: &DataModel,
    tables: &BTreeMap<&str, &ResolvedTable>,
    resolved: &mut [Option<ResolvedField>],
) -> Result<(), ModelError> {
    let mut pending: Vec<usize> = model
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| matches!(field.source, FieldSource::Expression { .. }))
        .map(|(index, _)| index)
        .collect();

    while !pending.is_empty() {
        let pending_names: BTreeSet<&str> = pending
            .iter()
            .map(|index| model.fields[*index].name.as_str())
            .collect();

        let mut deferred = Vec::new();
        let mut progressed = false;

        for index in pending {
            let field = &model.fields[index];
            let FieldSource::Expression { expression } = &field.source else {
                unreachable!("only expression fields are pending");
            };

            let known: Vec<ResolvedField> = resolved.iter().flatten().cloned().collect();
            match compile_expression(field, expression, &known, tables) {
                Ok(resolved_field) => {
                    resolved[index] = Some(resolved_field);
                    progressed = true;
                }
                Err(ModelError::Expression {
                    source: CalcError::UnknownField { ref name, .. },
                    ..
                }) if pending_names.contains(name.as_str()) => deferred.push(index),
                Err(error) => return Err(error),
            }
        }

        if !progressed {
            return Err(ModelError::Cycle {
                fields: deferred
                    .iter()
                    .map(|index| model.fields[*index].name.clone())
                    .collect(),
            });
        }

        pending = deferred;
    }

    Ok(())
}

/// Compiles one calculated field and checks it against its declaration.
fn compile_expression(
    field: &Field,
    source: &str,
    known: &[ResolvedField],
    tables: &BTreeMap<&str, &ResolvedTable>,
) -> Result<ResolvedField, ModelError> {
    let expression_error = |source: CalcError| ModelError::Expression {
        field: field.name.clone(),
        source,
    };

    let resolver = KnownFields(known);
    let compiled: Compiled = slickbi_calc::compile(source, &resolver).map_err(expression_error)?;
    let parsed = slickbi_calc::parse(source).map_err(expression_error)?;

    let mut referenced = Vec::new();
    collect_references(&parsed, &mut referenced);

    let mut used_tables = BTreeSet::new();
    for name in referenced {
        let referenced_field = known
            .iter()
            .find(|candidate| candidate.binding.name == name);
        let referenced_field = referenced_field.ok_or_else(|| ModelError::Expression {
            field: field.name.clone(),
            source: CalcError::UnknownField {
                name: name.clone(),
                span: calc_span(&parsed, &name),
            },
        })?;
        used_tables.extend(referenced_field.tables.iter().cloned());
    }

    let actual_type = compiled.value_type.data_type();
    let declared_type = match actual_type {
        Some(actual) if field.data_type != DataType::Unknown && field.data_type != actual => {
            return Err(ModelError::TypeMismatch {
                field: field.name.clone(),
                declared: field.data_type,
                actual,
            });
        }
        Some(actual) if field.data_type == DataType::Unknown => actual,
        _ => field.data_type,
    };

    if !used_tables.is_empty() && !used_tables.contains(&field.table) {
        return Err(ModelError::UnknownTable {
            field: field.name.clone(),
            table: field.table.as_str().to_owned(),
        });
    }

    if used_tables.is_empty() {
        if !tables.contains_key(field.table.as_str()) {
            return Err(ModelError::UnknownTable {
                field: field.name.clone(),
                table: field.table.as_str().to_owned(),
            });
        }
        used_tables.insert(field.table.clone());
    }

    Ok(ResolvedField {
        id: field.id.clone(),
        binding: FieldBinding::with_params(
            field.name.clone(),
            compiled.sql,
            compiled.value_type,
            compiled.params.clone(),
        ),
        role: field.role,
        data_type: declared_type,
        table: field.table.clone(),
        aggregate: compiled.aggregate,
        tables: used_tables.into_iter().collect(),
        expression: Some(source.to_owned()),
    })
}

/// Resolves the tables, rejecting names that would make a query ambiguous.
fn resolve_tables(model: &DataModel) -> Result<Vec<ResolvedTable>, ModelError> {
    let mut tables = Vec::with_capacity(model.tables.len());
    let mut table_names = BTreeSet::new();

    for table in &model.tables {
        if !table_names.insert(table.name.as_str()) {
            return Err(ModelError::DuplicateTableName {
                name: table.name.clone(),
            });
        }

        let mut column_names = BTreeSet::new();
        for column in &table.columns {
            if !column_names.insert(column.name.as_str()) {
                return Err(ModelError::DuplicateColumnName {
                    table: table.name.clone(),
                    name: column.name.clone(),
                });
            }
        }

        tables.push(resolved_table(table));
    }

    Ok(tables)
}

fn resolved_table(table: &Table) -> ResolvedTable {
    ResolvedTable {
        id: table.id.clone(),
        name: table.name.clone(),
        sql_name: quote_ident(&table.name),
        columns: table
            .columns
            .iter()
            .map(|column| ResolvedColumn {
                id: column.id.clone(),
                name: column.name.clone(),
                data_type: column.data_type,
            })
            .collect(),
    }
}

/// Quotes an identifier for SQL, doubling embedded quotes.
pub fn quote_ident(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for character in name.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// The calculation type that reads a column of this type.
///
/// [`DataType::Unknown`] is read as text: the source could not classify it, and text is the only
/// type that accepts every value without loss. The field keeps its unknown declared type so the
/// interface can warn about it.
fn value_type_of(data_type: DataType) -> ValueType {
    match data_type {
        DataType::Integer => ValueType::Integer,
        DataType::Number => ValueType::Number,
        DataType::Text | DataType::Unknown => ValueType::Text,
        DataType::Boolean => ValueType::Boolean,
        DataType::Date => ValueType::Date,
        DataType::Timestamp => ValueType::Timestamp,
    }
}

/// Collects the names of every field an expression refers to.
fn collect_references(expr: &slickbi_calc::Expr, out: &mut Vec<String>) {
    use slickbi_calc::ExprKind;

    match &expr.kind {
        ExprKind::Field { name } => out.push(name.clone()),
        ExprKind::Unary { operand, .. } => collect_references(operand, out),
        ExprKind::Binary { left, right, .. } => {
            collect_references(left, out);
            collect_references(right, out);
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                collect_references(arg, out);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_references(condition, out);
            collect_references(then_branch, out);
            collect_references(else_branch, out);
        }
        ExprKind::Integer(_)
        | ExprKind::Number(_)
        | ExprKind::Text(_)
        | ExprKind::Boolean(_)
        | ExprKind::Null => {}
    }
}

/// The span of the first reference to a field, for an error message.
fn calc_span(expr: &slickbi_calc::Expr, name: &str) -> slickbi_calc::Span {
    use slickbi_calc::{ExprKind, Span};

    match &expr.kind {
        ExprKind::Field { name: written } if written == name => expr.span,
        ExprKind::Unary { operand, .. } => calc_span(operand, name),
        ExprKind::Binary { left, right, .. } => {
            let left = calc_span(left, name);
            if left.start != left.end {
                left
            } else {
                calc_span(right, name)
            }
        }
        ExprKind::Call { args, .. } => args
            .iter()
            .map(|arg| calc_span(arg, name))
            .find(|span| span.start != span.end)
            .unwrap_or(Span::new(expr.span.start, expr.span.start)),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let spans = [
                calc_span(condition, name),
                calc_span(then_branch, name),
                calc_span(else_branch, name),
            ];
            spans
                .into_iter()
                .find(|span| span.start != span.end)
                .unwrap_or(Span::new(expr.span.start, expr.span.start))
        }
        _ => Span::new(expr.span.start, expr.span.start),
    }
}

/// Resolves names against fields that are already resolved.
struct KnownFields<'a>(&'a [ResolvedField]);

impl FieldResolver for KnownFields<'_> {
    fn resolve_field(&self, name: &str) -> Option<&FieldBinding> {
        self.0
            .iter()
            .find(|field| field.binding.name == name)
            .map(|field| &field.binding)
    }
}

/// Anything that can go wrong while resolving a model.
#[derive(Debug, Error)]
pub enum ModelError {
    /// A field names a table the model does not contain.
    #[error("field '{field}' belongs to table '{table}', which the model does not contain")]
    UnknownTable {
        /// The field.
        field: String,
        /// The table id as written.
        table: String,
    },
    /// A field names a column its table does not contain.
    #[error("field '{field}' refers to column '{column}', which table '{table}' does not contain")]
    UnknownColumn {
        /// The field.
        field: String,
        /// The table.
        table: String,
        /// The column id as written.
        column: String,
    },
    /// Two fields share a name, so `[Name]` would be ambiguous.
    #[error("two fields are named '{name}'; field names must be unique within a model")]
    DuplicateFieldName {
        /// The shared name.
        name: String,
    },
    /// Two tables share a name, so the query would be ambiguous.
    #[error("two tables are named '{name}'; table names must be unique within a model")]
    DuplicateTableName {
        /// The shared name.
        name: String,
    },
    /// Two columns of a table share a name.
    #[error("table '{table}' has two columns named '{name}'")]
    DuplicateColumnName {
        /// The table.
        table: String,
        /// The shared column name.
        name: String,
    },
    /// Calculated fields refer to each other in a cycle.
    #[error("calculated fields refer to each other in a cycle: {fields}", fields = fields.join(", "))]
    Cycle {
        /// The fields left unresolved.
        fields: Vec<String>,
    },
    /// A calculated field's expression is invalid.
    #[error("calculated field '{field}' is invalid: {source}")]
    Expression {
        /// The field.
        field: String,
        /// The calculation error, with the span inside the expression.
        #[source]
        source: CalcError,
    },
    /// A calculated field's declared type contradicts the expression.
    #[error("field '{field}' is declared as {declared:?}, but its expression produces {actual:?}")]
    TypeMismatch {
        /// The field.
        field: String,
        /// The declared type.
        declared: DataType,
        /// The type the expression produces.
        actual: DataType,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use slickbi_spec::{Column, ColumnId, Field};

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

    fn calculated(
        id: &str,
        name: &str,
        expression: &str,
        role: Role,
        data_type: DataType,
    ) -> Field {
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

    fn document(fields: Vec<Field>) -> DataModel {
        DataModel {
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
            fields,
            relationships: Vec::new(),
        }
    }

    fn base_fields() -> Vec<Field> {
        vec![
            field(
                "f_region",
                "Region",
                "region",
                Role::Dimension,
                DataType::Text,
            ),
            field("f_month", "Month", "month", Role::Dimension, DataType::Date),
            field(
                "f_revenue",
                "Revenue",
                "amount",
                Role::Measure,
                DataType::Number,
            ),
            field(
                "f_units",
                "Units",
                "units",
                Role::Measure,
                DataType::Integer,
            ),
        ]
    }

    fn resolve(fields: Vec<Field>) -> Result<Model, ModelError> {
        Model::resolve(&document(fields))
    }

    #[test]
    fn column_fields_resolve_to_qualified_quoted_sql() {
        let model = resolve(base_fields()).expect("resolves");
        let revenue = model.field_by_name("Revenue").expect("has Revenue");
        assert_eq!(revenue.sql(), "\"sales\".\"amount\"");
        assert_eq!(revenue.value_type(), ValueType::Number);
        assert_eq!(revenue.role, Role::Measure);
        assert!(!revenue.aggregate);
        assert_eq!(revenue.tables, vec![TableId::new("sales")]);
    }

    #[test]
    fn an_expression_field_resolves_with_its_parameters() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_margin",
            "Margin",
            "[Revenue] * 0.2",
            Role::Measure,
            DataType::Number,
        ));

        let model = resolve(fields).expect("resolves");
        let margin = model.field_by_name("Margin").expect("has Margin");
        assert_eq!(margin.sql(), "(\"sales\".\"amount\" * ?)");
        assert_eq!(margin.params(), &[ParamValue::Number(0.2)]);
        assert_eq!(margin.value_type(), ValueType::Number);
        assert!(!margin.aggregate);
        assert_eq!(margin.expression.as_deref(), Some("[Revenue] * 0.2"));
    }

    #[test]
    fn an_aggregate_expression_is_flagged() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_rate",
            "Revenue per unit",
            "SUM([Revenue]) / COUNT([Units])",
            Role::Measure,
            DataType::Number,
        ));

        let model = resolve(fields).expect("resolves");
        let rate = model.field_by_name("Revenue per unit").expect("has field");
        assert!(rate.aggregate);
        assert_eq!(
            rate.sql(),
            "(sum(\"sales\".\"amount\") / count(\"sales\".\"units\"))"
        );
    }

    #[test]
    fn expression_fields_resolve_in_any_document_order() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_double",
            "Double margin",
            "[Margin] * 2",
            Role::Measure,
            DataType::Number,
        ));
        fields.push(calculated(
            "f_margin",
            "Margin",
            "[Revenue] * 0.2",
            Role::Measure,
            DataType::Number,
        ));

        let model = resolve(fields).expect("resolves");
        let double = model.field_by_name("Double margin").expect("has field");
        assert_eq!(double.sql(), "((\"sales\".\"amount\" * ?) * ?)");
        assert_eq!(
            double.params(),
            vec![ParamValue::Number(0.2), ParamValue::Integer(2)]
        );
    }

    #[test]
    fn a_cycle_is_reported_with_the_participating_fields() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_a",
            "A",
            "[B] + 1",
            Role::Measure,
            DataType::Number,
        ));
        fields.push(calculated(
            "f_b",
            "B",
            "[A] + 1",
            Role::Measure,
            DataType::Number,
        ));

        let error = resolve(fields).expect_err("must fail");
        match error {
            ModelError::Cycle { fields } => {
                assert_eq!(fields, vec!["A".to_owned(), "B".to_owned()]);
            }
            other => panic!("expected a cycle error, got {other}"),
        }
    }

    #[test]
    fn duplicate_field_names_are_rejected() {
        let mut fields = base_fields();
        fields.push(field(
            "f_region2",
            "Region",
            "region",
            Role::Dimension,
            DataType::Text,
        ));

        let error = resolve(fields).expect_err("must fail");
        assert!(matches!(
            error,
            ModelError::DuplicateFieldName { ref name } if name == "Region"
        ));
    }

    #[test]
    fn an_unknown_column_is_reported() {
        let fields = vec![field(
            "f_x",
            "Missing",
            "does_not_exist",
            Role::Dimension,
            DataType::Text,
        )];

        let error = resolve(fields).expect_err("must fail");
        match error {
            ModelError::UnknownColumn { field, column, .. } => {
                assert_eq!(field, "Missing");
                assert_eq!(column, "does_not_exist");
            }
            other => panic!("expected an unknown column, got {other}"),
        }
    }

    #[test]
    fn an_unknown_table_is_reported() {
        let mut fields = base_fields();
        let mut orphan = field("f_x", "Orphan", "region", Role::Dimension, DataType::Text);
        orphan.table = TableId::new("elsewhere");
        fields.push(orphan);

        let error = resolve(fields).expect_err("must fail");
        assert!(matches!(
            error,
            ModelError::UnknownTable { ref field, ref table }
                if field == "Orphan" && table == "elsewhere"
        ));
    }

    #[test]
    fn an_unknown_field_inside_a_calculation_is_reported_at_its_span() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_x",
            "Broken",
            "[Revenue] + [Profit]",
            Role::Measure,
            DataType::Number,
        ));

        let error = resolve(fields).expect_err("must fail");
        match error {
            ModelError::Expression { field, source } => {
                assert_eq!(field, "Broken");
                let span = source.span().expect("has a span");
                assert_eq!(span.start, 12);
                assert_eq!(span.end, 20);
            }
            other => panic!("expected an expression error, got {other}"),
        }
    }

    #[test]
    fn a_declared_type_must_match_the_expression() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_x",
            "Wrong",
            "[Revenue] * 0.2",
            Role::Measure,
            DataType::Text,
        ));

        let error = resolve(fields).expect_err("must fail");
        match error {
            ModelError::TypeMismatch {
                declared, actual, ..
            } => {
                assert_eq!(declared, DataType::Text);
                assert_eq!(actual, DataType::Number);
            }
            other => panic!("expected a type mismatch, got {other}"),
        }
    }

    #[test]
    fn an_unknown_declared_type_adopts_the_expression_type() {
        let mut fields = base_fields();
        fields.push(calculated(
            "f_x",
            "Unknown",
            "[Revenue] * 0.2",
            Role::Measure,
            DataType::Unknown,
        ));

        let model = resolve(fields).expect("resolves");
        let field = model.field_by_name("Unknown").expect("has field");
        assert_eq!(field.data_type, DataType::Number);
    }

    #[test]
    fn tables_are_collected_across_expressions() {
        let mut document = document(base_fields());
        document.tables.push(Table {
            id: TableId::new("customers"),
            name: "customers".into(),
            columns: vec![column("name", "name", DataType::Text)],
        });
        document.fields.push(Field {
            id: FieldId::new("f_customer"),
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
        });
        document.fields.push(Field {
            id: FieldId::new("f_x"),
            name: "Cross table".into(),
            table: TableId::new("sales"),
            source: FieldSource::Expression {
                expression: "[Revenue] + 1".into(),
            },
            role: Role::Measure,
            data_type: DataType::Number,
            format: None,
            hidden: false,
            description: None,
        });

        let model = Model::resolve(&document).expect("resolves");
        let cross = model.field_by_name("Cross table").expect("has field");
        assert_eq!(cross.tables, vec![TableId::new("sales")]);

        document.tables.push(Table {
            id: TableId::new("returns"),
            name: "returns".into(),
            columns: vec![column("amount", "amount", DataType::Number)],
        });
        document.fields.push(Field {
            id: FieldId::new("f_returns"),
            name: "Returns".into(),
            table: TableId::new("returns"),
            source: FieldSource::Column {
                column: ColumnId::new("amount"),
            },
            role: Role::Measure,
            data_type: DataType::Number,
            format: None,
            hidden: false,
            description: None,
        });
        document.fields.push(Field {
            id: FieldId::new("f_net"),
            name: "Net".into(),
            table: TableId::new("sales"),
            source: FieldSource::Expression {
                expression: "[Revenue] - [Returns]".into(),
            },
            role: Role::Measure,
            data_type: DataType::Number,
            format: None,
            hidden: false,
            description: None,
        });

        let model = Model::resolve(&document).expect("resolves");
        let net = model.field_by_name("Net").expect("has field");
        assert_eq!(
            net.tables,
            vec![TableId::new("returns"), TableId::new("sales")]
        );
    }

    #[test]
    fn identifier_quoting_escapes_quotes() {
        assert_eq!(quote_ident("plain"), "\"plain\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident("with space"), "\"with space\"");
    }

    #[test]
    fn table_names_must_be_unique() {
        let mut document = document(base_fields());
        document.tables.push(document.tables[0].clone());

        let error = Model::resolve(&document).expect_err("must fail");
        assert!(matches!(
            error,
            ModelError::DuplicateTableName { ref name } if name == "sales"
        ));
    }

    #[test]
    fn column_names_must_be_unique_within_a_table() {
        let mut document = document(base_fields());
        document.tables[0]
            .columns
            .push(column("amount2", "amount", DataType::Number));

        let error = Model::resolve(&document).expect_err("must fail");
        assert!(matches!(
            error,
            ModelError::DuplicateColumnName { ref table, ref name }
                if table == "sales" && name == "amount"
        ));
    }

    #[test]
    fn the_model_resolves_field_names_for_calculations() {
        let model = resolve(base_fields()).expect("resolves");
        assert!(model.resolve_field("Revenue").is_some());
        assert!(model.resolve_field("revenue").is_none());
    }
}
