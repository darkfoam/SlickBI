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
use slickbi_scene::{Margins, Scene, SceneOptions, SceneSpec, build};
use slickbi_spec::{
    Agg, Channel, Connection, ConnectionId, DataModelId, DataType, Field, FieldFormat, FieldId,
    FieldSource, Filter, FilterOp, Mark, ParamId, Parameter, Report, Role, Scalar, SortKey,
    Workbook, Worksheet, WorksheetId,
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
    /// The colour shelf: splits marks into a series per value.
    Color,
    /// The size shelf: scales point marks.
    Size,
    /// The text shelf: labels marks.
    Text,
    /// The detail shelf: splits marks without a visual property.
    Detail,
    /// The tooltip shelf: fields shown on hover.
    Tooltip,
}

/// A field as owned data for the data pane: its identifier, display name, folder and visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInfo {
    /// The field's identifier.
    pub id: FieldId,
    /// The name shown in the pane.
    pub name: String,
    /// The field's role, which decides the shelf a click places it on.
    pub role: Role,
    /// The folder it is grouped under, if any.
    pub folder: Option<String>,
    /// Whether the pane hides it until "Show hidden" is on.
    pub hidden: bool,
}

/// The smallest width a report element can be resized to, in points.
const MIN_ELEMENT_WIDTH: f64 = 80.0;
/// The smallest height a report element can be resized to, in points.
const MIN_ELEMENT_HEIGHT: f64 = 60.0;

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
        !self.workbook.models.is_empty()
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

    /// The fields of the active model, split into dimensions and measures, including hidden ones.
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
            let info = FieldInfo {
                id: field.id.clone(),
                name: field.name.clone(),
                role: field.role,
                folder: field.folder.clone(),
                hidden: field.hidden,
            };
            match field.role {
                Role::Measure => measures.push(info),
                Role::Dimension => dimensions.push(info),
            }
        }
        (dimensions, measures)
    }

    /// Loads a data file: discovers its schema, registers it, and adds a worksheet.
    pub fn load_file(&mut self, path: &Path, format: SourceFormat) -> Result<(), SessionError> {
        let model = slickbi_connectors::open_file(&self.engine, path, format)?;

        self.workbook.models.push(model);
        self.workbook
            .connections
            .push(slickbi_connectors::connection(path, format));

        let worksheet = new_worksheet(
            self.unique_worksheet_id(),
            format!("Sheet {}", self.workbook.worksheets.len() + 1),
            DataModelId::new("m1"),
        );
        self.workbook.worksheets.push(worksheet);
        self.active = self.workbook.worksheets.len() - 1;
        self.source = Some((path.to_owned(), format));
        self.refresh();
        Ok(())
    }

    /// Loads any supported source file, choosing the reader by its extension.
    ///
    /// CSV, TSV, Parquet and JSON stay live views over the file; a spreadsheet is read through its
    /// parser and imported as a typed table.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnsupportedSource`] for an extension this build does not read, or
    /// the connector error when the file cannot be read.
    pub fn load_path(&mut self, path: &Path) -> Result<(), SessionError> {
        if let Some(kind) = slickbi_connectors::SpreadsheetKind::from_path(path) {
            return self.load_spreadsheet(path, kind);
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            return Err(SessionError::UnsupportedSource(String::new()));
        };
        let format = match extension.to_ascii_lowercase().as_str() {
            "csv" | "tsv" | "tab" => SourceFormat::Csv,
            "parquet" | "pq" => SourceFormat::Parquet,
            "json" | "ndjson" | "jsonl" => SourceFormat::Json,
            other => return Err(SessionError::UnsupportedSource(other.to_owned())),
        };
        self.load_file(path, format)
    }

    /// Reads a spreadsheet's first sheet and adds it as a typed table and a worksheet.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Connector`] when the file cannot be read, or
    /// [`SessionError::Engine`] when the rows cannot be registered.
    pub fn load_spreadsheet(
        &mut self,
        path: &Path,
        kind: slickbi_connectors::SpreadsheetKind,
    ) -> Result<(), SessionError> {
        self.load_spreadsheet_sheet(path, kind, None)
    }

    /// Reads a named sheet (or the first when `sheet` is `None`) and adds it as a worksheet.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Connector`] when the file or sheet cannot be read, or
    /// [`SessionError::Engine`] when the rows cannot be registered.
    pub fn load_spreadsheet_sheet(
        &mut self,
        path: &Path,
        kind: slickbi_connectors::SpreadsheetKind,
        sheet: Option<&str>,
    ) -> Result<(), SessionError> {
        let table = slickbi_connectors::open_spreadsheet(&self.engine, path, kind, sheet)?;

        self.workbook.models.push(table.model);
        self.workbook.connections.push(Connection {
            id: ConnectionId::new("c1"),
            name: format!("{} · {}", kind.label(), table.sheet),
            kind: kind.connection_kind(),
            location: path.to_string_lossy().into_owned(),
            read_only: true,
            options: [("sheet".to_owned(), table.sheet.clone())].into(),
        });

        let worksheet = new_worksheet(
            self.unique_worksheet_id(),
            format!("Sheet {}", self.workbook.worksheets.len() + 1),
            DataModelId::new("m1"),
        );
        self.workbook.worksheets.push(worksheet);
        self.active = self.workbook.worksheets.len() - 1;
        self.source = None;
        self.refresh();
        Ok(())
    }

    /// Connects to a database table, imports it into the engine, and adds a worksheet.
    ///
    /// The password is used only for this connection and is never written to the workbook.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Connector`] when the database cannot be reached or read, or
    /// [`SessionError::Engine`] when the imported rows cannot be registered.
    pub fn connect_database(
        &mut self,
        kind: slickbi_connectors::DatabaseKind,
        location: &str,
        user: Option<&str>,
        password: Option<&str>,
        table: &str,
    ) -> Result<(), SessionError> {
        let source = database_source(kind, location, user, password, table);
        let imported = slickbi_connectors::read_database(&source)?;
        self.engine.register_rows(
            slickbi_connectors::TABLE_NAME,
            &imported.columns,
            &imported.rows,
        )?;

        self.workbook.models.push(imported.model);
        self.workbook.connections.push(Connection {
            id: ConnectionId::new("c1"),
            name: format!("{} · {table}", kind.label()),
            kind: kind.connection_kind(),
            location: location.to_owned(),
            read_only: true,
            options: [("table".to_owned(), table.to_owned())].into(),
        });

        let worksheet = new_worksheet(
            self.unique_worksheet_id(),
            format!("Sheet {}", self.workbook.worksheets.len() + 1),
            DataModelId::new("m1"),
        );
        self.workbook.worksheets.push(worksheet);
        self.active = self.workbook.worksheets.len() - 1;
        self.source = None;
        self.refresh();
        Ok(())
    }

    /// Lists the tables a database exposes, without importing any of them.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Connector`] when the database cannot be reached.
    pub fn database_tables(
        &self,
        kind: slickbi_connectors::DatabaseKind,
        location: &str,
        user: Option<&str>,
        password: Option<&str>,
    ) -> Result<Vec<String>, SessionError> {
        let source = database_source(kind, location, user, password, "");
        Ok(slickbi_connectors::list_tables(&source)?)
    }

    /// Lists the sheets or tables a spreadsheet exposes, without importing any of them.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Connector`] when the file cannot be read.
    pub fn spreadsheet_sheets(
        &self,
        path: &Path,
        kind: slickbi_connectors::SpreadsheetKind,
    ) -> Result<Vec<String>, SessionError> {
        Ok(slickbi_connectors::spreadsheet_sheets(path, kind)?)
    }

    /// Adds a new, empty worksheet and makes it active.
    pub fn add_worksheet(&mut self) {
        let id = self.unique_worksheet_id();
        let name = format!("Sheet {}", self.workbook.worksheets.len() + 1);
        let worksheet = new_worksheet(id, name, DataModelId::new("m1"));
        self.workbook.worksheets.push(worksheet);
        self.active = self.workbook.worksheets.len() - 1;
        self.refresh();
    }

    /// Makes the given worksheet active.
    pub fn set_active(&mut self, index: usize) {
        if index < self.workbook.worksheets.len() {
            self.active = index;
            self.refresh();
        }
    }

    /// Makes the worksheet with this id active. Returns whether it exists.
    pub fn activate_worksheet(&mut self, id: &WorksheetId) -> bool {
        match self
            .workbook
            .worksheets
            .iter()
            .position(|worksheet| &worksheet.id == id)
        {
            Some(index) => {
                self.set_active(index);
                true
            }
            None => false,
        }
    }

    /// Renames a worksheet. Blank names are ignored.
    pub fn rename_worksheet(&mut self, index: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(worksheet) = self.workbook.worksheets.get_mut(index) {
            worksheet.name = name.to_owned();
        }
    }

    /// Duplicates a worksheet (its fields, mark and filters), inserting the copy right after it.
    pub fn duplicate_worksheet(&mut self, index: usize) {
        let Some(original) = self.workbook.worksheets.get(index).cloned() else {
            return;
        };
        let id = self.unique_worksheet_id();
        let mut copy = original;
        copy.id = id;
        copy.name = format!("{} copy", copy.name);
        let insert_at = index + 1;
        self.workbook.worksheets.insert(insert_at, copy);
        self.active = insert_at;
        self.refresh();
    }

    /// Moves a worksheet earlier or later among the tabs, keeping it active.
    pub fn move_worksheet(&mut self, index: usize, delta: isize) {
        let last = self.workbook.worksheets.len().saturating_sub(1) as isize;
        let target = (index as isize + delta).clamp(0, last) as usize;
        if target == index || index >= self.workbook.worksheets.len() {
            return;
        }
        self.workbook.worksheets.swap(index, target);
        if self.active == index {
            self.active = target;
        } else if self.active == target {
            self.active = index;
        }
        self.refresh();
    }

    /// The reports that reference a worksheet, by name.
    pub fn used_in(&self, worksheet: &WorksheetId) -> Vec<String> {
        self.workbook
            .reports
            .iter()
            .filter(|report| report.referenced_worksheets().contains(&worksheet))
            .map(|report| report.name.clone())
            .collect()
    }

    fn unique_worksheet_id(&self) -> WorksheetId {
        let mut n = self.workbook.worksheets.len();
        loop {
            let candidate = WorksheetId::new(format!("ws_{n}"));
            if !self.workbook.worksheets.iter().any(|w| w.id == candidate) {
                return candidate;
            }
            n += 1;
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
            Shelf::Color => worksheet.encoding.color = Some(channel),
            Shelf::Size => worksheet.encoding.size = Some(channel),
            Shelf::Text => worksheet.encoding.text = Some(channel),
            Shelf::Detail => worksheet.encoding.detail = vec![channel],
            Shelf::Tooltip => worksheet.encoding.tooltip = vec![channel],
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
            Shelf::Color => worksheet.encoding.color = None,
            Shelf::Size => worksheet.encoding.size = None,
            Shelf::Text => worksheet.encoding.text = None,
            Shelf::Detail => worksheet.encoding.detail.clear(),
            Shelf::Tooltip => worksheet.encoding.tooltip.clear(),
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

    /// Changes the aggregation of a shelf on the active worksheet.
    pub fn set_agg(&mut self, shelf: Shelf, agg: Agg) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        let channel = match shelf {
            Shelf::X => worksheet.encoding.x.as_mut(),
            Shelf::Y => worksheet.encoding.y.as_mut(),
            Shelf::Color => worksheet.encoding.color.as_mut(),
            Shelf::Size => worksheet.encoding.size.as_mut(),
            Shelf::Text => worksheet.encoding.text.as_mut(),
            Shelf::Detail => worksheet.encoding.detail.first_mut(),
            Shelf::Tooltip => worksheet.encoding.tooltip.first_mut(),
        };
        if let Some(channel) = channel {
            channel.agg = agg;
        }
        self.refresh();
    }

    /// Sorts the active worksheet by the field on a shelf, keeping that shelf's aggregation.
    pub fn set_sort(&mut self, shelf: Shelf, descending: bool) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        let channel = match shelf {
            Shelf::X => worksheet.encoding.x.as_ref(),
            Shelf::Y => worksheet.encoding.y.as_ref(),
            Shelf::Color | Shelf::Size | Shelf::Text | Shelf::Detail | Shelf::Tooltip => return,
        };
        let Some(channel) = channel else {
            return;
        };
        worksheet.sort = vec![SortKey {
            field: channel.field.clone(),
            agg: channel.agg,
            descending,
        }];
        self.refresh();
    }

    /// Removes the explicit sort from the active worksheet.
    pub fn clear_sort(&mut self) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        worksheet.sort.clear();
        self.refresh();
    }

    /// Sets the row limit on the active worksheet.
    pub fn set_limit(&mut self, limit: Option<u32>) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        worksheet.limit = limit;
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
            parameter: None,
        });
        self.refresh();
    }

    /// Adds a filter bound to a parameter to the active worksheet and rebuilds.
    pub fn add_parameter_filter(&mut self, field: &FieldId, op: FilterOp, parameter: &ParamId) {
        let Some(worksheet) = self.workbook.worksheets.get_mut(self.active) else {
            return;
        };
        worksheet.filters.push(Filter {
            field: field.clone(),
            op,
            values: Vec::new(),
            parameter: Some(parameter.clone()),
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

        let resolved = Model::resolve_with(model, &self.workbook.parameters).map_err(|e| {
            slickbi_calc::CalcError::UnknownFunction {
                name: e.to_string(),
                span: slickbi_calc::Span::new(0, 0),
            }
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
            folder: None,
            description: None,
        };
        model.fields.push(field);
        self.refresh();
        Ok(())
    }

    /// The display format declared on a field, if any.
    pub fn field_format(&self, field: &FieldId) -> Option<FieldFormat> {
        let worksheet = self.active_worksheet()?;
        let model = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)?;
        model
            .fields
            .iter()
            .find(|candidate| &candidate.id == field)
            .and_then(|candidate| candidate.format.clone())
    }

    /// Sets or clears a field's display format, then re-renders.
    pub fn set_field_format(&mut self, field: &FieldId, format: Option<FieldFormat>) {
        let Some(model_id) = self
            .active_worksheet()
            .map(|worksheet| worksheet.model.clone())
        else {
            return;
        };
        if let Some(model) = self.workbook.models.iter_mut().find(|m| m.id == model_id)
            && let Some(candidate) = model.fields.iter_mut().find(|f| &f.id == field)
        {
            candidate.format = format;
        }
        self.refresh();
    }

    /// The declared type of a field.
    pub fn field_type(&self, field: &FieldId) -> Option<DataType> {
        let worksheet = self.active_worksheet()?;
        let model = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)?;
        model
            .fields
            .iter()
            .find(|candidate| &candidate.id == field)
            .map(|candidate| candidate.data_type)
    }

    /// Changes a field's type; the planner casts the column to it, then re-renders.
    pub fn set_field_type(&mut self, field: &FieldId, data_type: DataType) {
        let Some(model_id) = self
            .active_worksheet()
            .map(|worksheet| worksheet.model.clone())
        else {
            return;
        };
        if let Some(model) = self.workbook.models.iter_mut().find(|m| m.id == model_id)
            && let Some(candidate) = model.fields.iter_mut().find(|f| &f.id == field)
        {
            candidate.data_type = data_type;
        }
        self.refresh();
    }

    /// The folder a field is shown under, if any.
    pub fn field_folder(&self, field: &FieldId) -> Option<String> {
        self.active_field(field)
            .and_then(|field| field.folder.clone())
    }

    /// Whether a field is hidden from the data pane.
    pub fn field_hidden(&self, field: &FieldId) -> bool {
        self.active_field(field).is_some_and(|field| field.hidden)
    }

    /// The field in the active model, if it exists.
    fn active_field(&self, field: &FieldId) -> Option<&Field> {
        let worksheet = self.active_worksheet()?;
        let model = self
            .workbook
            .models
            .iter()
            .find(|model| model.id == worksheet.model)?;
        model.fields.iter().find(|candidate| &candidate.id == field)
    }

    /// The index of the active worksheet's model.
    fn active_model_index(&self) -> Option<usize> {
        let worksheet = self.active_worksheet()?;
        self.workbook
            .models
            .iter()
            .position(|model| model.id == worksheet.model)
    }

    /// Renames a field and rewrites every calculation that referenced its old name.
    ///
    /// Column fields only change their display name; a calculation's `[Name]` reference is text, so
    /// renaming a field it reads would otherwise leave the calculation pointing at nothing.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::EmptyFieldName`] for a blank name,
    /// [`SessionError::DuplicateFieldName`] when another field already uses it, and
    /// [`SessionError::UnknownField`] when the field is not in the active model.
    pub fn rename_field(&mut self, field: &FieldId, name: &str) -> Result<(), SessionError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SessionError::EmptyFieldName);
        }
        let Some(index) = self.active_model_index() else {
            return Err(SessionError::NoWorksheet);
        };
        let model = &mut self.workbook.models[index];
        let Some(position) = model
            .fields
            .iter()
            .position(|candidate| &candidate.id == field)
        else {
            return Err(SessionError::UnknownField(field.clone()));
        };
        if model
            .fields
            .iter()
            .any(|candidate| &candidate.id != field && candidate.name == name)
        {
            return Err(SessionError::DuplicateFieldName(name.to_owned()));
        }
        let old = model.fields[position].name.clone();
        if old == name {
            return Ok(());
        }
        model.fields[position].name = name.to_owned();
        for candidate in &mut model.fields {
            if let FieldSource::Expression { expression } = &mut candidate.source {
                *expression = rewrite_field_reference(expression, &old, name);
            }
        }
        self.refresh();
        Ok(())
    }

    /// Hides or shows a field in the data pane. Hidden fields still resolve in a worksheet.
    pub fn set_field_hidden(&mut self, field: &FieldId, hidden: bool) {
        if let Some(index) = self.active_model_index()
            && let Some(candidate) = self.workbook.models[index]
                .fields
                .iter_mut()
                .find(|candidate| &candidate.id == field)
        {
            candidate.hidden = hidden;
        }
        self.refresh();
    }

    /// Moves a field into a folder, or out of one when `folder` is `None` or empty.
    pub fn set_field_folder(&mut self, field: &FieldId, folder: Option<&str>) {
        let folder = folder
            .map(str::trim)
            .filter(|folder| !folder.is_empty())
            .map(str::to_owned);
        if let Some(index) = self.active_model_index()
            && let Some(candidate) = self.workbook.models[index]
                .fields
                .iter_mut()
                .find(|candidate| &candidate.id == field)
        {
            candidate.folder = folder;
        }
        self.refresh();
    }

    /// Moves a field one place up or down within its role and folder.
    pub fn move_field(&mut self, field: &FieldId, delta: isize) {
        let Some(index) = self.active_model_index() else {
            return;
        };
        let model = &mut self.workbook.models[index];
        let Some(position) = model
            .fields
            .iter()
            .position(|candidate| &candidate.id == field)
        else {
            return;
        };
        let role = model.fields[position].role;
        let folder = model.fields[position].folder.clone();
        let group: Vec<usize> = model
            .fields
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.role == role && candidate.folder == folder)
            .map(|(index, _)| index)
            .collect();
        let Some(at) = group.iter().position(|index| *index == position) else {
            return;
        };
        let target = at as isize + delta;
        if target < 0 || target >= group.len() as isize {
            return;
        }
        model.fields.swap(position, group[target as usize]);
        self.refresh();
    }

    /// The workbook's parameters.
    pub fn parameters(&self) -> &[Parameter] {
        &self.workbook.parameters
    }

    /// The name of a parameter, if it exists.
    pub fn parameter_name(&self, id: &ParamId) -> Option<&str> {
        self.workbook
            .parameters
            .iter()
            .find(|parameter| &parameter.id == id)
            .map(|parameter| parameter.name.as_str())
    }

    /// Adds a parameter and returns its identifier.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::EmptyParameterName`], [`SessionError::DuplicateParameterName`],
    /// [`SessionError::ParameterNameConflict`] or [`SessionError::ParameterValueType`].
    pub fn add_parameter(
        &mut self,
        name: &str,
        data_type: DataType,
        value: Scalar,
    ) -> Result<ParamId, SessionError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SessionError::EmptyParameterName);
        }
        self.check_parameter_name(name, None)?;
        let value = coerce_parameter_value(data_type, value)?;
        let id = self.unique_parameter_id();
        self.workbook.parameters.push(Parameter {
            id: id.clone(),
            name: name.to_owned(),
            data_type,
            value,
        });
        self.refresh();
        Ok(id)
    }

    /// Changes a parameter's name, type and value, rewriting calculation references when the name
    /// changes.
    ///
    /// # Errors
    ///
    /// Returns the name and value errors of [`Session::add_parameter`], and
    /// [`SessionError::UnknownParameter`] when the id is not in the workbook.
    pub fn update_parameter(
        &mut self,
        id: &ParamId,
        name: &str,
        data_type: DataType,
        value: Scalar,
    ) -> Result<(), SessionError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SessionError::EmptyParameterName);
        }
        self.check_parameter_name(name, Some(id))?;
        let value = coerce_parameter_value(data_type, value)?;
        let Some(parameter) = self
            .workbook
            .parameters
            .iter_mut()
            .find(|parameter| &parameter.id == id)
        else {
            return Err(SessionError::UnknownParameter(id.clone()));
        };
        let old = parameter.name.clone();
        parameter.name = name.to_owned();
        parameter.data_type = data_type;
        parameter.value = value;
        if old != name {
            for model in &mut self.workbook.models {
                for field in &mut model.fields {
                    if let FieldSource::Expression { expression } = &mut field.source {
                        *expression = rewrite_field_reference(expression, &old, name);
                    }
                }
            }
        }
        self.refresh();
        Ok(())
    }

    /// Changes a parameter's value and re-renders.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownParameter`] or [`SessionError::ParameterValueType`].
    pub fn set_parameter_value(&mut self, id: &ParamId, value: Scalar) -> Result<(), SessionError> {
        let Some(parameter) = self
            .workbook
            .parameters
            .iter_mut()
            .find(|parameter| &parameter.id == id)
        else {
            return Err(SessionError::UnknownParameter(id.clone()));
        };
        parameter.value = coerce_parameter_value(parameter.data_type, value)?;
        self.refresh();
        Ok(())
    }

    /// Removes a parameter that no calculation or filter uses.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ParameterInUse`] when a calculation references it by name or a
    /// filter is bound to it, and [`SessionError::UnknownParameter`] when it is absent.
    pub fn remove_parameter(&mut self, id: &ParamId) -> Result<(), SessionError> {
        let Some(parameter) = self
            .workbook
            .parameters
            .iter()
            .find(|parameter| &parameter.id == id)
            .cloned()
        else {
            return Err(SessionError::UnknownParameter(id.clone()));
        };
        let used_by_calculation = self.workbook.models.iter().any(|model| {
            model.fields.iter().any(|field| {
                matches!(&field.source, FieldSource::Expression { expression }
                    if expression_uses(expression, &parameter.name))
            })
        });
        let used_by_filter = self.workbook.worksheets.iter().any(|worksheet| {
            worksheet
                .filters
                .iter()
                .any(|filter| filter.parameter.as_ref() == Some(id))
        });
        if used_by_calculation || used_by_filter {
            return Err(SessionError::ParameterInUse(parameter.name));
        }
        self.workbook
            .parameters
            .retain(|parameter| &parameter.id != id);
        self.refresh();
        Ok(())
    }

    fn unique_parameter_id(&self) -> ParamId {
        let mut index = 1;
        loop {
            let id = ParamId::new(format!("param_{index}"));
            if !self
                .workbook
                .parameters
                .iter()
                .any(|parameter| parameter.id == id)
            {
                return id;
            }
            index += 1;
        }
    }

    fn check_parameter_name(
        &self,
        name: &str,
        except: Option<&ParamId>,
    ) -> Result<(), SessionError> {
        if self
            .workbook
            .parameters
            .iter()
            .any(|parameter| Some(&parameter.id) != except && parameter.name == name)
        {
            return Err(SessionError::DuplicateParameterName(name.to_owned()));
        }
        if self
            .workbook
            .models
            .iter()
            .any(|model| model.fields.iter().any(|field| field.name == name))
        {
            return Err(SessionError::ParameterNameConflict(name.to_owned()));
        }
        Ok(())
    }

    /// The description of a field, if any.
    pub fn field_description(&self, field: &FieldId) -> Option<String> {
        let worksheet = self.active_worksheet()?;
        let model = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)?;
        model
            .fields
            .iter()
            .find(|candidate| &candidate.id == field)
            .and_then(|candidate| candidate.description.clone())
    }

    /// Sets or clears a field's description. A blank description clears it.
    pub fn set_field_description(&mut self, field: &FieldId, description: Option<String>) {
        let Some(model_id) = self
            .active_worksheet()
            .map(|worksheet| worksheet.model.clone())
        else {
            return;
        };
        let description = description
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        if let Some(model) = self.workbook.models.iter_mut().find(|m| m.id == model_id)
            && let Some(candidate) = model.fields.iter_mut().find(|f| &f.id == field)
        {
            candidate.description = description;
        }
    }

    /// Adds a report and makes it active.
    pub fn add_report(&mut self, name: &str) {
        let id = self.unique_report_id();
        let report = Report {
            id,
            name: name.to_owned(),
            target: slickbi_spec::ReportTarget::Screen,
            theme: None,
            canvas: slickbi_spec::Canvas::standard(),
            elements: Vec::new(),
        };
        self.workbook.reports.push(report);
        self.active_report = self.workbook.reports.len() - 1;
    }

    /// Renames a report. Blank names are ignored.
    pub fn rename_report(&mut self, index: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(report) = self.workbook.reports.get_mut(index) {
            report.name = name.to_owned();
        }
    }

    /// Sets (or clears) the theme of a report.
    pub fn set_report_theme(&mut self, index: usize, theme: Option<String>) {
        if let Some(report) = self.workbook.reports.get_mut(index) {
            report.theme = theme;
        }
    }

    /// Makes the given report active.
    pub fn set_active_report(&mut self, index: usize) {
        if index < self.workbook.reports.len() {
            self.active_report = index;
        }
    }

    /// Removes an element from a report.
    pub fn remove_report_element(&mut self, report: usize, index: usize) {
        if let Some(report) = self.workbook.reports.get_mut(report)
            && index < report.elements.len()
        {
            report.elements.remove(index);
        }
    }

    /// Moves a report element by `(dx, dy)` points, keeping it on the canvas.
    pub fn move_element(&mut self, report: usize, index: usize, dx: f64, dy: f64) {
        let Some(report) = self.workbook.reports.get_mut(report) else {
            return;
        };
        let canvas = report.canvas;
        let Some(element) = report.elements.get_mut(index) else {
            return;
        };
        let max_x = (canvas.width - element.frame.width).max(0.0);
        let max_y = (canvas.height - element.frame.height).max(0.0);
        element.frame.x = (element.frame.x + dx).clamp(0.0, max_x);
        element.frame.y = (element.frame.y + dy).clamp(0.0, max_y);
    }

    /// Resizes a report element, keeping a usable minimum and staying on the canvas.
    pub fn resize_element(&mut self, report: usize, index: usize, width: f64, height: f64) {
        let Some(report) = self.workbook.reports.get_mut(report) else {
            return;
        };
        let canvas = report.canvas;
        let Some(element) = report.elements.get_mut(index) else {
            return;
        };
        let max_width = (canvas.width - element.frame.x).max(MIN_ELEMENT_WIDTH);
        let max_height = (canvas.height - element.frame.y).max(MIN_ELEMENT_HEIGHT);
        element.frame.width = width.clamp(MIN_ELEMENT_WIDTH, max_width);
        element.frame.height = height.clamp(MIN_ELEMENT_HEIGHT, max_height);
    }

    /// Moves an element earlier or later in draw order (and in the elements list).
    pub fn reorder_element(&mut self, report: usize, index: usize, delta: isize) {
        let Some(report) = self.workbook.reports.get_mut(report) else {
            return;
        };
        let last = report.elements.len().saturating_sub(1) as isize;
        let target = (index as isize + delta).clamp(0, last) as usize;
        if target != index && index < report.elements.len() {
            report.elements.swap(index, target);
        }
    }

    /// Replaces the markdown of a text element in a report.
    pub fn set_text_element(&mut self, report: usize, index: usize, markdown: &str) {
        if let Some(report) = self.workbook.reports.get_mut(report)
            && let Some(element) = report.elements.get_mut(index)
            && let slickbi_spec::ElementKind::Text { text } = &mut element.kind
        {
            text.markdown = markdown.to_owned();
        }
    }

    /// Sets a chart element's title override. A blank title clears the override.
    pub fn set_chart_title(&mut self, report: usize, index: usize, title: &str) {
        if let Some(report) = self.workbook.reports.get_mut(report)
            && let Some(element) = report.elements.get_mut(index)
            && let slickbi_spec::ElementKind::Chart { chart } = &mut element.kind
        {
            chart.title = if title.trim().is_empty() {
                None
            } else {
                Some(title.trim().to_owned())
            };
        }
    }

    /// The title override on a chart element, if any.
    pub fn chart_title(&self, report: usize, index: usize) -> Option<&str> {
        let report = self.workbook.reports.get(report)?;
        let element = report.elements.get(index)?;
        match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => chart.title.as_deref(),
            slickbi_spec::ElementKind::Text { .. } => None,
        }
    }

    /// The presentation overrides of a chart element.
    pub fn chart_overrides(&self, report: usize, index: usize) -> Option<slickbi_spec::Overrides> {
        let report = self.workbook.reports.get(report)?;
        let element = report.elements.get(index)?;
        match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => Some(chart.overrides.clone()),
            slickbi_spec::ElementKind::Text { .. } => None,
        }
    }

    /// Replaces a chart element's presentation overrides. Nothing here changes the numbers.
    pub fn set_chart_overrides(
        &mut self,
        report: usize,
        index: usize,
        overrides: slickbi_spec::Overrides,
    ) {
        if let Some(report) = self.workbook.reports.get_mut(report)
            && let Some(element) = report.elements.get_mut(index)
            && let slickbi_spec::ElementKind::Chart { chart } = &mut element.kind
        {
            chart.overrides = overrides;
        }
    }

    /// The report of a chart element, as `(worksheet, title)`.
    pub fn chart_element(
        &self,
        report: usize,
        index: usize,
    ) -> Option<(&WorksheetId, Option<&str>)> {
        let report = self.workbook.reports.get(report)?;
        let element = report.elements.get(index)?;
        match &element.kind {
            slickbi_spec::ElementKind::Chart { chart } => {
                Some((&chart.worksheet, chart.title.as_deref()))
            }
            slickbi_spec::ElementKind::Text { .. } => None,
        }
    }

    /// The markdown of a text element.
    pub fn text_element(&self, report: usize, index: usize) -> Option<&str> {
        let report = self.workbook.reports.get(report)?;
        let element = report.elements.get(index)?;
        match &element.kind {
            slickbi_spec::ElementKind::Text { text } => Some(&text.markdown),
            slickbi_spec::ElementKind::Chart { .. } => None,
        }
    }

    fn unique_report_id(&self) -> slickbi_spec::ReportId {
        let mut n = self.workbook.reports.len();
        loop {
            let candidate = slickbi_spec::ReportId::new(format!("r{n}"));
            if !self.workbook.reports.iter().any(|r| r.id == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn unique_element_id(report: &Report) -> slickbi_spec::ElementId {
        let mut n = report.elements.len();
        loop {
            let candidate = format!("e{n}");
            if !report.elements.iter().any(|e| e.id.as_str() == candidate) {
                return slickbi_spec::ElementId::new(candidate);
            }
            n += 1;
        }
    }

    /// Places a worksheet as a chart element in the active report, auto-laid-out on a grid.
    pub fn add_chart_to_report(&mut self, worksheet: &WorksheetId) {
        let Some(report) = self.workbook.reports.get_mut(self.active_report) else {
            return;
        };
        let frame = grid_frame(report.elements.len(), report.canvas);
        let id = Session::unique_element_id(report);
        let element = slickbi_spec::Element {
            id,
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
        let id = Session::unique_element_id(report);
        report.elements.push(slickbi_spec::Element {
            id,
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

    /// Writes the active source table to a Parquet file (a local extract).
    ///
    /// This is D14: the whole source, independent of any worksheet, as a portable Parquet file.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::NoWorksheet`] when no source is loaded, or an engine error if the
    /// table cannot be written.
    pub fn export_extract(&mut self, path: &Path) -> Result<(), SessionError> {
        if self.workbook.models.is_empty() {
            return Err(SessionError::NoWorksheet);
        }
        let sql = format!(
            "COPY (SELECT * FROM \"{}\") TO ? (FORMAT PARQUET)",
            slickbi_connectors::TABLE_NAME
        );
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
        let model = Model::resolve_with(document, &self.workbook.parameters)?;
        let query = plan(&model, &worksheet, &self.plan_options())?;
        let result = self.engine.execute(&query.sql, &query.params)?;
        Ok((query, result.batches))
    }

    /// The plan options that carry the workbook's current parameter values.
    fn plan_options(&self) -> PlanOptions {
        PlanOptions {
            parameters: self.workbook.parameters.clone(),
            ..PlanOptions::default()
        }
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

    /// Builds the scene for a worksheet at a given size with the default presentation.
    ///
    /// Returns `Ok(None)` when the worksheet has no shelves yet (nothing to draw), and `Err`
    /// when planning, execution or scene building fails.
    pub fn scene_for(
        &mut self,
        worksheet_id: &WorksheetId,
        size: (f32, f32),
    ) -> Result<Option<Scene>, String> {
        self.scene_for_with(worksheet_id, size, SceneOptions::default())
    }

    /// Builds the scene for a worksheet with presentation options, as a report element does.
    pub fn scene_for_with(
        &mut self,
        worksheet_id: &WorksheetId,
        size: (f32, f32),
        options: SceneOptions,
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
        // A KPI tile or a table needs no x or y shelf; every other mark does.
        if !matches!(worksheet.mark, Mark::Text | Mark::Table)
            && (worksheet.encoding.x.is_none() || worksheet.encoding.y.is_none())
        {
            return Ok(None);
        }

        let document = self
            .workbook
            .models
            .iter()
            .find(|m| m.id == worksheet.model)
            .ok_or_else(|| "the worksheet's model is missing".to_owned())?;
        let model =
            Model::resolve_with(document, &self.workbook.parameters).map_err(|e| e.to_string())?;
        let query = plan(&model, &worksheet, &self.plan_options()).map_err(|e| e.to_string())?;
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
            options,
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

/// Coerces a value to the parameter's declared type.
fn coerce_parameter_value(data_type: DataType, value: Scalar) -> Result<Scalar, SessionError> {
    let mismatch = || SessionError::ParameterValueType { data_type };
    match data_type {
        DataType::Integer => match value {
            Scalar::Integer(_) => Ok(value),
            Scalar::Number(number) => Ok(Scalar::Integer(number.round() as i64)),
            _ => Err(mismatch()),
        },
        DataType::Number => match value {
            Scalar::Number(_) => Ok(value),
            Scalar::Integer(number) => Ok(Scalar::Number(number as f64)),
            _ => Err(mismatch()),
        },
        DataType::Boolean => match value {
            Scalar::Boolean(_) => Ok(value),
            Scalar::Text(text) => match text.to_ascii_lowercase().as_str() {
                "true" | "t" | "yes" | "1" => Ok(Scalar::Boolean(true)),
                "false" | "f" | "no" | "0" => Ok(Scalar::Boolean(false)),
                _ => Err(mismatch()),
            },
            _ => Err(mismatch()),
        },
        DataType::Text | DataType::Date | DataType::Timestamp | DataType::Unknown => match value {
            Scalar::Text(_) => Ok(value),
            Scalar::Integer(number) => Ok(Scalar::Text(number.to_string())),
            Scalar::Number(number) => Ok(Scalar::Text(number.to_string())),
            Scalar::Boolean(boolean) => Ok(Scalar::Text(boolean.to_string())),
        },
    }
}

/// Whether a calculation reads `[name]`.
fn expression_uses(expression: &str, name: &str) -> bool {
    let characters: Vec<char> = expression.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        match characters[index] {
            '\'' => {
                index += 1;
                while index < characters.len() {
                    let character = characters[index];
                    index += 1;
                    if character == '\'' {
                        if index < characters.len() && characters[index] == '\'' {
                            index += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            '[' => {
                let mut field = String::new();
                index += 1;
                let mut closed = false;
                while index < characters.len() {
                    if characters[index] == ']' {
                        if index + 1 < characters.len() && characters[index + 1] == ']' {
                            field.push(']');
                            index += 2;
                        } else {
                            index += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        field.push(characters[index]);
                        index += 1;
                    }
                }
                if closed && field.trim() == name {
                    return true;
                }
            }
            _ => index += 1,
        }
    }
    false
}

/// Rewrites `[old]` field references in a calculation, leaving text literals and escaped `]]`
/// brackets intact.
fn rewrite_field_reference(expression: &str, old: &str, new: &str) -> String {
    let characters: Vec<char> = expression.chars().collect();
    let mut out = String::with_capacity(expression.len());
    let mut index = 0usize;
    while index < characters.len() {
        match characters[index] {
            '\'' => {
                out.push('\'');
                index += 1;
                while index < characters.len() {
                    let character = characters[index];
                    out.push(character);
                    index += 1;
                    if character == '\'' {
                        if index < characters.len() && characters[index] == '\'' {
                            out.push('\'');
                            index += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            '[' => {
                let mut name = String::new();
                index += 1;
                let mut closed = false;
                while index < characters.len() {
                    if characters[index] == ']' {
                        if index + 1 < characters.len() && characters[index + 1] == ']' {
                            name.push(']');
                            index += 2;
                        } else {
                            index += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        name.push(characters[index]);
                        index += 1;
                    }
                }
                out.push('[');
                if closed && name.trim() == old {
                    out.push_str(&new.replace(']', "]]"));
                } else {
                    out.push_str(&name.replace(']', "]]"));
                }
                out.push(']');
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    out
}

fn database_source(
    kind: slickbi_connectors::DatabaseKind,
    location: &str,
    user: Option<&str>,
    password: Option<&str>,
    table: &str,
) -> slickbi_connectors::DatabaseSource {
    slickbi_connectors::DatabaseSource {
        kind,
        location: location.to_owned(),
        user: user.map(str::to_owned).filter(|value| !value.is_empty()),
        password: password
            .map(str::to_owned)
            .filter(|value| !value.is_empty()),
        table: table.to_owned(),
        row_limit: 1_000_000,
    }
}

fn new_worksheet(id: WorksheetId, name: String, model: DataModelId) -> Worksheet {
    Worksheet {
        id,
        name,
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
    /// A field name was blank.
    #[error("a field name cannot be empty")]
    EmptyFieldName,
    /// Another field already uses this name.
    #[error("a field named '{0}' already exists")]
    DuplicateFieldName(String),
    /// The field is not in the active model.
    #[error("the field is not in the active model")]
    UnknownField(FieldId),
    /// The file extension is not a source this build reads.
    #[error("unsupported file type: {0}")]
    UnsupportedSource(String),
    /// A parameter name was blank.
    #[error("a parameter name cannot be empty")]
    EmptyParameterName,
    /// Another parameter already uses this name.
    #[error("a parameter named '{0}' already exists")]
    DuplicateParameterName(String),
    /// A field already uses this name.
    #[error("a field is already named '{0}'; use a different parameter name")]
    ParameterNameConflict(String),
    /// The value does not fit the parameter's declared type.
    #[error("the value does not fit a {data_type:?} parameter")]
    ParameterValueType {
        /// The declared type.
        data_type: DataType,
    },
    /// A calculation or filter still reads the parameter.
    #[error("parameter '{0}' is used by a calculation or filter; unbind it first")]
    ParameterInUse(String),
    /// The parameter is not in the workbook.
    #[error("the parameter is not in the workbook")]
    UnknownParameter(ParamId),
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
    fn renaming_a_field_rewrites_calculations_and_rejects_duplicates() {
        let (mut session, path) = loaded();
        session
            .add_calculated_field("Double", "[amount] * 2")
            .expect("adds");

        session
            .rename_field(&FieldId::new("amount"), "Sales")
            .expect("renames");
        let amount = session.workbook().models[0]
            .fields
            .iter()
            .find(|field| field.id == FieldId::new("amount"))
            .expect("amount");
        assert_eq!(amount.name, "Sales");
        let model = &session.workbook().models[0];
        let double = model
            .fields
            .iter()
            .find(|field| field.name == "Double")
            .expect("Double");
        match &double.source {
            FieldSource::Expression { expression } => {
                assert_eq!(expression, "[Sales] * 2", "the reference is rewritten");
            }
            other => panic!("a calculation: {other:?}"),
        }

        let error = session
            .rename_field(&FieldId::new("amount"), "region")
            .expect_err("a duplicate name is refused");
        assert!(matches!(error, SessionError::DuplicateFieldName(_)));
        let amount = session.workbook().models[0]
            .fields
            .iter()
            .find(|field| field.id == FieldId::new("amount"))
            .expect("amount");
        assert_eq!(amount.name, "Sales", "the refused rename changes nothing");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hiding_a_field_keeps_it_chartable() {
        let (mut session, path) = loaded();
        session.set_field_hidden(&FieldId::new("region"), true);
        let (dimensions, _) = session.field_groups();
        let region = dimensions
            .iter()
            .find(|field| field.id == FieldId::new("region"))
            .expect("the hidden field is still listed with its flag");
        assert!(region.hidden);
        assert!(session.field_hidden(&FieldId::new("region")));

        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        assert!(session.scene().is_some(), "a hidden field still resolves");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn fields_reorder_within_their_group_and_move_into_folders() {
        let (mut session, path) = loaded();
        let order = |session: &Session| -> Vec<String> {
            session
                .field_groups()
                .1
                .iter()
                .map(|field| field.name.clone())
                .collect()
        };
        assert_eq!(order(&session), vec!["amount", "units"]);
        session.move_field(&FieldId::new("units"), -1);
        assert_eq!(order(&session), vec!["units", "amount"]);
        session.move_field(&FieldId::new("units"), -1);
        assert_eq!(
            order(&session),
            vec!["units", "amount"],
            "the first stays first"
        );

        session.set_field_folder(&FieldId::new("amount"), Some("Money"));
        assert_eq!(
            session.field_folder(&FieldId::new("amount")).as_deref(),
            Some("Money")
        );
        session.set_field_folder(&FieldId::new("amount"), Some("   "));
        assert_eq!(session.field_folder(&FieldId::new("amount")), None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn names_folders_and_visibility_round_trip() {
        let (mut session, path) = loaded();
        session
            .rename_field(&FieldId::new("amount"), "Sales")
            .expect("renames");
        session.set_field_folder(&FieldId::new("amount"), Some("Money"));
        session.set_field_hidden(&FieldId::new("units"), true);

        let scz = temp("m02.scz");
        session.save(&scz).expect("saves");
        let mut reopened = Session::new();
        reopened.open(&scz).expect("opens");

        let (_, measures) = reopened.field_groups();
        let sales = measures
            .iter()
            .find(|field| field.id == FieldId::new("amount"))
            .expect("Sales");
        assert_eq!(sales.name, "Sales");
        assert_eq!(sales.folder.as_deref(), Some("Money"));
        let units = measures
            .iter()
            .find(|field| field.id == FieldId::new("units"))
            .expect("units");
        assert!(units.hidden);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&scz).ok();
    }

    #[test]
    fn a_parameter_drives_a_filter_and_a_calculation() {
        let (mut session, path) = loaded();
        let minimum = session
            .add_parameter("Minimum amount", DataType::Number, Scalar::Number(5.0))
            .expect("adds a parameter");
        session.add_parameter_filter(&FieldId::new("amount"), FilterOp::GreaterOrEqual, &minimum);
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        let bars = |session: &Session| -> usize {
            match &session
                .scene()
                .unwrap_or_else(|| panic!("scene: {:?}", session.error()))
                .series[0]
                .kind
            {
                slickbi_scene::SeriesKind::Bars(bars) => bars.len(),
                _ => 0,
            }
        };
        assert_eq!(bars(&session), 1, "only the EMEA row over 5 remains");
        session
            .set_parameter_value(&minimum, Scalar::Number(1.0))
            .expect("moves the parameter");
        assert_eq!(bars(&session), 2, "both regions now pass");

        let rate = session
            .add_parameter("Rate", DataType::Number, Scalar::Number(2.0))
            .expect("adds a second parameter");
        session
            .add_calculated_field("Scaled", "[amount] * [Rate]")
            .expect("a calculation reads the parameter");
        session.assign(Shelf::Y, &FieldId::new("Scaled"));
        assert!(
            session.scene().is_some(),
            "the calculation resolves: {:?}",
            session.error()
        );
        session
            .set_parameter_value(&rate, Scalar::Number(3.0))
            .expect("moves the calculation parameter");

        assert!(matches!(
            session.remove_parameter(&minimum),
            Err(SessionError::ParameterInUse(_))
        ));
        session.remove_filter(0);
        session
            .remove_parameter(&minimum)
            .expect("removes an unused parameter");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_reference_rewrite_leaves_text_and_other_fields_alone() {
        assert_eq!(
            rewrite_field_reference("[amount] * 2 + [amount2]", "amount", "Sales"),
            "[Sales] * 2 + [amount2]"
        );
        assert_eq!(
            rewrite_field_reference("'[amount]'", "amount", "Sales"),
            "'[amount]'",
            "text is not a reference"
        );
        assert_eq!(
            rewrite_field_reference("[a]]b] + 1", "a]b", "c]d"),
            "[c]]d] + 1",
            "escaped brackets survive"
        );
    }

    #[test]
    fn a_heatmap_is_built_from_two_dimensions_and_a_measure() {
        let path = temp("heat.csv");
        std::fs::write(
            &path,
            "row,col,value\nA,One,10\nB,One,20\nA,Two,30\nB,Two,40\n",
        )
        .expect("writes csv");
        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        session.assign(Shelf::X, &FieldId::new("row"));
        session.assign(Shelf::Y, &FieldId::new("col"));
        session.assign(Shelf::Color, &FieldId::new("value"));
        session.set_mark(Mark::Rect);

        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("scene: {:?}", session.error()));
        let slickbi_scene::SeriesKind::Heat(cells) = &scene.series[0].kind else {
            panic!("heat cells");
        };
        assert_eq!(cells.len(), 4);
        assert_eq!(scene.x_axis.ticks.len(), 2);
        assert_eq!(scene.y_axis.ticks.len(), 2);
        assert!(scene.x_axis.categorical && scene.y_axis.categorical);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_date_column_on_a_line_builds_a_continuous_axis() {
        let path = temp("balance.csv");
        std::fs::write(
            &path,
            "day,balance\n\
             2025-01-01,2000\n\
             2025-01-08,1800\n\
             2025-01-15,1400\n\
             2025-01-22,1000\n\
             2025-01-29,500\n",
        )
        .expect("writes csv");
        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        session.assign(Shelf::X, &FieldId::new("day"));
        session.assign(Shelf::Y, &FieldId::new("balance"));
        session.set_mark(Mark::Line);

        let scene = session.scene().expect("scene builds");
        assert!(!scene.x_axis.categorical, "the time axis is continuous");
        let ticks: Vec<&str> = scene.x_axis.ticks.iter().map(|t| t.text.as_str()).collect();
        assert!(
            ticks.iter().any(|text| text.starts_with("2025-01")),
            "date ticks: {ticks:?}"
        );
        assert!(matches!(
            scene.series[0].kind,
            slickbi_scene::SeriesKind::Line(_)
        ));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn every_fixture_format_charts_a_balance_over_time() {
        for name in [
            "transactions.csv",
            "transactions.tsv",
            "transactions.xlsx",
            "transactions.ods",
            "transactions.numbers",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../connectors/tests/fixtures")
                .join(name);
            let mut session = Session::new();
            session
                .load_path(&path)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            session.assign(Shelf::X, &FieldId::new("Day"));
            session.assign(Shelf::Y, &FieldId::new("Balance"));
            session.set_mark(Mark::Line);

            let scene = session
                .scene()
                .unwrap_or_else(|| panic!("{name}: {:?}", session.error()));
            assert!(!scene.x_axis.categorical, "{name}: a continuous time axis");
            let ticks: Vec<&str> = scene
                .x_axis
                .ticks
                .iter()
                .map(|tick| tick.text.as_str())
                .collect();
            assert!(
                ticks.iter().any(|text| text.starts_with("2025-01")),
                "{name}: date ticks {ticks:?}"
            );
            assert!(
                matches!(scene.series[0].kind, slickbi_scene::SeriesKind::Line(_)),
                "{name}: a line series"
            );
        }
    }

    #[test]
    fn connecting_to_sqlite_adds_a_queryable_worksheet() {
        let path = temp("connect.sqlite");
        {
            let connection = rusqlite::Connection::open(&path).expect("opens");
            connection
                .execute_batch(
                    "CREATE TABLE sales (region TEXT, amount INTEGER);
                     INSERT INTO sales VALUES ('EMEA', 10), ('APAC', 5), ('EMEA', 2);",
                )
                .expect("seeds");
        }

        let mut session = Session::new();
        session
            .connect_database(
                slickbi_connectors::DatabaseKind::Sqlite,
                &path.to_string_lossy(),
                None,
                None,
                "sales",
            )
            .expect("connects");

        let (dimensions, measures) = session.field_groups();
        assert!(
            dimensions.iter().any(|field| field.name == "region"),
            "dimensions"
        );
        assert!(
            measures.iter().any(|field| field.name == "amount"),
            "measures"
        );
        assert!(session.has_source());

        // The imported table is a real DuckDB table, so a chart plans and queries end to end.
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        assert!(
            session.scene().is_some(),
            "scene builds: {:?}",
            session.error()
        );

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
    fn a_pie_mark_builds_arc_series() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.set_mark(Mark::Arc);
        let scene = session.scene().expect("scene");
        assert!(!scene.series.is_empty());
        for series in &scene.series {
            assert!(matches!(&series.kind, slickbi_scene::SeriesKind::Arc(_)));
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_horizontal_bar_runs_through_the_planner() {
        let (mut session, path) = loaded();
        // A dimension on rows and a measure on columns gives horizontal bars.
        session.assign(Shelf::Y, &FieldId::new("region"));
        session.assign(Shelf::X, &FieldId::new("amount"));
        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        assert!(scene.y_axis.categorical, "categories belong on the y axis");
        assert!(!scene.x_axis.categorical, "values belong on the x axis");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_field_number_format_reaches_the_chart() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.set_field_format(
            &FieldId::new("amount"),
            Some(slickbi_spec::FieldFormat::Number {
                decimals: 1,
                thousands: true,
                currency: Some("$".to_owned()),
            }),
        );

        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        let texts: Vec<&str> = scene.y_axis.ticks.iter().map(|t| t.text.as_str()).collect();
        assert!(
            texts.iter().any(|text| text.starts_with('$')),
            "the currency format reached the axis: {texts:?}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_report_element_can_jump_to_its_worksheet() {
        let (mut session, path) = loaded();
        session.add_worksheet();
        let first = session.workbook().worksheets[0].id.clone();

        assert!(session.activate_worksheet(&first));
        assert_eq!(session.active(), 0);
        assert!(!session.activate_worksheet(&WorksheetId::new("missing")));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_field_description_round_trips() {
        let (mut session, path) = loaded();
        let amount = FieldId::new("amount");
        session.set_field_description(&amount, Some("  Revenue in USD  ".to_owned()));
        assert_eq!(
            session.field_description(&amount).as_deref(),
            Some("Revenue in USD")
        );
        session.set_field_description(&amount, Some("   ".to_owned()));
        assert_eq!(session.field_description(&amount), None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_color_shelf_splits_bars_into_series() {
        let path = temp("color.csv");
        std::fs::write(
            &path,
            "region,segment,amount\nEMEA,Retail,10\nEMEA,Wholesale,20\nAPAC,Retail,30\n",
        )
        .expect("writes csv");
        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");

        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.assign(Shelf::Color, &FieldId::new("segment"));

        let scene = session.scene().expect("scene");
        assert_eq!(scene.series.len(), 2, "one series per segment");

        // Clearing the colour shelf collapses back to a single series.
        session.clear(Shelf::Color);
        assert_eq!(session.scene().expect("scene").series.len(), 1);
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
        assert!(measures.iter().any(|field| field.name == "Double"));

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

    #[test]
    fn a_report_outlives_a_deleted_worksheet_through_save_and_reopen() {
        let path = temp("outlive.csv");
        csv(&path);
        let scz_path = temp("outlive.scz");

        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        session.add_report("Board");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);

        // Delete the worksheet the report uses; the element stays for a placeholder.
        let index = session
            .workbook()
            .worksheets
            .iter()
            .position(|w| w.id == worksheet)
            .expect("worksheet");
        session.remove_worksheet(index);
        assert_eq!(session.reports()[0].elements.len(), 1);

        // Saving and reopening must not reject the dangling reference (T05).
        session.save(&scz_path).expect("saves");
        let mut reopened = Session::new();
        reopened
            .open(&scz_path)
            .expect("a dangling reference must not block reopen");
        assert_eq!(reopened.reports()[0].elements.len(), 1);
        // The element resolves to a placeholder (an error to draw), not a lost report.
        assert!(reopened.scene_for(&worksheet, (200.0, 150.0)).is_err());

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&scz_path).ok();
    }

    fn agg_fixture() -> (Session, PathBuf) {
        let path = temp("agg.csv");
        std::fs::write(&path, "region,amount\nEMEA,10\nEMEA,20\nAPAC,30\n").expect("writes csv");
        let mut session = Session::new();
        session.load_file(&path, SourceFormat::Csv).expect("loads");
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        (session, path)
    }

    fn bar_heights(session: &Session) -> Vec<f32> {
        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        match &scene.series[0].kind {
            slickbi_scene::SeriesKind::Bars(bars) => bars.iter().map(|bar| bar.height).collect(),
            other => panic!("expected bars, got {other:?}"),
        }
    }

    #[test]
    fn renaming_a_worksheet_updates_its_tab() {
        let (mut session, path) = loaded();
        session.rename_worksheet(0, "  Revenue by region  ");
        assert_eq!(session.workbook().worksheets[0].name, "Revenue by region");
        session.rename_worksheet(0, "   ");
        assert_eq!(session.workbook().worksheets[0].name, "Revenue by region");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicating_a_worksheet_copies_its_encoding_under_a_new_id() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        let original_id = session.workbook().worksheets[0].id.clone();

        session.duplicate_worksheet(0);
        let worksheets = &session.workbook().worksheets;
        assert_eq!(worksheets.len(), 2);
        assert_eq!(worksheets[1].name, "Sheet 1 copy");
        assert_ne!(worksheets[1].id, original_id);
        assert_eq!(worksheets[1].encoding.x, worksheets[0].encoding.x);
        assert_eq!(worksheets[1].encoding.y, worksheets[0].encoding.y);
        assert_eq!(session.active(), 1);
        assert!(session.scene().is_some());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn worksheets_can_be_reordered() {
        let (mut session, path) = loaded();
        session.add_worksheet();
        let first = session.workbook().worksheets[0].id.clone();
        let second = session.workbook().worksheets[1].id.clone();

        session.set_active(0);
        session.move_worksheet(0, 1);
        assert_eq!(session.workbook().worksheets[0].id, second);
        assert_eq!(session.workbook().worksheets[1].id, first);
        assert_eq!(session.active(), 1, "the moved worksheet stays active");

        // Moving by zero, or past either end, is a no-op.
        session.move_worksheet(0, 0);
        assert_eq!(session.workbook().worksheets[0].id, second);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn used_in_lists_reports_that_reference_a_worksheet() {
        let (mut session, path) = loaded();
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        assert!(session.used_in(&worksheet).is_empty());

        session.add_report("Quarterly");
        session.add_chart_to_report(&worksheet);
        session.add_report("Annual");
        session.add_chart_to_report(&worksheet);

        assert_eq!(session.used_in(&worksheet), vec!["Quarterly", "Annual"]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn changing_the_aggregation_changes_the_values() {
        let (mut session, path) = agg_fixture();
        // Sum: EMEA 10 + 20 = 30, APAC 30, so both bars are the full height.
        let sum = bar_heights(&session);
        assert!((sum[0] - sum[1]).abs() < 0.5, "sum bars should be equal");

        // Average: EMEA (10 + 20) / 2 = 15, APAC 30, so one bar is twice the other, whichever
        // order the engine returns the groups in.
        session.set_agg(Shelf::Y, Agg::Average);
        let average = bar_heights(&session);
        let (low, high) = if average[0] <= average[1] {
            (average[0], average[1])
        } else {
            (average[1], average[0])
        };
        assert!(
            (high - low * 2.0).abs() < 2.0,
            "average bars should be in a 2:1 ratio: {:?}",
            average
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sorting_orders_the_result_and_limit_caps_it() {
        let (mut session, path) = agg_fixture();
        session.set_sort(Shelf::Y, true);
        let descending = bar_heights(&session);
        assert!(
            descending[0] >= descending[1],
            "descending sort should put the tallest bar first: {:?}",
            descending
        );

        session.set_limit(Some(1));
        assert_eq!(bar_heights(&session).len(), 1);

        session.clear_sort();
        session.set_limit(None);
        assert_eq!(bar_heights(&session).len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn report_metadata_and_elements_can_be_edited() {
        let (mut session, path) = loaded();
        session.add_report("Draft");
        session.rename_report(0, "Board pack");
        session.set_report_theme(0, Some("slate".to_owned()));
        assert_eq!(session.reports()[0].name, "Board pack");
        assert_eq!(session.reports()[0].theme.as_deref(), Some("slate"));

        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);
        session.add_text_to_report("# Notes");
        assert_eq!(session.reports()[0].elements.len(), 2);

        // A chart title override and the element accessors.
        assert_eq!(session.chart_title(0, 0), None);
        session.set_chart_title(0, 0, "  Revenue  ");
        assert_eq!(session.chart_title(0, 0), Some("Revenue"));
        assert_eq!(
            session.chart_element(0, 0).map(|(id, _)| id),
            Some(&worksheet)
        );
        session.set_chart_title(0, 0, "   ");
        assert_eq!(session.chart_title(0, 0), None);

        // A text element is editable in place.
        assert_eq!(session.text_element(0, 1), Some("# Notes"));
        session.set_text_element(0, 1, "Updated");
        assert_eq!(session.text_element(0, 1), Some("Updated"));

        session.remove_report_element(0, 0);
        assert_eq!(session.reports()[0].elements.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn report_elements_move_and_stay_on_the_canvas() {
        let (mut session, path) = loaded();
        session.add_report("Layout");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);
        let canvas = session.reports()[0].canvas;

        session.move_element(0, 0, 100.0, 50.0);
        let frame = session.reports()[0].elements[0].frame;
        assert!(frame.x > 0.0 && frame.y > 0.0);

        // Pushing far past the right edge clamps so the frame stays inside.
        session.move_element(0, 0, 100_000.0, 0.0);
        let frame = session.reports()[0].elements[0].frame;
        assert!((frame.x + frame.width - canvas.width).abs() < 1e-6);

        // Pushing past the top-left clamps at the origin.
        session.move_element(0, 0, -100_000.0, -100_000.0);
        let frame = session.reports()[0].elements[0].frame;
        assert_eq!((frame.x, frame.y), (0.0, 0.0));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn report_elements_resize_with_a_minimum_and_clamp_to_the_canvas() {
        let (mut session, path) = loaded();
        session.add_report("Layout");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);
        let canvas = session.reports()[0].canvas;

        // Shrinking below the minimum keeps the minimum.
        session.resize_element(0, 0, 1.0, 1.0);
        let frame = session.reports()[0].elements[0].frame;
        assert!(frame.width >= 80.0 - 1e-6);
        assert!(frame.height >= 60.0 - 1e-6);

        // Growing past the canvas clamps to the remaining space.
        session.resize_element(0, 0, 100_000.0, 100_000.0);
        let frame = session.reports()[0].elements[0].frame;
        assert!((frame.x + frame.width - canvas.width).abs() < 1e-6);
        assert!((frame.y + frame.height - canvas.height).abs() < 1e-6);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn report_elements_reorder() {
        let (mut session, path) = loaded();
        session.add_report("Layout");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);
        session.add_text_to_report("# Notes");
        let first = session.reports()[0].elements[0].id.clone();
        let second = session.reports()[0].elements[1].id.clone();

        session.reorder_element(0, 0, 1);
        assert_eq!(session.reports()[0].elements[0].id, second);
        assert_eq!(session.reports()[0].elements[1].id, first);

        // Reordering past the ends is a no-op, not an error.
        session.reorder_element(0, 0, -5);
        assert_eq!(session.reports()[0].elements[0].id, second);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn chart_presentation_overrides_round_trip() {
        let (mut session, path) = loaded();
        session.add_report("Styled");
        let worksheet = session.active_worksheet().expect("ws").id.clone();
        session.add_chart_to_report(&worksheet);

        assert_eq!(
            session.chart_overrides(0, 0),
            Some(slickbi_spec::Overrides::default())
        );

        let overrides = slickbi_spec::Overrides {
            show_title: Some(false),
            show_legend: Some(false),
            show_axes: Some(false),
            padding: Some(8.0),
            border: Some(true),
            background: None,
        };
        session.set_chart_overrides(0, 0, overrides.clone());
        assert_eq!(session.chart_overrides(0, 0), Some(overrides));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn exporting_parquet_writes_the_result() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));

        let out = temp("export.parquet");
        session.export_parquet(&out).expect("exports parquet");
        let metadata = std::fs::metadata(&out).expect("file exists");
        assert!(metadata.len() > 0);
        let magic = std::fs::read(&out).expect("reads");
        assert_eq!(&magic[..4], b"PAR1");
        assert_eq!(&magic[magic.len() - 4..], b"PAR1");

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn exporting_an_extract_writes_the_source_table() {
        let (mut session, path) = loaded();
        let out = temp("extract.parquet");
        session.export_extract(&out).expect("writes an extract");
        let bytes = std::fs::read(&out).expect("reads");
        assert_eq!(&bytes[..4], b"PAR1", "a parquet extract");
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn a_kpi_worksheet_builds_a_text_tile() {
        let (mut session, path) = loaded();
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.set_mark(Mark::Text);
        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        assert!(matches!(
            &scene.series[0].kind,
            slickbi_scene::SeriesKind::Text(_)
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_tooltip_shelf_reaches_the_scene() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("units"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.assign(Shelf::Tooltip, &FieldId::new("region"));
        session.set_mark(Mark::Point);

        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        assert!(!scene.tooltips.is_empty(), "tooltips are built");
        assert!(
            scene
                .tooltips
                .iter()
                .any(|mark| mark.text.contains("region")),
            "tooltip text: {:?}",
            scene.tooltips
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_table_worksheet_builds() {
        let (mut session, path) = loaded();
        session.assign(Shelf::X, &FieldId::new("region"));
        session.assign(Shelf::Y, &FieldId::new("amount"));
        session.set_mark(Mark::Table);

        let scene = session
            .scene()
            .unwrap_or_else(|| panic!("no scene: {:?}", session.error()));
        let table = scene.table.as_ref().expect("a table");
        assert_eq!(table.headers.len(), 2);
        assert!(!table.rows.is_empty(), "the table has rows");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_field_type_can_be_overridden() {
        let (mut session, path) = loaded();
        let amount = FieldId::new("amount");
        assert_eq!(session.field_type(&amount), Some(DataType::Number));
        session.set_field_type(&amount, DataType::Text);
        assert_eq!(session.field_type(&amount), Some(DataType::Text));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn worksheet_ids_stay_unique_after_removal() {
        let (mut session, path) = loaded();
        session.add_worksheet();
        session.add_worksheet();
        let before: Vec<_> = session
            .workbook()
            .worksheets
            .iter()
            .map(|w| w.id.clone())
            .collect();
        session.remove_worksheet(1);
        session.add_worksheet();
        let after: Vec<_> = session
            .workbook()
            .worksheets
            .iter()
            .map(|w| w.id.clone())
            .collect();
        let unique: std::collections::BTreeSet<_> = after.iter().collect();
        assert_eq!(unique.len(), after.len(), "worksheet ids must be unique");
        assert_ne!(before, after);
        std::fs::remove_file(&path).ok();
    }
}
