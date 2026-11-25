#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The command line interface, factored so the commands are testable without spawning a process.
//!
//! Each command performs exactly what the library crates do, with the same errors, so the CLI and
//! the core can never drift apart. `validate`, `compile` and `inspect` are read-only; `query` runs
//! the planned SQL against the embedded DuckDB engine and prints the result.

use std::path::{Path, PathBuf};

use slickbi_engine::{Engine, SourceFormat};
pub use slickbi_entitlement::{Entitlement, EntitlementError, Feature, Plan};
use slickbi_model::Model;
use slickbi_planner::{PlanOptions, plan};
use slickbi_scene::Scene;
use slickbi_spec::{ChartElement, Workbook, Worksheet};
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

/// Prints the JSON Schema for a workbook, generated from the document types.
///
/// This is the published schema (O01, O02). It cannot drift from the serialiser because both are
/// produced from the same Rust types.
///
/// # Errors
///
/// Returns [`CliError::Json`] if the schema cannot be serialised.
pub fn schema() -> Result<String, CliError> {
    Ok(serde_json::to_string_pretty(
        &slickbi_spec::workbook_schema(),
    )?)
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
    let worksheet = find_worksheet(workbook, worksheet)?;

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

/// Renders a worksheet to the free, watermarked 1× PNG preview.
///
/// This is the preview the free tier may always make. It is 1× and carries a watermark; the
/// unwatermarked print export is [`render_print`] and needs Pro.
///
/// # Errors
///
/// Returns [`CliError`] for the same reasons as [`query`], plus scene and rendering failures.
pub fn render_preview(
    path: &Path,
    worksheet: &str,
    out: &Path,
    size: (f32, f32),
) -> Result<(), CliError> {
    let scene = load_scene(path, worksheet, size)?;
    let png = slickbi_render::render_preview_png(&scene)?;
    std::fs::write(out, png)?;
    Ok(())
}

/// Renders a worksheet to a print-quality PNG, which requires a Pro entitlement.
///
/// # Errors
///
/// Returns [`CliError::Entitlement`] on the free tier, and the same errors as [`query`] plus scene
/// and rendering failures.
pub fn render_print(
    path: &Path,
    worksheet: &str,
    out: &Path,
    size: (f32, f32),
    scale: f32,
    entitlement: Entitlement,
) -> Result<(), CliError> {
    entitlement.require(Feature::PrintPng)?;
    let scene = load_scene(path, worksheet, size)?;
    let png = slickbi_render::render_png(&scene, scale)?;
    std::fs::write(out, png)?;
    Ok(())
}

/// Renders a worksheet to a standalone SVG, which requires a Pro entitlement.
///
/// # Errors
///
/// Returns [`CliError::Entitlement`] on the free tier, and the same errors as [`query`] plus scene
/// failures.
pub fn render_svg(
    path: &Path,
    worksheet: &str,
    out: &Path,
    size: (f32, f32),
    entitlement: Entitlement,
) -> Result<(), CliError> {
    entitlement.require(Feature::Svg)?;
    let scene = load_scene(path, worksheet, size)?;
    std::fs::write(out, slickbi_render::render_svg(&scene))?;
    Ok(())
}

/// Plans, executes and builds the scene for one worksheet's embedded extract.
fn load_scene(path: &Path, worksheet: &str, size: (f32, f32)) -> Result<Scene, CliError> {
    let scz = Scz::open(path)?;
    let workbook = scz.workbook();
    let worksheet = find_worksheet(workbook, worksheet)?;
    let model = model_for(workbook, worksheet)?;
    ensure_extracts(&scz, &model)?;

    let engine = Engine::open()?;
    let temp_files = register_extracts(&engine, &scz)?;
    let scene = scene_for(
        &engine,
        workbook,
        worksheet,
        size,
        slickbi_scene::SceneOptions::default(),
    );
    for path in temp_files {
        std::fs::remove_file(path).ok();
    }
    scene
}

/// Plans, executes and builds a scene for a worksheet against an already-open engine.
fn scene_for(
    engine: &Engine,
    workbook: &Workbook,
    worksheet: &Worksheet,
    size: (f32, f32),
    options: slickbi_scene::SceneOptions,
) -> Result<Scene, CliError> {
    let model = model_for(workbook, worksheet)?;
    let query = plan(&model, worksheet, &PlanOptions::default())?;
    let result = engine.execute(&query.sql, &query.params)?;
    let spec = slickbi_scene::SceneSpec {
        mark: worksheet.mark,
        columns: &query.columns,
        batches: &result.batches,
        size,
        margins: slickbi_scene::Margins::default(),
        options,
    };
    Ok(slickbi_scene::build(&spec)?)
}

/// Renders a report to the free, watermarked PNG preview.
///
/// Chart elements are rebuilt from their worksheets at the element's size, with the report's
/// presentation overrides applied; text elements are drawn line by line. The document header is
/// part of the report name, not the canvas.
///
/// # Errors
///
/// Returns [`CliError::ReportNotFound`] when the report is missing, and the same errors as
/// [`query`] plus scene and rendering failures for each chart element.
pub fn render_report_preview(
    path: &Path,
    report: &str,
    out: &Path,
    scale: f32,
) -> Result<(), CliError> {
    let scz = Scz::open(path)?;
    let workbook = scz.workbook();
    let report = workbook
        .reports
        .iter()
        .find(|candidate| candidate.id.as_str() == report || candidate.name == report)
        .ok_or_else(|| CliError::ReportNotFound(report.to_owned()))?;

    let engine = Engine::open()?;
    let temp_files = register_extracts(&engine, &scz)?;

    // Build every chart scene first, so the layers can borrow them without the vector moving.
    let mut scenes: Vec<Option<Scene>> = Vec::with_capacity(report.elements.len());
    for element in &report.elements {
        match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => {
                let Some(worksheet) = workbook
                    .worksheets
                    .iter()
                    .find(|worksheet| worksheet.id == chart.worksheet)
                else {
                    scenes.push(None);
                    continue;
                };
                let model = model_for(workbook, worksheet)?;
                ensure_extracts(&scz, &model)?;
                let layout = chart_layout(element, chart, worksheet);
                let options = slickbi_scene::SceneOptions {
                    show_axes: chart.overrides.show_axes.unwrap_or(true),
                    show_legend: chart.overrides.show_legend.unwrap_or(true),
                };
                scenes.push(Some(scene_for(
                    &engine,
                    workbook,
                    worksheet,
                    (layout.chart_width.max(1.0), layout.chart_height.max(1.0)),
                    options,
                )?));
            }
            slickbi_spec::ElementKind::Text { .. } => scenes.push(None),
        }
    }

    // Titles own their strings, so the layers can borrow them for the render call.
    let titles: Vec<Option<String>> = report
        .elements
        .iter()
        .map(|element| match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => workbook
                .worksheets
                .iter()
                .find(|worksheet| worksheet.id == chart.worksheet)
                .and_then(|worksheet| chart_layout(element, chart, worksheet).title),
            _ => None,
        })
        .collect();

    let mut layers = Vec::with_capacity(report.elements.len());
    for (index, element) in report.elements.iter().enumerate() {
        match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => {
                let worksheet = workbook
                    .worksheets
                    .iter()
                    .find(|worksheet| worksheet.id == chart.worksheet);
                let layout = worksheet.map(|worksheet| chart_layout(element, chart, worksheet));
                let (x, y, width, height) = layout
                    .map(|layout| (layout.x, layout.y, layout.width, layout.height))
                    .unwrap_or((
                        element.frame.x as f32,
                        element.frame.y as f32,
                        element.frame.width as f32,
                        element.frame.height as f32,
                    ));
                layers.push(slickbi_render::ReportLayer {
                    x,
                    y,
                    width,
                    height,
                    scene: scenes[index].as_ref(),
                    text: None,
                    title: titles[index].as_deref(),
                    border: chart.overrides.border == Some(true),
                    background: None,
                });
            }
            slickbi_spec::ElementKind::Text { text } => {
                layers.push(slickbi_render::ReportLayer {
                    x: element.frame.x as f32,
                    y: element.frame.y as f32,
                    width: element.frame.width as f32,
                    height: element.frame.height as f32,
                    scene: None,
                    text: Some(&text.markdown),
                    title: None,
                    border: false,
                    background: None,
                });
            }
        }
    }

    let (background, ink) = report_colors(report.theme.as_deref());
    let png = slickbi_render::render_report_png(
        (report.canvas.width as f32, report.canvas.height as f32),
        scale,
        background,
        ink,
        &layers,
        true,
    )?;
    std::fs::write(out, png)?;
    for path in temp_files {
        std::fs::remove_file(path).ok();
    }
    Ok(())
}

/// The drawing rectangle of a chart element after padding and its title band.
struct ChartLayout {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    chart_width: f32,
    chart_height: f32,
    title: Option<String>,
}

/// Applies an element's padding and title band, mirroring what the compositor will draw.
fn chart_layout(
    element: &slickbi_spec::Element,
    chart: &ChartElement,
    worksheet: &Worksheet,
) -> ChartLayout {
    let padding = chart.overrides.padding.unwrap_or(0.0) as f32;
    let x = element.frame.x as f32 + padding;
    let y = element.frame.y as f32 + padding;
    let width = (element.frame.width as f32 - padding * 2.0).max(1.0);
    let height = (element.frame.height as f32 - padding * 2.0).max(1.0);

    let show_title = chart.overrides.show_title.unwrap_or(true);
    let title = chart.title.clone().or_else(|| Some(worksheet.name.clone()));
    let has_title =
        show_title && title.is_some() && height > slickbi_render::REPORT_TITLE_BAND + 40.0;
    let chart_height = if has_title {
        height - slickbi_render::REPORT_TITLE_BAND
    } else {
        height
    };

    ChartLayout {
        x,
        y,
        width,
        height,
        chart_width: width,
        chart_height,
        title: if has_title { title } else { None },
    }
}

/// The background and ink colours for a report theme.
fn report_colors(theme: Option<&str>) -> ([f32; 4], [f32; 4]) {
    match theme {
        Some("slate") => ([0.106, 0.122, 0.141, 1.0], [0.925, 0.937, 0.949, 1.0]),
        Some("warm") => ([0.984, 0.969, 0.941, 1.0], [0.169, 0.149, 0.133, 1.0]),
        _ => ([1.0, 1.0, 1.0, 1.0], [0.141, 0.173, 0.212, 1.0]),
    }
}

fn find_worksheet<'a>(
    workbook: &'a Workbook,
    name: &str,
) -> Result<&'a slickbi_spec::Worksheet, CliError> {
    workbook
        .worksheets
        .iter()
        .find(|w| w.id.as_str() == name || w.name == name)
        .ok_or_else(|| CliError::WorksheetNotFound(name.to_owned()))
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
    // A per-call suffix keeps concurrent imports in one process from sharing a temp file.
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut temp_files = Vec::new();
    for (table, bytes) in scz.extracts() {
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "slickbi-extract-{}-{unique}-{}.parquet",
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
    /// A scene could not be built.
    #[error("{0}")]
    Scene(#[from] slickbi_scene::SceneError),
    /// A scene could not be rendered.
    #[error("{0}")]
    Render(#[from] slickbi_render::RenderError),
    /// A paid feature was requested without an entitlement.
    #[error("{0}")]
    Entitlement(#[from] slickbi_entitlement::EntitlementError),
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// JSON could not be produced or parsed.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// No worksheet has that id or name.
    #[error("no worksheet named '{0}'")]
    WorksheetNotFound(String),
    /// No report has that id or name.
    #[error("no report named '{0}'")]
    ReportNotFound(String),
    /// A worksheet's model does not exist in the workbook.
    #[error("worksheet refers to model '{0}', which is not in the workbook")]
    ModelNotFound(String),
    /// A table the query reads has no embedded extract.
    #[error("table '{0}' has no Parquet extract in this workbook")]
    TableHasNoExtract(String),
}
