#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The SlickBI document model.
//!
//! A workbook is the single source of truth for everything the application shows: the
//! connections to data, the semantic model, the individual charts (worksheets) and the composed
//! documents (reports) built from them.
//!
//! The model is deliberately split along one rule: **worksheets own data and encoding, reports
//! own presentation**. A report element references a worksheet by id and may override only
//! presentation properties, which is what lets a report stay current when its charts change.
//!
//! # Example
//!
//! ```
//! use slickbi_spec::{Workbook, WorkbookId, WorkbookMeta, FORMAT_VERSION};
//!
//! let book = Workbook::new(
//!     WorkbookId::new("book-1"),
//!     WorkbookMeta::new("Quarterly review"),
//! );
//!
//! assert_eq!(book.format_version, FORMAT_VERSION);
//! book.validate().expect("a new workbook is valid");
//! ```

mod chart;
mod ids;
mod model;
mod report;
mod workbook;

pub use chart::{
    Agg, AggSpec, Channel, Encoding, Filter, FilterOp, Mark, Scalar, SortKey, Transform,
    WindowFrame, WindowFrameKind, WindowFunction, WindowSpec, Worksheet,
};
pub use ids::{
    ColumnId, ConnectionId, DataModelId, ElementId, FieldId, ParamId, RelationshipId, ReportId,
    ScaleId, TableId, WorkbookId, WorksheetId,
};
pub use model::{
    Cardinality, Column, Connection, ConnectionKind, CrossFilter, DataModel, DataType, Field,
    FieldFormat, FieldSource, KeyRef, Relationship, Role, Table,
};
pub use report::{
    Canvas, ChartElement, Element, ElementKind, Margins, Orientation, Overrides, PageSetup,
    PageSize, Rect, Report, ReportTarget, TextAlign, TextElement,
};
pub use workbook::{
    FORMAT_VERSION, FormatError, Parameter, Theme, ValidationError, Workbook, WorkbookMeta,
    workbook_schema,
};
