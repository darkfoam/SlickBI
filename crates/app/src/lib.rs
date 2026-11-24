#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The application state and its logic, separated from the windowing layer so it can be tested
//! without a display.
//!
//! A [`Session`] owns the embedded engine, the workbook document, and the cached scene for the
//! active worksheet. The windowing layer mutates the session (load a file, assign a field, change
//! the mark) and reads the cached scene to draw it.

use std::path::{Path, PathBuf};

use slickbi_engine::{Engine, SourceFormat};
use slickbi_model::Model;
use slickbi_planner::{PlanOptions, plan};
use slickbi_scene::{Margins, Scene, SceneSpec, build};
use slickbi_spec::{Agg, Channel, DataModelId, FieldId, Mark, Workbook, Worksheet, WorksheetId};
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

    /// The index of the active worksheet.
    pub fn active(&self) -> usize {
        self.active
    }

    /// The active worksheet.
    pub fn active_worksheet(&self) -> Option<&Worksheet> {
        self.workbook.worksheets.get(self.active)
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
                slickbi_spec::Role::Measure => measures.push(info),
                slickbi_spec::Role::Dimension => dimensions.push(info),
            }
        }
        (dimensions, measures)
    }

    /// Loads a data file: discovers its schema, registers it, and adds a worksheet.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] if the file cannot be read or described.
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

    /// Assigns a field to a shelf on the active worksheet.
    ///
    /// A measure defaults to [`Agg::Sum`], so dragging revenue onto a shelf produces a grouped
    /// total rather than raw rows; a dimension is used as-is.
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
            .is_some_and(|field| field.role == slickbi_spec::Role::Measure);
        if is_measure { Agg::Sum } else { Agg::None }
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

    /// Rebuilds the scene for the active worksheet at the given panel size.
    pub fn refresh(&mut self) {
        self.scene = None;
        self.error = None;

        let Some(worksheet) = self.workbook.worksheets.get(self.active) else {
            return;
        };

        // A chart needs both shelves; before that there is nothing to draw.
        if worksheet.encoding.x.is_none() || worksheet.encoding.y.is_none() {
            return;
        }

        let Some(document) = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)
        else {
            self.error = Some("the worksheet's model is missing".to_owned());
            return;
        };
        let model = match Model::resolve(document) {
            Ok(model) => model,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };

        let query = match plan(&model, worksheet, &PlanOptions::default()) {
            Ok(query) => query,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let result = match self.engine.execute(&query.sql, &query.params) {
            Ok(result) => result,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };

        let spec = SceneSpec {
            mark: worksheet.mark,
            columns: &query.columns,
            batches: &result.batches,
            size: self.scene_size,
            margins: Margins::default(),
        };
        match build(&spec) {
            Ok(scene) => self.scene = Some(scene),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Saves the workbook and an extract of its data to a `.scz`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] if the extract cannot be produced or the archive written.
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
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] if the archive is malformed or its data cannot be read.
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
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
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

    #[test]
    fn loading_a_csv_populates_dimensions_and_measures() {
        let path = temp("load.csv");
        csv(&path);

        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");

        let (dimensions, measures) = session.field_groups();
        assert_eq!(dimensions.len(), 1); // region
        assert_eq!(measures.len(), 2); // amount, units
        assert!(session.has_source());
        assert_eq!(
            session.active_worksheet().expect("worksheet").name,
            "Sheet 0"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn assigning_shelves_builds_a_scene() {
        let path = temp("shelves.csv");
        csv(&path);

        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");

        // No shelves yet: no scene.
        assert!(session.scene().is_none());

        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        let scene = session.scene().expect("scene built");
        assert_eq!(scene.series.len(), 1);

        std::fs::remove_file(&path).ok();
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
        assert_eq!(reopened.workbook().worksheets.len(), 1);
        assert!(reopened.scene().is_some(), "reopen rebuilds the scene");

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&scz_path).ok();
    }
}
