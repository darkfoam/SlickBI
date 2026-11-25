//! Spreadsheet sources: Excel workbooks, OpenDocument spreadsheets and Apple Numbers documents.
//!
//! The engine reads CSV, Parquet and JSON as live views over the file. Spreadsheets need a parser,
//! so their first sheet (or the Numbers document's first table) is read here and imported as a
//! typed table. A formula cell contributes the value the spreadsheet application stored with it,
//! which is the calculated result the user sees.

use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use iwork::CellValue;
use slickbi_engine::{Cell, StorageType};
use slickbi_spec::{
    Column, ColumnId, ConnectionKind, DataModel, DataModelId, DataType, Field, FieldSource, Table,
    TableId,
};

use crate::temporal::{
    days_from_civil, format_date, format_timestamp, parse_display_date, parse_timestamp,
};
use crate::{
    ConnectorError, TABLE_NAME, infer_role, looks_like_money, parse_money, unique_field_id,
};

/// Apple's reference date (2001-01-01) as days since the Unix epoch.
const APPLE_EPOCH_DAYS: i64 = 11_323;

/// A spreadsheet format this build can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadsheetKind {
    /// An Excel workbook (`.xlsx`, `.xlsm`, `.xlsb`, `.xls`).
    Excel,
    /// An OpenDocument spreadsheet (`.ods`).
    OpenDocument,
    /// An Apple Numbers document (`.numbers`).
    Numbers,
}

impl SpreadsheetKind {
    /// The spreadsheet kind for a path, or `None` when the extension is not one this build reads.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "xlsx" | "xlsm" | "xlsb" | "xls" => Some(Self::Excel),
            "ods" => Some(Self::OpenDocument),
            "numbers" => Some(Self::Numbers),
            _ => None,
        }
    }

    /// The connection kind recorded in the workbook.
    pub fn connection_kind(self) -> ConnectionKind {
        match self {
            SpreadsheetKind::Excel => ConnectionKind::Excel,
            SpreadsheetKind::OpenDocument => ConnectionKind::OpenDocument,
            SpreadsheetKind::Numbers => ConnectionKind::Numbers,
        }
    }

    /// A human-readable name for menus and messages.
    pub fn label(self) -> &'static str {
        match self {
            SpreadsheetKind::Excel => "Excel",
            SpreadsheetKind::OpenDocument => "OpenDocument",
            SpreadsheetKind::Numbers => "Numbers",
        }
    }
}

/// A table read from a spreadsheet, ready to register in the engine and model.
pub struct SheetTable {
    /// The semantic model for the table, named [`TABLE_NAME`].
    pub model: DataModel,
    /// The columns and the DuckDB storage type each one uses.
    pub columns: Vec<(String, StorageType)>,
    /// The rows, one cell per column, in column order.
    pub rows: Vec<Vec<Cell>>,
    /// The sheet or table the rows came from.
    pub sheet: String,
}

/// Reads the first sheet of a spreadsheet and registers it as the `data` table.
///
/// # Errors
///
/// Returns [`ConnectorError::Spreadsheet`] when the file cannot be opened or holds no table, and
/// [`ConnectorError::Engine`] when the rows cannot be registered.
pub fn open_spreadsheet(
    engine: &slickbi_engine::Engine,
    path: &Path,
    kind: SpreadsheetKind,
    sheet: Option<&str>,
) -> Result<SheetTable, ConnectorError> {
    let table = read_spreadsheet(path, kind, sheet)?;
    engine.register_rows(TABLE_NAME, &table.columns, &table.rows)?;
    Ok(table)
}

/// The sheet or table names a spreadsheet exposes.
///
/// # Errors
///
/// Returns [`ConnectorError::Spreadsheet`] when the file cannot be opened.
pub fn spreadsheet_sheets(
    path: &Path,
    kind: SpreadsheetKind,
) -> Result<Vec<String>, ConnectorError> {
    match kind {
        SpreadsheetKind::Excel | SpreadsheetKind::OpenDocument => {
            let workbook = open_workbook_auto(path).map_err(sheet_error)?;
            Ok(workbook.sheet_names())
        }
        SpreadsheetKind::Numbers => {
            let document = iwork::Document::open(path).map_err(sheet_error)?;
            Ok(document
                .tables()
                .into_iter()
                .map(|table| table.name)
                .collect())
        }
    }
}

/// Reads one sheet of a spreadsheet into typed rows and a model.
///
/// `sheet` selects a sheet or table by name; `None` reads the first one.
///
/// # Errors
///
/// Returns [`ConnectorError::Spreadsheet`] when the file cannot be opened, the named sheet is
/// missing, or the file holds no table.
pub fn read_spreadsheet(
    path: &Path,
    kind: SpreadsheetKind,
    sheet: Option<&str>,
) -> Result<SheetTable, ConnectorError> {
    let raw = match kind {
        SpreadsheetKind::Excel | SpreadsheetKind::OpenDocument => read_workbook(path, sheet)?,
        SpreadsheetKind::Numbers => read_numbers(path, sheet)?,
    };
    build(raw)
}

/// One cell before its column's type is known.
#[derive(Clone, Debug, PartialEq)]
enum RawCell {
    Empty,
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Date(i32),
    Timestamp(i64),
}

impl RawCell {
    fn is_empty(&self) -> bool {
        matches!(self, RawCell::Empty)
    }

    /// The cell as display text, which is how a text column and a header are read.
    fn text(&self) -> Option<String> {
        match self {
            RawCell::Empty => None,
            RawCell::Integer(value) => Some(value.to_string()),
            RawCell::Number(value) => Some(format_number(*value)),
            RawCell::Boolean(value) => Some(value.to_string()),
            RawCell::Date(days) => Some(format_date(*days)),
            RawCell::Timestamp(micros) => Some(format_timestamp(*micros)),
            RawCell::Text(text) => (!text.is_empty()).then(|| text.clone()),
        }
    }
}

/// A sheet as its reader returned it.
struct RawSheet {
    sheet: String,
    rows: Vec<Vec<RawCell>>,
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn sheet_error(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError::Spreadsheet(error.to_string())
}

// ---- Excel and OpenDocument ----

fn read_workbook(path: &Path, sheet: Option<&str>) -> Result<RawSheet, ConnectorError> {
    let mut workbook = open_workbook_auto(path).map_err(sheet_error)?;
    let sheet = match sheet {
        Some(wanted) => workbook
            .sheet_names()
            .into_iter()
            .find(|name| name == wanted)
            .ok_or_else(|| sheet_error(format!("the workbook has no sheet named '{wanted}'")))?,
        None => workbook
            .sheet_names()
            .first()
            .cloned()
            .ok_or_else(|| sheet_error("the workbook has no sheets"))?,
    };
    let range = workbook.worksheet_range(&sheet).map_err(sheet_error)?;
    let rows = range
        .rows()
        .map(|row| row.iter().map(raw_cell_from_data).collect())
        .collect();
    Ok(RawSheet { sheet, rows })
}

fn raw_cell_from_data(data: &Data) -> RawCell {
    match data {
        Data::Empty => RawCell::Empty,
        Data::Int(value) => RawCell::Integer(*value),
        Data::Float(value) => raw_number(*value),
        Data::String(text) => RawCell::Text(text.clone()),
        Data::Bool(value) => RawCell::Boolean(*value),
        Data::DateTime(datetime) => {
            let (year, month, day, hour, minute, second, milli) = datetime.to_ymd_hms_milli();
            let days = days_from_civil(year as i64, month as u32, day as u32);
            if hour == 0 && minute == 0 && second == 0 && milli == 0 {
                RawCell::Date(days as i32)
            } else {
                RawCell::Timestamp(
                    days * 86_400_000_000
                        + (hour as i64 * 3_600 + minute as i64 * 60 + second as i64) * 1_000_000
                        + milli as i64 * 1_000,
                )
            }
        }
        Data::DateTimeIso(text) => {
            if let Some(micros) = parse_timestamp(text) {
                if micros.rem_euclid(86_400_000_000) == 0 {
                    RawCell::Date(micros.div_euclid(86_400_000_000) as i32)
                } else {
                    RawCell::Timestamp(micros)
                }
            } else {
                RawCell::Text(text.clone())
            }
        }
        Data::DurationIso(_) | Data::Error(_) => RawCell::Empty,
    }
}

fn raw_number(value: f64) -> RawCell {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 {
        RawCell::Integer(value as i64)
    } else {
        RawCell::Number(value)
    }
}

// ---- Apple Numbers ----

fn read_numbers(path: &Path, sheet: Option<&str>) -> Result<RawSheet, ConnectorError> {
    let document = iwork::Document::open(path).map_err(sheet_error)?;
    let tables = document.tables();
    let table = match sheet {
        Some(wanted) => tables
            .into_iter()
            .find(|table| table.name == wanted)
            .ok_or_else(|| sheet_error(format!("the document has no table named '{wanted}'")))?,
        None => tables
            .into_iter()
            .next()
            .ok_or_else(|| sheet_error("the Numbers document has no table"))?,
    };
    let height = table.rows;
    let width = table.columns;
    let mut rows = Vec::with_capacity(height);
    for row in 0..height {
        let mut cells = Vec::with_capacity(width);
        for column in 0..width {
            cells.push(raw_cell_from_numbers(table.value(row, column)));
        }
        rows.push(cells);
    }
    Ok(RawSheet {
        sheet: table.name.clone(),
        rows,
    })
}

fn raw_cell_from_numbers(value: CellValue) -> RawCell {
    match value {
        CellValue::Empty | CellValue::Span | CellValue::Error | CellValue::Unknown(_) => {
            RawCell::Empty
        }
        CellValue::Text(text) | CellValue::RichText(text) => {
            if text.is_empty() {
                RawCell::Empty
            } else {
                RawCell::Text(text)
            }
        }
        CellValue::Bool(value) => RawCell::Boolean(value),
        CellValue::Number(decimal) => raw_number(decimal.to_f64()),
        CellValue::Currency(decimal) => RawCell::Number(decimal.to_f64()),
        CellValue::Date(seconds) => {
            let total = (seconds * 1_000_000.0).round() as i64;
            let day = 86_400_000_000;
            let days = total.div_euclid(day) + APPLE_EPOCH_DAYS;
            let rest = total.rem_euclid(day);
            if rest == 0 {
                RawCell::Date(days as i32)
            } else {
                RawCell::Timestamp(days * day + rest)
            }
        }
        CellValue::Duration(_) => RawCell::Empty,
    }
}

// ---- Typing ----

/// The type a column's cells agree on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Number,
    Boolean,
    Date,
    Timestamp,
    Text,
    Empty,
}

fn build(raw: RawSheet) -> Result<SheetTable, ConnectorError> {
    let mut rows: Vec<Vec<RawCell>> = raw
        .rows
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .collect();
    if rows.is_empty() {
        return Err(sheet_error("the sheet is empty"));
    }
    let width = rows
        .iter()
        .map(|row| {
            row.iter()
                .rposition(|cell| !cell.is_empty())
                .map_or(0, |index| index + 1)
        })
        .max()
        .unwrap_or(0);
    if width == 0 {
        return Err(sheet_error("the sheet is empty"));
    }
    for row in &mut rows {
        row.resize(width, RawCell::Empty);
    }

    let header = rows.remove(0);
    let mut used_names = std::collections::HashSet::new();
    let names: Vec<String> = (0..width)
        .map(|index| {
            let base = header
                .get(index)
                .and_then(RawCell::text)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let base = if base.is_empty() {
                format!("Column {}", index + 1)
            } else {
                base
            };
            let mut candidate = base.clone();
            let mut suffix = 2;
            while !used_names.insert(candidate.clone()) {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            candidate
        })
        .collect();

    let kinds: Vec<Kind> = (0..width)
        .map(|index| infer_kind(rows.iter().filter_map(|row| row.get(index))))
        .collect();

    let mut columns = Vec::with_capacity(width);
    let mut spec_columns = Vec::with_capacity(width);
    let mut fields = Vec::with_capacity(width);
    let mut field_names = std::collections::HashSet::new();
    for (index, name) in names.iter().enumerate() {
        let (storage, data_type) = storage_for(kinds[index]);
        columns.push((name.clone(), storage));
        spec_columns.push(Column {
            id: ColumnId::new(format!("col_{index}")),
            name: name.clone(),
            data_type,
            nullable: true,
        });
        fields.push(Field {
            id: unique_field_id(name, &mut field_names),
            name: name.clone(),
            table: TableId::new(TABLE_NAME),
            source: FieldSource::Column {
                column: ColumnId::new(format!("col_{index}")),
            },
            role: infer_role(data_type),
            data_type,
            format: None,
            hidden: false,
            description: None,
        });
    }

    let rows = rows
        .iter()
        .map(|row| {
            (0..width)
                .map(|index| {
                    row.get(index)
                        .map_or(Cell::Null, |cell| convert(kinds[index], cell))
                })
                .collect()
        })
        .collect();

    Ok(SheetTable {
        model: DataModel {
            id: DataModelId::new("m1"),
            name: "Data".to_owned(),
            tables: vec![Table {
                id: TableId::new(TABLE_NAME),
                name: TABLE_NAME.to_owned(),
                columns: spec_columns,
            }],
            fields,
            relationships: vec![],
        },
        columns,
        rows,
        sheet: raw.sheet,
    })
}

fn infer_kind<'a>(cells: impl Iterator<Item = &'a RawCell>) -> Kind {
    let mut integer = false;
    let mut number = false;
    let mut boolean = false;
    let mut date = false;
    let mut timestamp = false;
    let mut money = false;
    let mut text = false;
    for cell in cells {
        match cell {
            RawCell::Empty => {}
            RawCell::Integer(_) => integer = true,
            RawCell::Number(_) => number = true,
            RawCell::Boolean(_) => boolean = true,
            RawCell::Date(_) => date = true,
            RawCell::Timestamp(_) => timestamp = true,
            RawCell::Text(value) if looks_like_money(value) => money = true,
            RawCell::Text(value) if parse_display_date(value).is_some() => date = true,
            RawCell::Text(_) => text = true,
        }
    }
    if text {
        Kind::Text
    } else if date || timestamp {
        if timestamp || number || money {
            Kind::Timestamp
        } else {
            Kind::Date
        }
    } else if number || money {
        Kind::Number
    } else if integer {
        Kind::Integer
    } else if boolean {
        Kind::Boolean
    } else {
        Kind::Empty
    }
}

fn storage_for(kind: Kind) -> (StorageType, DataType) {
    match kind {
        Kind::Integer => (StorageType::BigInt, DataType::Integer),
        Kind::Number => (StorageType::Double, DataType::Number),
        Kind::Boolean => (StorageType::Boolean, DataType::Boolean),
        Kind::Date => (StorageType::Date, DataType::Date),
        Kind::Timestamp => (StorageType::Timestamp, DataType::Timestamp),
        Kind::Text => (StorageType::VarChar, DataType::Text),
        Kind::Empty => (StorageType::VarChar, DataType::Text),
    }
}

fn convert(kind: Kind, cell: &RawCell) -> Cell {
    match kind {
        Kind::Integer => match cell {
            RawCell::Integer(value) => Cell::Integer(*value),
            RawCell::Number(value) if value.fract() == 0.0 => Cell::Integer(*value as i64),
            RawCell::Text(text) => parse_money(text)
                .map(|value| Cell::Integer(value.round() as i64))
                .unwrap_or(Cell::Null),
            _ => Cell::Null,
        },
        Kind::Number => match cell {
            RawCell::Integer(value) => Cell::Number(*value as f64),
            RawCell::Number(value) => Cell::Number(*value),
            RawCell::Text(text) => parse_money(text).map(Cell::Number).unwrap_or(Cell::Null),
            _ => Cell::Null,
        },
        Kind::Boolean => match cell {
            RawCell::Boolean(value) => Cell::Boolean(*value),
            _ => Cell::Null,
        },
        Kind::Date => match cell {
            RawCell::Date(days) => Cell::Date(*days),
            RawCell::Timestamp(micros) => Cell::Date(micros.div_euclid(86_400_000_000) as i32),
            RawCell::Text(text) => parse_display_date(text)
                .map(Cell::Date)
                .unwrap_or(Cell::Null),
            _ => Cell::Null,
        },
        Kind::Timestamp => match cell {
            RawCell::Timestamp(micros) => Cell::Timestamp(*micros),
            RawCell::Date(days) => Cell::Timestamp(*days as i64 * 86_400_000_000),
            RawCell::Text(text) => parse_timestamp(text)
                .or_else(|| parse_display_date(text).map(|days| days as i64 * 86_400_000_000))
                .map(Cell::Timestamp)
                .unwrap_or(Cell::Null),
            _ => Cell::Null,
        },
        Kind::Text => cell.text().map(Cell::Text).unwrap_or(Cell::Null),
        Kind::Empty => Cell::Null,
    }
}
