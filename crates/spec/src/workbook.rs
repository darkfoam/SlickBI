//! The workbook container, its format version and its invariants.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chart::{Scalar, Worksheet};
use crate::ids::{FieldId, ParamId, WorkbookId, WorksheetId};
use crate::model::{Connection, DataModel, DataType, FieldSource};
use crate::report::Report;

/// The workbook format this build reads and writes.
///
/// Bumped only with a migration in [`crate::Workbook::load_str`]'s path and a fixture that proves
/// the previous version still opens.
pub const FORMAT_VERSION: u32 = 1;

/// Descriptive information about a workbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookMeta {
    /// The workbook title.
    pub name: String,
    /// An optional description, shown in the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// When the workbook was created, as an RFC 3339 timestamp. Informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// When the workbook was last saved, as an RFC 3339 timestamp. Informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// The application version that last wrote the file. Informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}

impl WorkbookMeta {
    /// Creates metadata with just a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            created: None,
            modified: None,
            app_version: None,
        }
    }
}

/// Colours and typography used when a worksheet or report does not override them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// The theme name.
    pub name: String,
    /// Categorical colours, as CSS colour strings, in order of use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<String>,
    /// The page or canvas background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The default text colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    /// The font family for body text and axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
}

impl Theme {
    /// A light theme with a colourblind-safe categorical palette.
    pub fn light() -> Self {
        Self {
            name: "Light".to_owned(),
            // The Okabe–Ito palette: distinguishable under the common forms of colour blindness.
            palette: [
                "#0072b2", "#e69f00", "#009e73", "#cc79a7", "#d55e00", "#56b4e9", "#f0e442",
                "#999999",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            background: Some("#ffffff".to_owned()),
            foreground: Some("#1f2937".to_owned()),
            font_family: None,
        }
    }
}

/// A named value the author can change without editing a calculation or a filter.
///
/// A calculation refers to it as `[Name]`, and a filter may bind to it instead of holding a
/// literal. The value is part of the workbook, so changing it re-renders every worksheet that
/// reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// Identifier of this parameter.
    pub id: ParamId,
    /// The name used inside brackets and in the parameter list.
    pub name: String,
    /// The type the value is compared and calculated as.
    pub data_type: DataType,
    /// The current value.
    pub value: Scalar,
}

/// A complete workbook: the single source of truth for everything the application shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Workbook {
    /// The format version this document was written in.
    pub format_version: u32,
    /// Identifier of the workbook.
    pub id: WorkbookId,
    /// Title, description and timestamps.
    pub meta: WorkbookMeta,
    /// Data sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
    /// Semantic models.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<DataModel>,
    /// Charts, one per tab.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worksheets: Vec<Worksheet>,
    /// Composed documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<Report>,
    /// Named values that calculations and filters can read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
    /// The default theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
}

impl Workbook {
    /// Creates an empty workbook in the current format version.
    pub fn new(id: WorkbookId, meta: WorkbookMeta) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            id,
            meta,
            connections: Vec::new(),
            models: Vec::new(),
            worksheets: Vec::new(),
            reports: Vec::new(),
            parameters: Vec::new(),
            theme: Some(Theme::light()),
        }
    }

    /// Returns the reports that display the given worksheet.
    ///
    /// This backs the "used in" indicator (T03) and the warning before deleting a worksheet (T05).
    pub fn referencing_reports(&self, worksheet: &WorksheetId) -> Vec<&Report> {
        self.reports
            .iter()
            .filter(|report| report.referenced_worksheets().contains(&worksheet))
            .collect()
    }

    /// Serialises the workbook as pretty-printed JSON.
    pub fn to_json_string(&self) -> Result<String, FormatError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a workbook, migrates it to the current format version and then reads it.
    ///
    /// The version is read before the rest of the document is deserialised, so that a file
    /// written by a newer build produces a version error rather than a confusing field error.
    pub fn from_json_str(json: &str) -> Result<Self, FormatError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("format_version")
            .ok_or(FormatError::MissingVersion)?
            .as_u64()
            .ok_or(FormatError::InvalidVersion)?;
        let found = u32::try_from(found).map_err(|_| FormatError::InvalidVersion)?;

        let value = migrate(value, found)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Parses, checks the version and validates the invariants.
    pub fn load_str(json: &str) -> Result<Self, FormatError> {
        let workbook = Self::from_json_str(json)?;
        workbook.validate()?;
        Ok(workbook)
    }

    /// Checks the invariants a saved workbook must satisfy.
    ///
    /// Validation is separate from parsing so that a draft being edited in memory can be invalid
    /// without being unrepresentable.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.meta.name.trim().is_empty() {
            return Err(ValidationError::EmptyName {
                entity: "workbook",
                id: self.id.to_string(),
            });
        }

        ensure_ids("connection", self.connections.iter().map(|c| c.id.as_str()))?;
        ensure_ids("model", self.models.iter().map(|m| m.id.as_str()))?;
        ensure_ids("worksheet", self.worksheets.iter().map(|w| w.id.as_str()))?;
        ensure_ids("report", self.reports.iter().map(|r| r.id.as_str()))?;
        ensure_ids("parameter", self.parameters.iter().map(|p| p.id.as_str()))?;

        let mut parameter_names = HashSet::new();
        for parameter in &self.parameters {
            if parameter.name.trim().is_empty() {
                return Err(ValidationError::EmptyName {
                    entity: "parameter",
                    id: parameter.id.to_string(),
                });
            }
            if !parameter_names.insert(parameter.name.as_str()) {
                return Err(ValidationError::DuplicateParameterName {
                    name: parameter.name.clone(),
                });
            }
        }
        let parameter_ids: HashSet<&str> = self
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect();

        for model in &self.models {
            self.validate_model(model)?;
        }

        let models: HashMap<&str, &DataModel> = self
            .models
            .iter()
            .map(|model| (model.id.as_str(), model))
            .collect();

        for worksheet in &self.worksheets {
            if worksheet.name.trim().is_empty() {
                return Err(ValidationError::EmptyName {
                    entity: "worksheet",
                    id: worksheet.id.to_string(),
                });
            }

            let Some(model) = models.get(worksheet.model.as_str()) else {
                return Err(ValidationError::UnknownModel {
                    worksheet: worksheet.id.to_string(),
                    model: worksheet.model.to_string(),
                });
            };

            let has_field = |field: &FieldId| model.fields.iter().any(|f| f.id == *field);

            for channel in worksheet.encoding.channels() {
                if !has_field(&channel.field) {
                    return Err(ValidationError::UnknownField {
                        worksheet: worksheet.id.to_string(),
                        field: channel.field.to_string(),
                    });
                }
            }

            for filter in &worksheet.filters {
                if !has_field(&filter.field) {
                    return Err(ValidationError::UnknownField {
                        worksheet: worksheet.id.to_string(),
                        field: filter.field.to_string(),
                    });
                }
                if let Some(parameter) = &filter.parameter
                    && !parameter_ids.contains(parameter.as_str())
                {
                    return Err(ValidationError::UnknownParameter {
                        worksheet: worksheet.id.to_string(),
                        parameter: parameter.to_string(),
                    });
                }
            }

            for key in &worksheet.sort {
                if !has_field(&key.field) {
                    return Err(ValidationError::UnknownField {
                        worksheet: worksheet.id.to_string(),
                        field: key.field.to_string(),
                    });
                }
            }
        }

        for report in &self.reports {
            if report.name.trim().is_empty() {
                return Err(ValidationError::EmptyName {
                    entity: "report",
                    id: report.id.to_string(),
                });
            }

            if report.canvas.width <= 0.0 || report.canvas.height <= 0.0 {
                return Err(ValidationError::InvalidCanvas {
                    report: report.id.to_string(),
                });
            }

            ensure_ids("element", report.elements.iter().map(|e| e.id.as_str()))?;

            for element in &report.elements {
                if element.frame.width <= 0.0 || element.frame.height <= 0.0 {
                    return Err(ValidationError::InvalidFrame {
                        report: report.id.to_string(),
                        element: element.id.to_string(),
                    });
                }
                // A chart element may reference a worksheet that has since been deleted. That is
                // intentional: the product keeps the report and renders a placeholder, so the
                // reference is not an invariant violation (T05).
            }
        }

        Ok(())
    }

    fn validate_model(&self, model: &DataModel) -> Result<(), ValidationError> {
        if model.name.trim().is_empty() {
            return Err(ValidationError::EmptyName {
                entity: "model",
                id: model.id.to_string(),
            });
        }

        ensure_ids("table", model.tables.iter().map(|t| t.id.as_str()))?;

        for table in &model.tables {
            if table.name.trim().is_empty() {
                return Err(ValidationError::EmptyName {
                    entity: "table",
                    id: table.id.to_string(),
                });
            }
            ensure_ids("column", table.columns.iter().map(|c| c.id.as_str()))?;
        }

        ensure_ids("field", model.fields.iter().map(|f| f.id.as_str()))?;

        for field in &model.fields {
            if field.name.trim().is_empty() {
                return Err(ValidationError::EmptyName {
                    entity: "field",
                    id: field.id.to_string(),
                });
            }

            let Some(table) = model.tables.iter().find(|t| t.id == field.table) else {
                return Err(ValidationError::UnknownTable {
                    model: model.id.to_string(),
                    field: field.id.to_string(),
                    table: field.table.to_string(),
                });
            };

            if let FieldSource::Column { column } = &field.source
                && !table.columns.iter().any(|c| c.id == *column)
            {
                return Err(ValidationError::UnknownColumn {
                    model: model.id.to_string(),
                    table: table.id.to_string(),
                    column: column.to_string(),
                });
            }
        }

        ensure_ids(
            "relationship",
            model.relationships.iter().map(|r| r.id.as_str()),
        )?;

        for relationship in &model.relationships {
            for key in [&relationship.from, &relationship.to] {
                let Some(table) = model.tables.iter().find(|t| t.id == key.table) else {
                    return Err(ValidationError::UnknownTable {
                        model: model.id.to_string(),
                        field: relationship.id.to_string(),
                        table: key.table.to_string(),
                    });
                };

                for column in &key.columns {
                    if !table.columns.iter().any(|c| c.id == *column) {
                        return Err(ValidationError::UnknownColumn {
                            model: model.id.to_string(),
                            table: table.id.to_string(),
                            column: column.to_string(),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

/// Migrates a document from `found` to [`FORMAT_VERSION`].
///
/// Every future version adds one pure step here and a fixture proving the previous version still
/// opens with identical semantics (O04). Today the only version is the current one, so this is the
/// identity, and any other version is refused before a field is read.
fn migrate(value: serde_json::Value, found: u32) -> Result<serde_json::Value, FormatError> {
    if found != FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion {
            found,
            supported: FORMAT_VERSION,
        });
    }
    Ok(value)
}

fn ensure_ids<'a>(
    entity: &'static str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<(), ValidationError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(ValidationError::EmptyId { entity });
        }
        if !seen.insert(id) {
            return Err(ValidationError::DuplicateId {
                entity,
                id: id.to_owned(),
            });
        }
    }
    Ok(())
}

/// Something that prevents a workbook from being loaded.
#[derive(Debug, Error)]
pub enum FormatError {
    /// The document is not valid JSON, or does not match the format.
    #[error("workbook is not valid: {0}")]
    Json(#[from] serde_json::Error),
    /// The document has no `format_version` field.
    #[error("workbook has no format_version; it is not a SlickBI workbook")]
    MissingVersion,
    /// The `format_version` field is not a supported integer.
    #[error("workbook format_version is not a valid version number")]
    InvalidVersion,
    /// The document was written by a build with a different format.
    #[error(
        "workbook format version {found} is not supported by this build, which reads version {supported}"
    )]
    UnsupportedVersion {
        /// The version in the document.
        found: u32,
        /// The version this build supports.
        supported: u32,
    },
    /// The document parses but breaks an invariant.
    #[error("workbook is invalid: {0}")]
    Invalid(#[from] ValidationError),
}

/// A broken invariant in a workbook.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// A required name is blank.
    #[error("{entity} '{id}' has an empty name")]
    EmptyName {
        /// The kind of entity.
        entity: &'static str,
        /// Its identifier.
        id: String,
    },
    /// An identifier is empty.
    #[error("{entity} has an empty identifier")]
    EmptyId {
        /// The kind of entity.
        entity: &'static str,
    },
    /// Two entities of the same kind share an identifier.
    #[error("two {entity} objects share the identifier '{id}'")]
    DuplicateId {
        /// The kind of entity.
        entity: &'static str,
        /// The repeated identifier.
        id: String,
    },
    /// A worksheet names a model that does not exist.
    #[error("worksheet '{worksheet}' refers to model '{model}', which does not exist")]
    UnknownModel {
        /// The worksheet.
        worksheet: String,
        /// The missing model.
        model: String,
    },
    /// A worksheet names a field that its model does not expose.
    #[error("worksheet '{worksheet}' refers to field '{field}', which its model does not expose")]
    UnknownField {
        /// The worksheet.
        worksheet: String,
        /// The missing field.
        field: String,
    },
    /// A field names a table that does not exist.
    #[error("model '{model}' field '{field}' refers to table '{table}', which does not exist")]
    UnknownTable {
        /// The model.
        model: String,
        /// The field.
        field: String,
        /// The missing table.
        table: String,
    },
    /// A field or relationship names a column that does not exist.
    #[error("model '{model}' table '{table}' has no column '{column}'")]
    UnknownColumn {
        /// The model.
        model: String,
        /// The table.
        table: String,
        /// The missing column.
        column: String,
    },
    /// A report's canvas has no area.
    #[error("report '{report}' has a canvas with no area")]
    InvalidCanvas {
        /// The report.
        report: String,
    },
    /// An element has no area.
    #[error("report '{report}' element '{element}' has no area")]
    InvalidFrame {
        /// The report.
        report: String,
        /// The element.
        element: String,
    },
    /// Two parameters share a name.
    #[error("two parameters are named '{name}'; parameter names must be unique")]
    DuplicateParameterName {
        /// The shared name.
        name: String,
    },
    /// A worksheet filter binds a parameter that does not exist.
    #[error("worksheet '{worksheet}' binds parameter '{parameter}', which does not exist")]
    UnknownParameter {
        /// The worksheet.
        worksheet: String,
        /// The missing parameter.
        parameter: String,
    },
}

/// Returns the JSON Schema for a workbook, generated from the same types that serialise it.
///
/// This is the artifact published for each format version (O02). It is generated rather than
/// written, so it cannot drift from the implementation.
pub fn workbook_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Workbook))
        .expect("a schema is always representable as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_workbook_is_valid() {
        let workbook = Workbook::new(WorkbookId::new("w1"), WorkbookMeta::new("Test"));
        assert_eq!(workbook.format_version, FORMAT_VERSION);
        assert!(workbook.validate().is_ok());
    }

    #[test]
    fn empty_name_is_rejected() {
        let workbook = Workbook::new(WorkbookId::new("w1"), WorkbookMeta::new("  "));
        assert_eq!(
            workbook.validate().expect_err("must fail"),
            ValidationError::EmptyName {
                entity: "workbook",
                id: "w1".into()
            }
        );
    }

    #[test]
    fn missing_version_is_reported_clearly() {
        let error = Workbook::from_json_str(r#"{"id":"w1"}"#).expect_err("must fail");
        assert!(matches!(error, FormatError::MissingVersion));
    }

    #[test]
    fn newer_version_is_rejected_before_field_validation() {
        let json = r#"{
            "format_version": 99,
            "id": "w1",
            "meta": {"name": "Future"},
            "something_new": true
        }"#;
        let error = Workbook::from_json_str(json).expect_err("must fail");
        match error {
            FormatError::UnsupportedVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, FORMAT_VERSION);
            }
            other => panic!("expected a version error, got {other:?}"),
        }
    }

    #[test]
    fn schema_has_the_workbook_shape() {
        let schema = workbook_schema();
        assert_eq!(schema["title"], "Workbook");
        assert!(schema["properties"]["worksheets"].is_object());
        assert!(schema["properties"]["format_version"].is_object());
    }
}
