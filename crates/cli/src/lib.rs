#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The command line interface, factored so the commands are testable without spawning a process.
//!
//! Each command performs exactly what the library crates do, with the same errors, so the CLI and
//! the core can never drift apart. `validate`, `compile` and `inspect` are read-only; `query` runs
//! the planned SQL against the embedded DuckDB engine and prints the result.

use std::path::{Path, PathBuf};

use slickbi_engine::{Engine, SourceFormat};
use slickbi_model::Model;
use slickbi_planner::{PlanOptions, plan};
use slickbi_spec::Workbook;
use slickbi_workbook::Scz;
use thiserror::Error;

/// How a query result is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// An aligned, human-readable table.
    Table,
    /// Comma-separated values with a header row.
    Csv,
}

/// Validates that a `.scz` opens and its document is well-formed.
///
/// # Errors
///
/// Returns [`CliError`] when the archive or document is malformed, or the version is unsupported.
pub fn validate(path: &Path) -> Result<(), CliError> {
    Scz::open(path).map(|_| ()).map_err(CliError::from)
}

/// Prints the planned SQL for one worksheet, or every worksheet.
///
/// # Errors
///
/// Returns [`CliError`] when the workbook cannot be opened, a worksheet is missing, or a worksheet
/// cannot be planned.
pub fn compile(path: &Path, worksheet: Option<&str>) -> Result<String, CliError> {
    let scz = Scz::open(path)?;
    let workbook = scz.workbook();
    let worksheets = select_worksheets(workbook, worksheet)?;

    let mut out = String::new();
    for worksheet in worksheets {
        let model = model_for(workbook, worksheet)?;
        let query = plan(&model, worksheet, &PlanOptions::default())?;
        out.push_str(&format!("-- {}\n", worksheet.name));
        out.push_str(&query.sql);
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Prints a summary of the workbook: models, worksheets and reports.
///
/// # Errors
///
/// Returns [`CliError`] when the workbook cannot be opened.
pub fn inspect(path: &Path) -> Result<String, CliError> {
    let scz = Scz::open(path)?;
    let workbook = scz.workbook();

    let mut out = String::new();
    out.push_str(&format!("{}\n", workbook.meta.name));
    out.push_str(&format!("format version: {}\n", workbook.format_version));

    for model in &workbook.models {
        out.push_str(&format!(
            "model '{}': {} table(s), {} field(s)\n",
            model.name,
            model.tables.len(),
            model.fields.len()
        ));
    }
    for worksheet in &workbook.worksheets {
        out.push_str(&format!(
            "worksheet '{}': {:?} mark, {} filter(s)\n",
            worksheet.name,
            worksheet.mark,
            worksheet.filters.len()
        ));
    }
    for report in &workbook.reports {
        out.push_str(&format!(
            "report '{}': {} element(s)\n",
            report.name,
            report.elements.len()
        ));
    }
    Ok(out)
}

/// Runs the planned query for a worksheet and returns the printed result.
///
/// The tables the worksheet reads must be embedded in the `.scz` as Parquet extracts; a table
/// without an extract is reported rather than left to fail deep in the engine.
///
/// # Errors
///
/// Returns [`CliError`] for an unopenable workbook, a missing worksheet, a missing extract, a
/// planning failure, or an engine error.
pub fn query(path: &Path, worksheet: &str, format: OutputFormat) -> Result<String, CliError> {
    let scz = Scz::open(path)?;
    let workbook = scz.workbook();
    let worksheet = workbook
        .worksheets
        .iter()
        .find(|w| w.id.as_str() == worksheet || w.name == worksheet)
        .ok_or_else(|| CliError::WorksheetNotFound(worksheet.to_owned()))?;

    let model = model_for(workbook, worksheet)?;
    ensure_extracts(&scz, &model)?;

    let engine = Engine::open()?;
    let temp_files = register_extracts(&engine, &scz)?;

    let query = plan(&model, worksheet, &PlanOptions::default())?;
    let result = engine.execute(&query.sql, &query.params)?;

    for path in temp_files {
        std::fs::remove_file(path).ok();
    }

    match format {
        OutputFormat::Table => Ok(pretty(&result.batches)),
        OutputFormat::Csv => Ok(csv(&result.batches)),
    }
}

fn select_worksheets<'a>(
    workbook: &'a Workbook,
    name: Option<&str>,
) -> Result<Vec<&'a slickbi_spec::Worksheet>, CliError> {
    match name {
        Some(name) => {
            let worksheet = workbook
                .worksheets
                .iter()
                .find(|w| w.id.as_str() == name || w.name == name)
                .ok_or_else(|| CliError::WorksheetNotFound(name.to_owned()))?;
            Ok(vec![worksheet])
        }
        None => Ok(workbook.worksheets.iter().collect()),
    }
}

fn model_for(workbook: &Workbook, worksheet: &slickbi_spec::Worksheet) -> Result<Model, CliError> {
    let document = workbook
        .models
        .iter()
        .find(|m| m.id == worksheet.model)
        .ok_or_else(|| CliError::ModelNotFound(worksheet.model.to_string()))?;
    Ok(Model::resolve(document)?)
}

fn ensure_extracts(scz: &Scz, model: &Model) -> Result<(), CliError> {
    for table in model.tables() {
        if scz.extract(&table.name).is_none() {
            return Err(CliError::TableHasNoExtract(table.name.clone()));
        }
    }
    Ok(())
}

/// Registers every embedded extract as a table, returning the temp files to clean up later.
fn register_extracts(engine: &Engine, scz: &Scz) -> Result<Vec<PathBuf>, CliError> {
    let mut temp_files = Vec::new();
    for (table, bytes) in scz.extracts() {
        let path = std::env::temp_dir().join(format!(
            "slickbi-extract-{}-{}.parquet",
            std::process::id(),
            table
        ));
        std::fs::write(&path, bytes)?;
        engine.register(table, &path, SourceFormat::Parquet)?;
        temp_files.push(path);
    }
    Ok(temp_files)
}

fn pretty(batches: &[slickbi_engine::arrow::record_batch::RecordBatch]) -> String {
    format!(
        "{}",
        slickbi_engine::arrow::util::pretty::pretty_format_batches(batches)
            .expect("batches format")
    )
}

fn csv(batches: &[slickbi_engine::arrow::record_batch::RecordBatch]) -> String {
    use slickbi_engine::arrow::util::display::array_value_to_string;

    let Some(first) = batches.first() else {
        return String::new();
    };

    let header = first
        .schema()
        .fields()
        .iter()
        .map(|field| csv_field(field.name()))
        .collect::<Vec<_>>()
        .join(",");

    let mut out = String::with_capacity(first.num_rows() * 16);
    out.push_str(&header);
    out.push('\n');

    for batch in batches {
        for row in 0..batch.num_rows() {
            let values = (0..batch.num_columns())
                .map(|column| {
                    csv_field(&array_value_to_string(batch.column(column), row).unwrap_or_default())
                })
                .collect::<Vec<_>>();
            out.push_str(&values.join(","));
            out.push('\n');
        }
    }
    out
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// Anything that can go wrong while running a command.
#[derive(Debug, Error)]
pub enum CliError {
    /// The `.scz` could not be opened or read.
    #[error("{0}")]
    Workbook(#[from] slickbi_workbook::WorkbookError),
    /// A model could not be resolved.
    #[error("{0}")]
    Model(#[from] slickbi_model::ModelError),
    /// A worksheet could not be planned.
    #[error("{0}")]
    Plan(#[from] slickbi_planner::PlanError),
    /// The engine reported an error.
    #[error("{0}")]
    Engine(#[from] slickbi_engine::EngineError),
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// No worksheet has that id or name.
    #[error("no worksheet named '{0}'")]
    WorksheetNotFound(String),
    /// A worksheet's model does not exist in the workbook.
    #[error("worksheet refers to model '{0}', which is not in the workbook")]
    ModelNotFound(String),
    /// A table the query reads has no embedded extract.
    #[error("table '{0}' has no Parquet extract in this workbook")]
    TableHasNoExtract(String),
}
