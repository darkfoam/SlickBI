//! Typed identifiers.
//!
//! Every entity in a workbook has an opaque identifier. Identifiers are strings chosen by the
//! application, not by the user, and are never parsed for meaning. Using a distinct type per
//! entity means a worksheet id can never be passed where a report id is expected.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates an identifier from anything string-like.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Reports whether the identifier is empty, which is always invalid in a saved
            /// workbook.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id!(
    /// Identifies a workbook.
    WorkbookId
);
define_id!(
    /// Identifies a connection to a data source.
    ConnectionId
);
define_id!(
    /// Identifies a semantic model.
    DataModelId
);
define_id!(
    /// Identifies a table inside a model.
    TableId
);
define_id!(
    /// Identifies a column inside a table.
    ColumnId
);
define_id!(
    /// Identifies a relationship between two tables.
    RelationshipId
);
define_id!(
    /// Identifies a field exposed by a model, which may be a column or a calculation.
    FieldId
);
define_id!(
    /// Identifies a worksheet, which is one chart on one tab.
    WorksheetId
);
define_id!(
    /// Identifies a report, which composes worksheets into a document.
    ReportId
);
define_id!(
    /// Identifies a scale configuration.
    ScaleId
);
define_id!(
    /// Identifies a parameter.
    ParamId
);
define_id!(
    /// Identifies an element inside a report.
    ElementId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_strings() {
        let id = WorksheetId::new("sales-by-region");
        assert_eq!(id.as_str(), "sales-by-region");
        assert_eq!(id.to_string(), "sales-by-region");
        assert_eq!(WorksheetId::from("sales-by-region"), id);
        assert_eq!(WorksheetId::from(String::from("sales-by-region")), id);
        assert_eq!(id.as_ref(), "sales-by-region");
    }

    #[test]
    fn ids_serialize_transparently() {
        let id = ReportId::new("annual");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"annual\"");
        let back: ReportId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn empty_id_is_detected() {
        assert!(WorkbookId::new("").is_empty());
        assert!(!WorkbookId::new("x").is_empty());
    }

    #[test]
    fn distinct_id_types_are_not_interchangeable() {
        // This is a compile-time guarantee; the test documents it by checking that the
        // transparent representation is all that is shared.
        let worksheet = WorksheetId::new("a");
        let report = ReportId::new("a");
        assert_eq!(worksheet.as_str(), report.as_str());
    }
}
