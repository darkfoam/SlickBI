//! The semantic model: where data comes from and how it is presented as fields.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{ColumnId, ConnectionId, DataModelId, FieldId, RelationshipId, TableId};

/// The type of a value as understood by the application.
///
/// This is intentionally a small set. Types that a source reports but the application cannot use
/// are mapped to [`DataType::Unknown`] rather than rejected, so that a model can be partially
/// built from an unfamiliar source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    /// A whole number.
    Integer,
    /// A number with a fractional part.
    Number,
    /// Text of arbitrary length.
    Text,
    /// A boolean.
    Boolean,
    /// A calendar date without a time.
    Date,
    /// A date with a time and, usually, a time zone.
    Timestamp,
    /// A type the application could not classify.
    Unknown,
}

/// How a field participates in analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// A field used to group, filter or label: something you slice by.
    Dimension,
    /// A field used in aggregation: something you measure.
    Measure,
}

/// A connection to a data source.
///
/// A connection never carries a secret. File paths and host names are stored so the workbook can
/// reopen its sources; passwords and keys live in the operating system keychain, or in the hosted
/// credential vault for Pro accounts, and are referenced out of band.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Identifier of this connection.
    pub id: ConnectionId,
    /// Human-readable name shown in the data pane.
    pub name: String,
    /// The kind of source.
    pub kind: ConnectionKind,
    /// A path or a host-and-database descriptor. Never a secret.
    pub location: String,
    /// Whether the user asked for this connection to be read-only.
    #[serde(default)]
    pub read_only: bool,
    /// Source-specific settings, such as a sheet name or a schema.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

/// The kinds of source the application can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// A delimited text file.
    Csv,
    /// An Excel workbook.
    Excel,
    /// An OpenDocument spreadsheet.
    OpenDocument,
    /// An Apple Numbers document.
    Numbers,
    /// A Parquet file.
    Parquet,
    /// A JSON or newline-delimited JSON file.
    Json,
    /// A SQLite database file.
    Sqlite,
    /// A PostgreSQL server.
    Postgres,
    /// A MySQL or MariaDB server.
    MySql,
}

/// A semantic model: the tables, fields and relationships a workbook analyses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataModel {
    /// Identifier of this model.
    pub id: DataModelId,
    /// Human-readable name.
    pub name: String,
    /// The tables this model draws from.
    pub tables: Vec<Table>,
    /// The fields exposed to worksheets.
    pub fields: Vec<Field>,
    /// Relationships between tables, used from 0.2.0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
}

/// A physical table or file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Identifier of this table.
    pub id: TableId,
    /// The name as the source presents it.
    pub name: String,
    /// The columns it exposes.
    pub columns: Vec<Column>,
}

/// A column as the source presents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// Identifier of this column, unique within its table.
    pub id: ColumnId,
    /// The name as the source presents it.
    pub name: String,
    /// The classified type.
    pub data_type: DataType,
    /// Whether the source permits nulls.
    #[serde(default)]
    pub nullable: bool,
}

/// Where a field's values come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldSource {
    /// A straight reference to a column.
    Column {
        /// The column this field exposes.
        column: ColumnId,
    },
    /// A calculated expression, owned by the calculation language.
    Expression {
        /// The expression source, parsed and typed by the calculation crate.
        expression: String,
    },
}

/// A field exposed to worksheets: a column, or a calculation over columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// Identifier of this field.
    pub id: FieldId,
    /// The name shown to the user; may differ from the column name.
    pub name: String,
    /// The table this field belongs to.
    pub table: TableId,
    /// Where the values come from.
    pub source: FieldSource,
    /// Whether the field groups or aggregates.
    pub role: Role,
    /// The classified type.
    pub data_type: DataType,
    /// How values are displayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FieldFormat>,
    /// Whether the field is hidden from the data pane.
    #[serde(default)]
    pub hidden: bool,
    /// The folder the field is shown under in the data pane, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// A short explanation, shown in tooltips and offered to external agents through MCP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// How a field's values are rendered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldFormat {
    /// A numeric format.
    Number {
        /// Digits after the decimal point.
        decimals: u8,
        /// Whether to group thousands.
        thousands: bool,
        /// An ISO 4217 code, or a symbol, when the value is money.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        currency: Option<String>,
    },
    /// A date or timestamp format, using the application's pattern syntax.
    Date {
        /// The pattern.
        pattern: String,
    },
    /// A text format.
    Text {
        /// Truncate display at this many characters, when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_chars: Option<u32>,
    },
}

/// One end of a relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyRef {
    /// The table.
    pub table: TableId,
    /// The columns forming the key.
    pub columns: Vec<ColumnId>,
}

/// The cardinality of a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    /// Exactly one row on each side.
    OneToOne,
    /// One row on the `from` side, many on the `to` side.
    OneToMany,
    /// Many rows on the `from` side, one on the `to` side.
    ManyToOne,
    /// Many rows on both sides.
    ManyToMany,
}

/// Which direction filters propagate across a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrossFilter {
    /// Filters flow from `from` to `to` only.
    Single,
    /// Filters flow in both directions.
    Both,
}

/// A relationship between two tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    /// Identifier of this relationship.
    pub id: RelationshipId,
    /// The originating key.
    pub from: KeyRef,
    /// The target key.
    pub to: KeyRef,
    /// How rows on each side correspond.
    pub cardinality: Cardinality,
    /// Which direction filters propagate.
    #[serde(default = "default_cross_filter")]
    pub cross_filter: CrossFilter,
}

fn default_cross_filter() -> CrossFilter {
    CrossFilter::Single
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_type_serializes_snake_case() {
        let json = serde_json::to_string(&DataType::Timestamp).expect("serialize");
        assert_eq!(json, "\"timestamp\"");
    }

    #[test]
    fn field_source_is_tagged() {
        let source = FieldSource::Column {
            column: ColumnId::new("amount"),
        };
        let json = serde_json::to_string(&source).expect("serialize");
        assert_eq!(json, r#"{"kind":"column","column":"amount"}"#);
    }

    #[test]
    fn connection_defaults_are_stable() {
        let json = r#"{
            "id": "c1",
            "name": "Sales",
            "kind": "csv",
            "location": "/tmp/sales.csv"
        }"#;
        let connection: Connection = serde_json::from_str(json).expect("deserialize");
        assert!(!connection.read_only);
        assert!(connection.options.is_empty());
    }

    #[test]
    fn connection_rejects_unknown_fields() {
        let json = r#"{
            "id": "c1",
            "name": "Sales",
            "kind": "csv",
            "location": "/tmp/sales.csv",
            "password": "hunter2"
        }"#;
        let error = serde_json::from_str::<Connection>(json).expect_err("must reject");
        assert!(error.to_string().contains("password"));
    }

    #[test]
    fn relationship_cross_filter_defaults_to_single() {
        let json = r#"{
            "id": "r1",
            "from": {"table": "orders", "columns": ["customer_id"]},
            "to": {"table": "customers", "columns": ["id"]},
            "cardinality": "many_to_one"
        }"#;
        let relationship: Relationship = serde_json::from_str(json).expect("deserialize");
        assert_eq!(relationship.cross_filter, CrossFilter::Single);
    }

    #[test]
    fn a_field_without_a_folder_or_hidden_flag_opens_with_defaults() {
        let json = r#"{
            "id": "amount",
            "name": "Amount",
            "table": "data",
            "source": {"kind": "column", "column": "col_1"},
            "role": "measure",
            "data_type": "number"
        }"#;
        let field: Field = serde_json::from_str(json).expect("an older field still opens");
        assert!(field.folder.is_none());
        assert!(!field.hidden);
        assert!(field.format.is_none());
    }
}
