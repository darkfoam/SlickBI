#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The application state and its logic, separated from the windowing layer so it can be tested
//! without a display.
//!
//! A [`Session`] owns the embedded engine, the workbook document, and the cached scene for the
//! active worksheet. The windowing layer mutates the session (load a file, assign a field, change
//! the mark, filter, calculate) and reads the cached scene to draw it.

use std::path::{Path, PathBuf};

use slickbi_engine::{Engine, SourceFormat};
use slickbi_model::Model;
use slickbi_planner::{PlanOptions, plan};
use slickbi_scene::{Margins, Scene, SceneSpec, build};
use slickbi_spec::{
    Agg, Channel, DataModelId, DataType, Field, FieldId, FieldSource, Filter, FilterOp, Mark,
    Report, Role, Scalar, Workbook, Worksheet, WorksheetId,
};
use slickbi_workbook::Scz;
use thiserror::Error;

/// Which shelf a field is assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// The horizontal shelf.
    X,
    /// The vertical shelf.
    Y,
}

/// A field as owned data for the data pane: its identifier and display name.
pub type FieldInfo = (FieldId, String);

/// The application state.
pub struct Session {
    engine: Engine,
    workbook: Workbook,
    source: Option<(PathBuf, SourceFormat)>,
    active: usize,
    active_report: usize,
    scene: Option<Scene>,
    error: Option<String>,
    scene_size: (f32, f32),
    temp_files: Vec<PathBuf>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    /// An empty session: no source, no worksheets.
    pub fn new() -> Session {
        Session {
            engine: Engine::open().expect("engine opens"),
            workbook: Workbook::new(
                slickbi_spec::WorkbookId::new("w1"),
                slickbi_spec::WorkbookMeta::new("Untitled"),
            ),
            source: None,
            active: 0,
            active_report: 0,
            scene: None,
            error: None,
            scene_size: (800.0, 500.0),
            temp_files: Vec::new(),
        }
    }

    /// The workbook document.
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    /// The cached scene, if one was built.
    pub fn scene(&self) -> Option<&Scene> {
        self.scene.as_ref()
    }

    /// The last error, for display.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a data source has been loaded.
    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// The number of worksheets.
    pub fn worksheet_count(&self) -> usize {
        self.workbook.worksheets.len()
    }

    /// The index of the active worksheet.
    pub fn active(&self) -> usize {
        self.active
    }

    /// The active worksheet.
    pub fn active_worksheet(&self) -> Option<&Worksheet> {
        self.workbook.worksheets.get(self.active)
    }

    /// The reports.
    pub fn reports(&self) -> &[Report] {
        &self.workbook.reports
    }

    /// The index of the active report.
    pub fn active_report(&self) -> usize {
        self.active_report
    }

    /// The active report, if any.
    pub fn active_report_doc(&self) -> Option<&Report> {
        self.workbook.reports.get(self.active_report)
    }

    /// The fields of the active model, split into dimensions and measures, as owned `(id, name)`.
    pub fn field_groups(&self) -> (Vec<FieldInfo>, Vec<FieldInfo>) {
        let Some(worksheet) = self.active_worksheet() else {
            return (Vec::new(), Vec::new());
        };
        let Some(model) = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)
        else {
            return (Vec::new(), Vec::new());
        };
        let (mut dimensions, mut measures) = (Vec::new(), Vec::new());
        for field in &model.fields {
            if field.hidden {
                continue;
            }
            let info = (field.id.clone(), field.name.clone());
            match field.role {
                Role::Measure => measures.push(info),
                Role::Dimension => dimensions.push(info),
            }
        }
        (dimensions, measures)
    }

    /// Loads a data file: discovers its schema, registers it, and adds a worksheet.
    pub fn load_file(&mut self, path: &Path, format: SourceFormat) -> Result<(), SessionError> {
        let model = slickbi_connectors::discover(&self.engine, path, format)?;
        self.engine
            .register(slickbi_connectors::TABLE_NAME, path, format)?;

        self.workbook.models.push(model);
        self.workbook
            .connections
            .push(slickbi_connectors::connection(path, format));

        let worksheet = new_worksheet(self.workbook.worksheets.len(), DataModelId::new("m1"));
        self.workbook.worksheets.push(worksheet);
        self.active = self.workbook.worksheets.len() - 1;
        self.source = Some((path.to_owned(), format));
        self.refresh();
        Ok(())
    }

    /// Adds a new, empty worksheet and makes it active.
    pub fn add_worksheet(&mut self) {
        let index = self.workbook.worksheets.len();
        let model = DataModelId::new("m1");
        self.workbook.worksheets.push(new_worksheet(index, model));
        self.active = index;
        self.refresh();
    }

    /// Makes the given worksheet active.
    pub fn set_active(&mut self, index: usize) {
        if index < self.workbook.worksheets.len() {
            self.active = index;
            self.refresh();
        }
    }

    /// Removes a worksheet and updates the active index.
    pub fn remove_worksheet(&mut self, index: usize) {
        if index >= self.workbook.worksheets.len() {
            return;
        }
        self.workbook.worksheets.remove(index);
        if self.active >= self.workbook.worksheets.len() {
            self.active = self.workbook.worksheets.len().saturating_sub(1);
        }
        self.refresh();
    }

    /// Assigns a field to a shelf on the active worksheet; measures default to [`Agg::Sum`].
    pub fn assign(&mut self, shelf: Shelf, field: &FieldId) {
        let agg = self.default_agg(field);
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        let channel = Channel {
            field: field.clone(),
            agg,
            title: None,
            scale: None,
        };
        match shelf {
            Shelf::X => worksheet.encoding.x = Some(channel),
            Shelf::Y => worksheet.encoding.y = Some(channel),
        }
        self.refresh();
    }

    /// Clears a shelf on the active worksheet.
    pub fn clear(&mut self, shelf: Shelf) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        match shelf {
            Shelf::X => worksheet.encoding.x = None,
            Shelf::Y => worksheet.encoding.y = None,
        }
        self.refresh();
    }

    /// Changes the mark of the active worksheet.
    pub fn set_mark(&mut self, mark: Mark) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        worksheet.mark = mark;
        self.refresh();
    }

    /// Adds a filter to the active worksheet and rebuilds.
    pub fn add_filter(&mut self, field: &FieldId, op: FilterOp, values: Vec<Scalar>) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        worksheet.filters.push(Filter {
            field: field.clone(),
            op,
            values,
        });
        self.refresh();
    }

    /// Removes a filter from the active worksheet and rebuilds.
    pub fn remove_filter(&mut self, index: usize) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        if index < worksheet.filters.len() {
            worksheet.filters.remove(index);
        }
        self.refresh();
    }

    /// Compiles and adds a calculated field to the active model.
    ///
    /// # Errors
    ///
    /// Returns the calculation error (with its span) if the expression is invalid.
    pub fn add_calculated_field(
        &mut self,
        name: &str,
        expression: &str,
    ) -> Result<(), slickbi_calc::CalcError> {
        let Some(worksheet) = self.active_worksheet() else {
            return Err(slickbi_calc::CalcError::UnknownFunction {
                name: "<no model>".to_owned(),
                span: slickbi_calc::Span::new(0, 0),
            });
        };
        let model_id = worksheet.model.clone();
        let Some(model) = self.workbook.models.iter().find(|m| m.id == model_id) else {
            return Err(slickbi_calc::CalcError::UnknownFunction {
                name: "<no model>".to_owned(),
                span: slickbi_calc::Span::new(0, 0),
            });
        };

        let resolved =
            Model::resolve(model).map_err(|e| slickbi_calc::CalcError::UnknownFunction {
                name: e.to_string(),
                span: slickbi_calc::Span::new(0, 0),
            })?;
        let compiled = slickbi_calc::compile(expression, &resolved)?;
        let data_type = compiled.value_type.data_type().unwrap_or(DataType::Number);
        let role = match data_type {
            DataType::Integer | DataType::Number => Role::Measure,
            _ => Role::Dimension,
        };

        let model = self
            .workbook
            .models
            .iter_mut()
            .find(|m| m.id == model_id)
            .expect("model exists");
        let field = Field {
            id: FieldId::new(name),
            name: name.to_owned(),
            table: model.tables[0].id.clone(),
            source: FieldSource::Expression {
                expression: expression.to_owned(),
            },
            role,
            data_type,
            format: None,
            hidden: false,
            description: None,
        };
        model.fields.push(field);
        self.refresh();
        Ok(())
    }

    /// Adds a report and makes it active.
    pub fn add_report(&mut self, name: &str) {
        let report = Report {
            id: slickbi_spec::ReportId::new(format!("r{}", self.workbook.reports.len())),
            name: name.to_owned(),
            target: slickbi_spec::ReportTarget::Screen,
            theme: None,
            canvas: slickbi_spec::Canvas::standard(),
            elements: Vec::new(),
        };
        self.workbook.reports.push(report);
        self.active_report = self.workbook.reports.len() - 1;
    }

    /// Places a worksheet as a chart element in the active report, auto-laid-out on a grid.
    pub fn add_chart_to_report(&mut self, worksheet: &WorksheetId) {
        let Some(report) = self.workbook.reports.get_mut(self.active_report) else {
            return;
        };
        let frame = grid_frame(report.elements.len(), report.canvas);
        let element = slickbi_spec::Element {
            id: slickbi_spec::ElementId::new(format!("e{}", report.elements.len())),
            frame,
            kind: slickbi_spec::ElementKind::Chart {
                chart: slickbi_spec::ChartElement {
                    worksheet: worksheet.clone(),
                    title: None,
                    overrides: Default::default(),
                },
            },
        };
        report.elements.push(element);
    }

    /// Adds a text block to the active report.
    pub fn add_text_to_report(&mut self, markdown: &str) {
        let Some(report) = self.workbook.reports.get_mut(self.active_report) else {
            return;
        };
        let frame = grid_frame(report.elements.len(), report.canvas);
        report.elements.push(slickbi_spec::Element {
            id: slickbi_spec::ElementId::new(format!("e{}", report.elements.len())),
            frame,
            kind: slickbi_spec::ElementKind::Text {
                text: slickbi_spec::TextElement {
                    markdown: markdown.to_owned(),
                    align: Default::default(),
                    style: None,
                },
            },
        });
    }

    /// Removes an element from the active report.
    pub fn remove_element(&mut self, index: usize) {
        let Some(report) = self.workbook.reports.get_mut(self.active_report) else {
            return;
        };
        if index < report.elements.len() {
            report.elements.remove(index);
        }
    }

    /// Exports the active worksheet's result as CSV.
    pub fn export_csv(&mut self, path: &Path) -> Result<(), SessionError> {
        let (_, batches) = self.execute_active()?;
        std::fs::write(path, to_csv(&batches))?;
        Ok(())
    }

    /// Exports the active worksheet's result as Parquet via DuckDB.
    pub fn export_parquet(&mut self, path: &Path) -> Result<(), SessionError> {
        let (query, _) = self.execute_active()?;
        let sql = format!("COPY ({}) TO ? (FORMAT PARQUET)", query.sql);
        self.engine.execute(
            &sql,
            &[slickbi_calc::ParamValue::Text(
                path.to_string_lossy().into_owned(),
            )],
        )?;
        Ok(())
    }

    fn execute_active(
        &mut self,
    ) -> Result<
        (
            slickbi_planner::Query,
            Vec<slickbi_engine::arrow::record_batch::RecordBatch>,
        ),
        SessionError,
    > {
        let Some(worksheet) = self.active_worksheet().cloned() else {
            return Err(SessionError::NoWorksheet);
        };
        let Some(document) = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)
        else {
            return Err(SessionError::NoWorksheet);
        };
        let model = Model::resolve(document)?;
        let query = plan(&model, &worksheet, &PlanOptions::default())?;
        let result = self.engine.execute(&query.sql, &query.params)?;
        Ok((query, result.batches))
    }

    fn default_agg(&self, field: &FieldId) -> Agg {
        let is_measure = self
            .active_worksheet()
            .and_then(|worksheet| {
                self.workbook
                    .models
                    .iter()
                    .find(|m| m.id == worksheet.model)
            })
            .and_then(|model| model.fields.iter().find(|f| &f.id == field))
            .is_some_and(|field| field.role == Role::Measure);
        if is_measure { Agg::Sum } else { Agg::None }
    }

    /// Builds the scene for any worksheet at a given size.
    ///
    /// Returns `Ok(None)` when the worksheet has no shelves yet (nothing to draw), and `Err`
    /// when planning, execution or scene building fails.
    pub fn scene_for(
        &mut self,
        worksheet_id: &WorksheetId,
        size: (f32, f32),
    ) -> Result<Option<Scene>, String> {
        let Some(worksheet) = self
            .workbook
            .worksheets
            .iter()
            .find(|w| &w.id == worksheet_id)
            .cloned()
        else {
            return Err("the worksheet is missing".to_owned());
        };
        if worksheet.encoding.x.is_none() || worksheet.encoding.y.is_none() {
            return Ok(None);
        }

        let document = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)
            .ok_or_else(|| "the worksheet's model is missing".to_owned())?;
        let model = Model::resolve(document).map_err(|e| e.to_string())?;
        let query = plan(&model, &worksheet, &PlanOptions::default()).map_err(|e| e.to_string())?;
        let result = self
            .engine
            .execute(&query.sql, &query.params)
            .map_err(|e| e.to_string())?;

        let spec = SceneSpec {
            mark: worksheet.mark,
            columns: &query.columns,
            batches: &result.batches,
            size,
            margins: Margins::default(),
        };
        build(&spec).map(Some).map_err(|e| e.to_string())
    }

    /// Rebuilds the scene for the active worksheet at the given panel size.
    pub fn refresh(&mut self) {
        self.scene = None;
        self.error = None;

        let Some(worksheet) = self.active_worksheet() else {
            return;
        };
        let id = worksheet.id.clone();
        match self.scene_for(&id, self.scene_size) {
            Ok(Some(scene)) => self.scene = Some(scene),
            Ok(None) => {}
            Err(error) => self.error = Some(error),
        }
    }

    /// Saves the workbook and an extract of its data to a `.scz`.
    pub fn save(&mut self, path: &Path) -> Result<(), SessionError> {
        let mut scz = Scz::new(self.workbook.clone());
        for model in &self.workbook.models {
            for table in &model.tables {
                let tmp = temp_path(&table.name);
                let copy = format!(
                    "COPY (SELECT * FROM \"{}\") TO ? (FORMAT PARQUET)",
                    table.name
                );
                self.engine.execute(
                    &copy,
                    &[slickbi_calc::ParamValue::Text(
                        tmp.to_string_lossy().into_owned(),
                    )],
                )?;
                let bytes = std::fs::read(&tmp)?;
                scz = scz.with_extract(table.name.clone(), bytes);
                std::fs::remove_file(&tmp).ok();
            }
        }
        scz.save(path)?;
        Ok(())
    }

    /// Opens a `.scz`, registering its extracts and rebuilding the scene.
    pub fn open(&mut self, path: &Path) -> Result<(), SessionError> {
        let scz = Scz::open(path)?;
        let engine = Engine::open()?;

        for (table, bytes) in scz.extracts() {
            let tmp = temp_path(table);
            std::fs::write(&tmp, bytes)?;
            engine.register(table, &tmp, SourceFormat::Parquet)?;
            self.temp_files.push(tmp);
        }

        self.engine = engine;
        self.workbook = scz.into_workbook();
        self.active = 0;
        self.source = None;
        self.refresh();
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for path in &self.temp_files {
            std::fs::remove_file(path).ok();
        }
    }
}

fn new_worksheet(index: usize, model: DataModelId) -> Worksheet {
    Worksheet {
        id: WorksheetId::new(format!("ws_{index}")),
        name: format!("Sheet {index}"),
        model,
        mark: Mark::Bar,
        encoding: Default::default(),
        transforms: vec![],
        filters: vec![],
        scales: vec![],
        sort: vec![],
        limit: None,
        description: None,
    }
}

fn grid_frame(index: usize, canvas: slickbi_spec::Canvas) -> slickbi_spec::Rect {
    let columns = 2;
    let cell_width = canvas.width / columns as f64;
    let cell_height = 400.0;
    let col = index % columns;
    let row = index / columns;
    slickbi_spec::Rect {
        x: col as f64 * cell_width,
        y: row as f64 * cell_height,
        width: cell_width,
        height: cell_height,
    }
}

fn to_csv(batches: &[slickbi_engine::arrow::record_batch::RecordBatch]) -> String {
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

    let mut out = String::new();
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

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "slickbi-app-{label}-{}-{}.parquet",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

/// Anything that can go wrong while using the session.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The source could not be read or described.
    #[error("{0}")]
    Connector(#[from] slickbi_connectors::ConnectorError),
    /// The engine reported an error.
    #[error("{0}")]
    Engine(#[from] slickbi_engine::EngineError),
    /// The workbook could not be read or written.
    #[error("{0}")]
    Workbook(#[from] slickbi_workbook::WorkbookError),
    /// A model could not be resolved.
    #[error("{0}")]
    Model(#[from] slickbi_model::ModelError),
    /// A worksheet could not be planned.
    #[error("{0}")]
    Plan(#[from] slickbi_planner::PlanError),
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// There is no active worksheet to act on.
    #[error("no worksheet is active")]
    NoWorksheet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("slickbi-app-{label}-{n}-{}", std::process::id()))
    }

    fn csv(path: &Path) {
        std::fs::write(
            path,
            "region,amount,units\nEMEA,10.5,3\nAPAC,4.0,1\nEMEA,3.25,2\n",
        )
        .expect("writes csv");
    }

    fn loaded() -> (Session, PathBuf) {
        let path = temp("fixture.csv");
        csv(&path);
        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        (session, path)
    }

    #[test]
    fn loading_a_csv_populates_dimensions_and_measures() {
        let (session, path) = loaded();
        let (dimensions, measures) = session.field_groups();
        assert_eq!(dimensions.len(), 1);
        assert_eq!(measures.len(), 2);
        assert!(session.has_source());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn assigning_shelves_builds_a_grouped_scene() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        let scene = session.scene().expect("scene");
        assert_eq!(scene.series.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_filter_reduces_the_scene() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        let before = session.scene().expect("scene").clone();

        session.add_filter(
            &FieldId::new("region"),
            FilterOp::Equal,
            vec![Scalar::Text("APAC".to_owned())],
        );
        let after = session.scene().expect("scene");
        assert_ne!(before.series, after.series);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_calculated_field_is_added_and_usable() {
        let (mut session, path) = loaded();
        session
            .add_calculated_field("Double", "[amount] * 2")
            .expect("adds");
        let (_, measures) = session.field_groups();
        assert!(measures.iter().any(|(_, name)| name == "Double"));

        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("Double"));
        assert!(session.scene().is_some());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_invalid_calculation_is_rejected() {
        let (mut session, path) = loaded();
        let error = session
            .add_calculated_field("Bad", "[amount] + [nope]")
            .expect_err("rejects");
        assert!(error.to_string().contains("nope"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn reports_compose_worksheets() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        session.add_report("My report");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);
        session.add_text_to_report("# Title");

        let report = session.active_report_doc().expect("report");
        assert_eq!(report.elements.len(), 2);
        assert_eq!(report.elements[0].worksheet(), Some(&worksheet));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn exporting_csv_writes_the_result() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        let out = temp("export.csv");
        session.export_csv(&out).expect("exports");
        let text = std::fs::read_to_string(&out).expect("reads");
        assert!(text.contains("EMEA"));
        assert!(text.contains("APAC"));

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn saving_and_reopening_round_trips() {
        let path = temp("roundtrip.csv");
        csv(&path);
        let scz_path = temp("roundtrip.scz");

        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.save(&scz_path).expect("saves");

        let mut reopened = Session::new();
        reopened.open(&scz_path).expect("opens");
        assert_eq!(reopened.workbook().models.len(), 1);
        assert!(reopened.scene().is_some());

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&scz_path).ok();
    }
}
