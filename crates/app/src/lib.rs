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
    FieldSource, Filter, FilterOp, Mark, Report, Role, Scalar, SortKey, Workbook, Worksheet,
    WorksheetId,
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

/// A field as owned data for the data pane: its identifier and display name.
pub type FieldInfo = (FieldId, String);

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
            dimensions.iter().any(|(_, name)| name == "region"),
            "dimensions"
        );
        assert!(
            measures.iter().any(|(_, name)| name == "amount"),
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
