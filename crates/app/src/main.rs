//! The SlickBI desktop window.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use slickbi_app::{Session, Shelf};
use slickbi_engine::SourceFormat;
use slickbi_scene::{Bar, Scene, SeriesKind};
use slickbi_spec::{Agg, Channel, FieldId, FilterOp, Mark, Role, Scalar};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 860.0])
            .with_min_inner_size([820.0, 560.0])
            .with_title("SlickBI"),
        ..Default::default()
    };
    eframe::run_native(
        "SlickBI",
        options,
        Box::new(|cc| Ok(Box::new(SlickBiApp::new(cc)))),
    )
}

/// The payload carried when dragging a field from the data pane onto a shelf.
struct DragField {
    field: FieldId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Charts,
    Reports,
}

/// An action requested by the tab strip, applied after the strip is drawn.
enum TabAction {
    Activate(usize),
    StartRename(usize, String),
    CommitRename(usize, String),
    CancelRename,
    Duplicate(usize),
    Close(usize),
    Move(usize, isize),
    Add,
}

/// An action requested by the report element list, applied after the list is drawn.
enum ElementAction {
    Remove(usize),
    Reorder(usize, isize),
    Move(usize, f64, f64),
    Resize(usize, f64, f64),
}

struct SlickBiApp {
    session: Session,
    mode: Mode,
    path_input: String,

    calc_open: bool,
    calc_name: String,
    calc_expr: String,
    calc_error: Option<String>,

    filter_open: bool,
    filter_field: Option<FieldId>,
    filter_op: FilterOp,
    filter_value: String,

    renaming: Option<(usize, String)>,
    pending_delete: Option<(usize, Vec<String>)>,

    format_field: Option<FieldId>,
    format_decimals: u8,
    format_thousands: bool,
    format_currency: String,
}

impl SlickBiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        install_style(&cc.egui_ctx);

        SlickBiApp {
            session: Session::new(),
            mode: Mode::Charts,
            path_input: String::new(),
            calc_open: false,
            calc_name: String::new(),
            calc_expr: String::new(),
            calc_error: None,
            filter_open: false,
            filter_field: None,
            filter_op: FilterOp::Equal,
            filter_value: String::new(),
            renaming: None,
            pending_delete: None,
            format_field: None,
            format_decimals: 0,
            format_thousands: false,
            format_currency: String::new(),
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        let format = match path.extension().and_then(|e| e.to_str()) {
            Some("csv") => SourceFormat::Csv,
            Some("parquet") | Some("pq") => SourceFormat::Parquet,
            Some("json") | Some("ndjson") | Some("jsonl") => SourceFormat::Json,
            _ => {
                eprintln!("unsupported file type: {path:?}");
                return;
            }
        };
        if let Err(error) = self.session.load_file(&path, format) {
            eprintln!("failed to load {path:?}: {error}");
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("SlickBI").heading().strong());
            ui.add_space(8.0);

            if ui.button("Open…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                self.open_path(path);
            }
            if ui.button("Save").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("SlickBI workbook", &["scz"])
                    .save_file()
                && let Err(error) = self.session.save(&path)
            {
                eprintln!("failed to save: {error}");
            }

            ui.add_enabled_ui(self.session.has_source(), |ui| {
                ui.menu_button("Export ▾", |ui| {
                    if ui.button("CSV…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("CSV", &["csv"])
                            .save_file()
                            && let Err(error) = self.session.export_csv(&path)
                        {
                            eprintln!("failed to export CSV: {error}");
                        }
                        ui.close();
                    }
                    if ui.button("Parquet…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Parquet", &["parquet"])
                            .save_file()
                            && let Err(error) = self.session.export_parquet(&path)
                        {
                            eprintln!("failed to export Parquet: {error}");
                        }
                        ui.close();
                    }
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if ui
                    .selectable_label(self.mode == Mode::Reports, "Reports")
                    .clicked()
                {
                    self.mode = Mode::Reports;
                }
                if ui
                    .selectable_label(self.mode == Mode::Charts, "Charts")
                    .clicked()
                {
                    self.mode = Mode::Charts;
                }
            });
        });
    }

    fn charts_ui(&mut self, ui: &mut egui::Ui) {
        self.worksheet_tabs(ui);
        egui::Panel::left("data")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.data_pane(ui);
                    self.shelves(ui);
                    self.filters(ui);
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.chart_panel(ui);
        });
    }

    fn worksheet_tabs(&mut self, ui: &mut egui::Ui) {
        let count = self.session.worksheet_count();
        let active = self.session.active();
        let names: Vec<String> = self
            .session
            .workbook()
            .worksheets
            .iter()
            .map(|w| w.name.clone())
            .collect();

        let mut action: Option<TabAction> = None;
        ui.horizontal_wrapped(|ui| {
            for (index, name) in names.iter().enumerate() {
                if let Some((renaming, buffer)) = self.renaming.as_mut()
                    && *renaming == index
                {
                    let response = ui.text_edit_singleline(buffer);
                    response.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = Some(TabAction::CommitRename(index, buffer.clone()));
                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        action = Some(TabAction::CancelRename);
                    }
                    continue;
                }

                let response = ui.selectable_label(index == active, name);
                if response.clicked() {
                    action = Some(TabAction::Activate(index));
                }
                if response.double_clicked() {
                    action = Some(TabAction::StartRename(index, name.clone()));
                }
                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        action = Some(TabAction::StartRename(index, name.clone()));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        action = Some(TabAction::Duplicate(index));
                        ui.close();
                    }
                    if ui.button("Move left").clicked() {
                        action = Some(TabAction::Move(index, -1));
                        ui.close();
                    }
                    if ui.button("Move right").clicked() {
                        action = Some(TabAction::Move(index, 1));
                        ui.close();
                    }
                    if ui.button("Close").clicked() {
                        action = Some(TabAction::Close(index));
                        ui.close();
                    }
                });
            }
            if ui.small_button("＋").clicked() {
                action = Some(TabAction::Add);
            }
        });

        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Mark").weak().small());
            for (mark, label) in [
                (Mark::Bar, "Bar"),
                (Mark::Line, "Line"),
                (Mark::Area, "Area"),
                (Mark::Point, "Point"),
                (Mark::Arc, "Pie"),
            ] {
                let selected = self
                    .session
                    .active_worksheet()
                    .is_some_and(|w| w.mark == mark);
                if ui.selectable_label(selected, label).clicked() {
                    self.session.set_mark(mark);
                }
            }
        });

        let _ = count;
        if let Some(action) = action {
            self.apply_tab_action(action);
        }
    }

    fn apply_tab_action(&mut self, action: TabAction) {
        match action {
            TabAction::Activate(index) => self.session.set_active(index),
            TabAction::StartRename(index, name) => self.renaming = Some((index, name)),
            TabAction::CommitRename(index, name) => {
                self.session.rename_worksheet(index, &name);
                self.renaming = None;
            }
            TabAction::CancelRename => self.renaming = None,
            TabAction::Duplicate(index) => self.session.duplicate_worksheet(index),
            TabAction::Move(index, delta) => self.session.move_worksheet(index, delta),
            TabAction::Close(index) => {
                let Some(worksheet) = self
                    .session
                    .workbook()
                    .worksheets
                    .get(index)
                    .map(|w| w.id.clone())
                else {
                    return;
                };
                let used = self.session.used_in(&worksheet);
                if used.is_empty() {
                    self.session.remove_worksheet(index);
                } else {
                    self.pending_delete = Some((index, used));
                }
            }
            TabAction::Add => self.session.add_worksheet(),
        }
    }

    fn data_pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("DATA").weak().small().strong());

        let (dimensions, measures) = self.session.field_groups();
        if dimensions.is_empty() && measures.is_empty() {
            ui.label("Drop a CSV, Parquet or JSON file onto the window.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Or path:");
                ui.text_edit_singleline(&mut self.path_input);
                if ui.button("Load").clicked() && !self.path_input.is_empty() {
                    self.open_path(PathBuf::from(self.path_input.trim().to_owned()));
                }
            });
            return;
        }

        ui.label(
            egui::RichText::new("Drag a field onto a shelf, or click it to place it.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);
        if !dimensions.is_empty() {
            ui.label(egui::RichText::new("Dimensions").weak().small());
            for (id, name) in dimensions {
                self.field_card(ui, id, name, Role::Dimension);
            }
        }
        ui.add_space(6.0);
        if !measures.is_empty() {
            ui.label(egui::RichText::new("Measures").weak().small());
            for (id, name) in measures {
                self.field_card(ui, id, name, Role::Measure);
            }
        }

        ui.add_space(10.0);
        if ui.button("ƒ  New calculation").clicked() {
            self.calc_open = true;
            self.calc_error = None;
        }
    }

    fn field_card(&mut self, ui: &mut egui::Ui, id: FieldId, name: String, role: Role) {
        let payload = DragField { field: id.clone() };
        let inner = ui.dnd_drag_source(egui::Id::unique(("field", id.as_str())), payload, |ui| {
            let button =
                egui::Button::new(name.clone()).min_size(egui::vec2(ui.available_width(), 26.0));
            ui.add(button)
        });
        if inner.inner.clicked() {
            let shelf = if role == Role::Measure {
                Shelf::Y
            } else {
                Shelf::X
            };
            self.session.assign(shelf, &id);
        }
        inner.inner.context_menu(|ui| {
            if ui.button("Format…").clicked() {
                self.open_format(&id);
                ui.close();
            }
        });
    }

    /// Loads a field's current format into the editor and opens it.
    fn open_format(&mut self, id: &FieldId) {
        match self.session.field_format(id) {
            Some(slickbi_spec::FieldFormat::Number {
                decimals,
                thousands,
                currency,
            }) => {
                self.format_decimals = decimals;
                self.format_thousands = thousands;
                self.format_currency = currency.unwrap_or_default();
            }
            _ => {
                self.format_decimals = 0;
                self.format_thousands = false;
                self.format_currency.clear();
            }
        }
        self.format_field = Some(id.clone());
    }

    fn shelves(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(egui::RichText::new("SHELVES").weak().small().strong());
        for (shelf, label, hint) in [
            (Shelf::X, "Columns", "Drop a dimension here"),
            (Shelf::Y, "Rows", "Drop a measure here"),
            (Shelf::Color, "Color", "Drop a dimension here to split"),
        ] {
            ui.label(egui::RichText::new(label).small());
            self.shelf_drop_zone(ui, shelf, hint);
            ui.add_space(4.0);
        }
    }

    fn shelf_drop_zone(&mut self, ui: &mut egui::Ui, shelf: Shelf, hint: &str) {
        let channel = self.shelf_channel(shelf);
        let name = channel
            .as_ref()
            .and_then(|c| self.field_name(&c.field))
            .unwrap_or_default();

        let (_, payload) = ui.dnd_drop_zone::<DragField, _>(shelf_frame(), |ui| {
            if let Some(channel) = &channel {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&name).strong().monospace());
                    agg_combo(ui, shelf_id(shelf), channel.agg, |agg| {
                        self.session.set_agg(shelf, agg)
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").clicked() {
                            self.session.clear(shelf);
                        }
                    });
                });
            } else {
                ui.label(egui::RichText::new(hint).weak());
            }
        });
        if let Some(payload) = payload {
            self.session.assign(shelf, &payload.field);
        }
    }

    fn shelf_channel(&self, shelf: Shelf) -> Option<Channel> {
        let worksheet = self.session.active_worksheet()?;
        match shelf {
            Shelf::X => worksheet.encoding.x.clone(),
            Shelf::Y => worksheet.encoding.y.clone(),
            Shelf::Color => worksheet.encoding.color.clone(),
        }
    }

    fn field_name(&self, id: &FieldId) -> Option<String> {
        let worksheet = self.session.active_worksheet()?;
        let model = self
            .session
            .workbook()
            .models
            .iter()
            .find(|m| m.id == worksheet.model)?;
        model
            .fields
            .iter()
            .find(|f| &f.id == id)
            .map(|f| f.name.clone())
    }

    fn filters(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("FILTERS").weak().small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("＋").clicked() {
                    self.filter_open = true;
                    self.filter_value.clear();
                    if self.filter_field.is_none() {
                        self.filter_field = self
                            .session
                            .field_groups()
                            .0
                            .first()
                            .map(|(id, _)| id.clone());
                    }
                }
            });
        });

        let Some(worksheet) = self.session.active_worksheet() else {
            return;
        };
        let filters = worksheet.filters.clone();
        if filters.is_empty() {
            ui.label(egui::RichText::new("No filters").weak().small());
        }
        for (index, filter) in filters.iter().enumerate() {
            ui.horizontal(|ui| {
                let name = self
                    .field_name(&filter.field)
                    .unwrap_or_else(|| filter.field.to_string());
                ui.label(
                    egui::RichText::new(format!(
                        "{name} {}",
                        describe_filter(&filter.op, &filter.values)
                    ))
                    .small(),
                );
                if ui.small_button("×").clicked() {
                    self.session.remove_filter(index);
                }
            });
        }
    }

    fn chart_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                self.chart_toolbar(ui);
                ui.add_space(6.0);

                if let Some(error) = self.session.error() {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xB0, 0x30, 0x20),
                        format!("⚠ {error}"),
                    );
                    ui.add_space(4.0);
                }

                egui::Frame::NONE
                    .fill(egui::Color32::WHITE)
                    .corner_radius(10.0)
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(0xE2, 0xE6, 0xEB),
                    ))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| match self.session.scene() {
                        Some(scene) => draw_scene(ui, scene),
                        None => {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 360.0),
                                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(
                                            "Put a dimension on Columns and a measure on Rows \
                                             to draw a chart.",
                                        )
                                        .weak(),
                                    );
                                },
                            );
                        }
                    });
            });
    }

    fn chart_toolbar(&mut self, ui: &mut egui::Ui) {
        let sort = self.sort_state();
        let limit = self
            .session
            .active_worksheet()
            .and_then(|w| w.limit)
            .unwrap_or(0);
        let used_in = self
            .session
            .active_worksheet()
            .map(|w| self.session.used_in(&w.id))
            .unwrap_or_default();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Sort").small().weak());
            let label = match sort {
                None => "None",
                Some((Shelf::X, false)) => "Columns ↑",
                Some((Shelf::X, true)) => "Columns ↓",
                Some((Shelf::Y, false)) => "Rows ↑",
                Some((Shelf::Y, true)) => "Rows ↓",
                Some((Shelf::Color, _)) => "None",
            };
            egui::ComboBox::from_id_salt("sort-combo")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(sort.is_none(), "None").clicked() {
                        self.session.clear_sort();
                    }
                    for (shelf, label, descending) in [
                        (Shelf::X, "Columns ↑", false),
                        (Shelf::X, "Columns ↓", true),
                        (Shelf::Y, "Rows ↑", false),
                        (Shelf::Y, "Rows ↓", true),
                    ] {
                        if ui
                            .selectable_label(sort == Some((shelf, descending)), label)
                            .clicked()
                        {
                            self.session.set_sort(shelf, descending);
                        }
                    }
                });

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Limit").small().weak());
            let mut value = limit;
            if ui
                .add(
                    egui::DragValue::new(&mut value)
                        .range(0..=1_000_000u32)
                        .speed(1.0),
                )
                .changed()
            {
                self.session
                    .set_limit(if value == 0 { None } else { Some(value) });
            }

            if !used_in.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("Used in: {}", used_in.join(", ")))
                        .small()
                        .weak(),
                );
            }
        });
    }

    fn sort_state(&self) -> Option<(Shelf, bool)> {
        let worksheet = self.session.active_worksheet()?;
        let key = worksheet.sort.first()?;
        let x = worksheet.encoding.x.as_ref();
        let y = worksheet.encoding.y.as_ref();
        if x.is_some_and(|c| c.field == key.field) {
            Some((Shelf::X, key.descending))
        } else if y.is_some_and(|c| c.field == key.field) {
            Some((Shelf::Y, key.descending))
        } else {
            None
        }
    }

    fn reports_ui(&mut self, ui: &mut egui::Ui) {
        if self.session.worksheet_count() == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(
                        "Open a data file to build charts, then compose them into a report.",
                    )
                    .weak(),
                );
            });
            return;
        }

        egui::Panel::left("report-palette")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.report_palette(ui);
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            self.report_canvas(ui);
        });
    }

    fn report_palette(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("REPORT").weak().small().strong());

        let reports: Vec<(usize, String)> = self
            .session
            .reports()
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.clone()))
            .collect();
        let active = self.session.active_report();

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("report-combo")
                .selected_text(
                    reports
                        .iter()
                        .find(|(i, _)| *i == active)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| "None".to_owned()),
                )
                .show_ui(ui, |ui| {
                    for (index, name) in &reports {
                        if ui.selectable_label(*index == active, name).clicked() {
                            self.session.set_active_report(*index);
                        }
                    }
                });
            if ui.button("＋ New").clicked() {
                let name = format!("Report {}", self.session.reports().len() + 1);
                self.session.add_report(&name);
            }
        });

        if self.session.reports().is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Create a report, then add charts and text to it.")
                    .weak()
                    .small(),
            );
            return;
        }

        let theme = self
            .session
            .active_report_doc()
            .and_then(|r| r.theme.clone());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Theme").small().weak());
            let label = theme.as_deref().unwrap_or("Light");
            egui::ComboBox::from_id_salt("theme-combo")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (value, name) in [
                        (None, "Light"),
                        (Some("slate".to_owned()), "Slate"),
                        (Some("warm".to_owned()), "Warm"),
                    ] {
                        if ui.selectable_label(theme == value, name).clicked() {
                            self.session.set_report_theme(active, value);
                        }
                    }
                });
        });

        ui.separator();
        ui.label(egui::RichText::new("ADD").weak().small().strong());
        if ui.button("＋ Add active chart").clicked()
            && let Some(worksheet) = self.session.active_worksheet()
        {
            let id = worksheet.id.clone();
            self.session.add_chart_to_report(&id);
        }
        ui.label(egui::RichText::new("Worksheets").weak().small());
        let worksheets: Vec<(usize, String)> = self
            .session
            .workbook()
            .worksheets
            .iter()
            .enumerate()
            .map(|(i, w)| (i, w.name.clone()))
            .collect();
        for (index, name) in worksheets {
            if ui.button(format!("＋ {name}")).clicked() {
                let id = self.session.workbook().worksheets[index].id.clone();
                self.session.add_chart_to_report(&id);
            }
        }
        if ui.button("＋ Text block").clicked() {
            self.session.add_text_to_report("# New section");
        }

        ui.separator();
        ui.label(egui::RichText::new("ELEMENTS").weak().small().strong());
        self.report_elements(ui);
    }

    fn report_elements(&mut self, ui: &mut egui::Ui) {
        let report = self.session.active_report();
        let Some(doc) = self.session.reports().get(report) else {
            return;
        };
        let count = doc.elements.len();
        if count == 0 {
            ui.label(egui::RichText::new("Nothing yet").weak().small());
        }

        let mut action: Option<ElementAction> = None;
        for index in 0..count {
            let frame = self
                .session
                .reports()
                .get(report)
                .and_then(|report| report.elements.get(index))
                .map(|element| element.frame);

            let worksheet = self
                .session
                .chart_element(report, index)
                .map(|(id, _)| id.clone());
            if let Some(worksheet) = worksheet {
                let name = self
                    .session
                    .workbook()
                    .worksheets
                    .iter()
                    .find(|w| w.id == worksheet)
                    .map(|w| w.name.clone())
                    .unwrap_or_else(|| "Chart".to_owned());
                let title = self
                    .session
                    .chart_title(report, index)
                    .map(|t| t.to_owned())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Chart · {name}")).small());
                    self.element_buttons(ui, index, &mut action);
                });
                let mut buffer = title;
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut buffer)
                            .hint_text("Title override")
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.session.set_chart_title(report, index, &buffer);
                }
                self.element_style(ui, report, index);
            }

            if let Some(text) = self
                .session
                .text_element(report, index)
                .map(|t| t.to_owned())
            {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Text").small());
                    self.element_buttons(ui, index, &mut action);
                });
                let mut buffer = text;
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut buffer)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.session.set_text_element(report, index, &buffer);
                }
            }

            if let Some(frame) = frame {
                self.element_geometry(ui, index, frame, &mut action);
            }
            ui.separator();
        }

        if let Some(action) = action {
            match action {
                ElementAction::Remove(index) => self.session.remove_report_element(report, index),
                ElementAction::Reorder(index, delta) => {
                    self.session.reorder_element(report, index, delta);
                }
                ElementAction::Move(index, dx, dy) => {
                    self.session.move_element(report, index, dx, dy);
                }
                ElementAction::Resize(index, width, height) => {
                    self.session.resize_element(report, index, width, height);
                }
            }
        }
    }

    /// The reorder and remove buttons shown on each element row.
    fn element_buttons(&self, ui: &mut egui::Ui, index: usize, action: &mut Option<ElementAction>) {
        if ui
            .small_button("↑")
            .on_hover_text("Send backward")
            .clicked()
        {
            *action = Some(ElementAction::Reorder(index, -1));
        }
        if ui
            .small_button("↓")
            .on_hover_text("Bring forward")
            .clicked()
        {
            *action = Some(ElementAction::Reorder(index, 1));
        }
        if ui
            .small_button("×")
            .on_hover_text("Remove from report")
            .clicked()
        {
            *action = Some(ElementAction::Remove(index));
        }
    }

    /// The nudge and resize controls for one element.
    fn element_geometry(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        frame: slickbi_spec::Rect,
        action: &mut Option<ElementAction>,
    ) {
        ui.horizontal(|ui| {
            if ui.small_button("◀").on_hover_text("Move left").clicked() {
                *action = Some(ElementAction::Move(index, -20.0, 0.0));
            }
            if ui.small_button("▲").on_hover_text("Move up").clicked() {
                *action = Some(ElementAction::Move(index, 0.0, -20.0));
            }
            if ui.small_button("▼").on_hover_text("Move down").clicked() {
                *action = Some(ElementAction::Move(index, 0.0, 20.0));
            }
            if ui.small_button("▶").on_hover_text("Move right").clicked() {
                *action = Some(ElementAction::Move(index, 20.0, 0.0));
            }
            ui.label(egui::RichText::new("W").small().weak());
            let mut width = frame.width as f32;
            if ui
                .add(
                    egui::DragValue::new(&mut width)
                        .range(80.0..=4000.0)
                        .speed(5.0),
                )
                .changed()
            {
                *action = Some(ElementAction::Resize(index, width as f64, frame.height));
            }
            ui.label(egui::RichText::new("H").small().weak());
            let mut height = frame.height as f32;
            if ui
                .add(
                    egui::DragValue::new(&mut height)
                        .range(60.0..=4000.0)
                        .speed(5.0),
                )
                .changed()
            {
                *action = Some(ElementAction::Resize(index, frame.width, height as f64));
            }
        });
    }

    /// The presentation override toggles for one chart element.
    fn element_style(&mut self, ui: &mut egui::Ui, report: usize, index: usize) {
        let Some(mut overrides) = self.session.chart_overrides(report, index) else {
            return;
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            let mut show_title = overrides.show_title.unwrap_or(true);
            if ui.checkbox(&mut show_title, "Title").changed() {
                overrides.show_title = Some(show_title);
                changed = true;
            }
            let mut show_legend = overrides.show_legend.unwrap_or(true);
            if ui.checkbox(&mut show_legend, "Legend").changed() {
                overrides.show_legend = Some(show_legend);
                changed = true;
            }
            let mut show_axes = overrides.show_axes.unwrap_or(true);
            if ui.checkbox(&mut show_axes, "Axes").changed() {
                overrides.show_axes = Some(show_axes);
                changed = true;
            }
            let mut border = overrides.border.unwrap_or(false);
            if ui.checkbox(&mut border, "Border").changed() {
                overrides.border = Some(border);
                changed = true;
            }
            ui.label(egui::RichText::new("Pad").small().weak());
            let mut padding = overrides.padding.unwrap_or(0.0) as f32;
            if ui
                .add(
                    egui::DragValue::new(&mut padding)
                        .range(0.0..=48.0)
                        .speed(1.0),
                )
                .changed()
            {
                overrides.padding = Some(padding as f64);
                changed = true;
            }
        });
        if changed {
            self.session.set_chart_overrides(report, index, overrides);
        }
    }

    fn report_canvas(&mut self, ui: &mut egui::Ui) {
        /// Space reserved above an element's chart for its title, in points.
        const TITLE_BAND: f32 = 22.0;

        if self.session.reports().is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No reports yet — create one in the panel on the left.")
                        .weak(),
                );
            });
            return;
        }

        let Some(report) = self.session.active_report_doc().cloned() else {
            return;
        };

        // The report name is a document header, outside the element canvas, so an element's
        // frame coordinates map directly onto the canvas with nothing hidden underneath.
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&report.name).heading());
        ui.add_space(6.0);

        let available = ui.available_size();
        let scale = (available.x / report.canvas.width as f32).clamp(0.2, 1.4);
        let size = egui::vec2(
            report.canvas.width as f32 * scale,
            report.canvas.height as f32 * scale,
        );
        let (canvas_rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter_at(canvas_rect);

        let (background, ink) = theme_colors(report.theme.as_deref());
        painter.rect_filled(canvas_rect, 8.0, background);

        let report_index = self.session.active_report();
        let mut remove = None;

        for (index, element) in report.elements.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    canvas_rect.min.x + element.frame.x as f32 * scale,
                    canvas_rect.min.y + element.frame.y as f32 * scale,
                ),
                egui::vec2(
                    element.frame.width as f32 * scale,
                    element.frame.height as f32 * scale,
                )
                .max(egui::vec2(40.0, 30.0)),
            );

            match &element.kind {
                slickbi_spec::ElementKind::Chart { chart } => {
                    // The element title: the override, else the worksheet name, unless hidden.
                    let show_title = chart.overrides.show_title.unwrap_or(true);
                    let mut chart_rect = rect;
                    if show_title && rect.height() > TITLE_BAND + 40.0 {
                        let title = chart.title.clone().or_else(|| {
                            self.session
                                .workbook()
                                .worksheets
                                .iter()
                                .find(|w| w.id == chart.worksheet)
                                .map(|w| w.name.clone())
                        });
                        if let Some(title) = title {
                            painter.text(
                                rect.min,
                                egui::Align2::LEFT_TOP,
                                &title,
                                egui::FontId::proportional(13.0),
                                ink,
                            );
                            chart_rect = egui::Rect::from_min_max(
                                rect.min + egui::vec2(0.0, TITLE_BAND),
                                rect.max,
                            );
                        }
                    }
                    let padding = chart.overrides.padding.unwrap_or(0.0) as f32 * scale;
                    if padding > 0.0 {
                        chart_rect = chart_rect.shrink(padding);
                    }
                    let options = slickbi_scene::SceneOptions {
                        show_axes: chart.overrides.show_axes.unwrap_or(true),
                        show_legend: chart.overrides.show_legend.unwrap_or(true),
                    };
                    let scaled = (chart_rect.width().max(1.0), chart_rect.height().max(1.0));
                    let scene = self
                        .session
                        .scene_for_with(&chart.worksheet, scaled, options)
                        .ok()
                        .flatten();
                    match scene {
                        Some(scene) => {
                            draw_scene_in(ui, &scene, chart_rect);
                        }
                        None => {
                            painter.rect_filled(
                                chart_rect,
                                6.0,
                                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 12),
                            );
                            painter.text(
                                chart_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "No shelves yet",
                                egui::FontId::proportional(13.0),
                                egui::Color32::from_gray(140),
                            );
                        }
                    }
                    if chart.overrides.border == Some(true) {
                        painter.rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 40),
                            ),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                slickbi_spec::ElementKind::Text { text } => {
                    draw_text_block(&painter, rect, &text.markdown, ink);
                }
            }

            let remove_rect = egui::Rect::from_min_size(
                rect.right_top() + egui::vec2(-24.0, 4.0),
                egui::vec2(20.0, 20.0),
            );
            if ui
                .put(remove_rect, egui::Button::new("×").small())
                .on_hover_text("Remove from report")
                .clicked()
            {
                remove = Some(index);
            }
        }

        if let Some(index) = remove {
            self.session.remove_report_element(report_index, index);
        }
    }

    fn calc_window(&mut self, ctx: &egui::Context) {
        if !self.calc_open {
            return;
        }
        let mut open = self.calc_open;
        egui::Window::new("New calculation")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Field name").small().weak());
                ui.text_edit_singleline(&mut self.calc_name);
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Expression").small().weak());
                ui.add(
                    egui::TextEdit::multiline(&mut self.calc_expr)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Fields are written [Like This]. Try: [amount] / [units], or SUM([amount]).",
                    )
                    .weak()
                    .small(),
                );
                if let Some(error) = &self.calc_error {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xB0, 0x30, 0x20), error);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Add field").clicked() && !self.calc_name.trim().is_empty() {
                        let name = self.calc_name.trim().to_owned();
                        let expr = self.calc_expr.clone();
                        match self.session.add_calculated_field(&name, &expr) {
                            Ok(()) => {
                                self.calc_open = false;
                                self.calc_name.clear();
                                self.calc_expr.clear();
                                self.calc_error = None;
                            }
                            Err(error) => self.calc_error = Some(error.to_string()),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.calc_open = false;
                        self.calc_error = None;
                    }
                });
            });
        self.calc_open = open;
    }

    fn filter_window(&mut self, ctx: &egui::Context) {
        if !self.filter_open {
            return;
        }
        let mut open = self.filter_open;
        egui::Window::new("Add filter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                let fields: Vec<(FieldId, String)> = {
                    let (d, m) = self.session.field_groups();
                    d.into_iter().chain(m).collect()
                };
                ui.label(egui::RichText::new("Field").small().weak());
                let selected = self.filter_field.clone();
                egui::ComboBox::from_id_salt("filter-field")
                    .selected_text(
                        fields
                            .iter()
                            .find(|(id, _)| Some(id) == selected.as_ref())
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "Field".to_owned()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &fields {
                            ui.selectable_value(&mut self.filter_field, Some(id.clone()), name);
                        }
                    });

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Condition").small().weak());
                egui::ComboBox::from_id_salt("filter-op")
                    .selected_text(op_label(self.filter_op))
                    .show_ui(ui, |ui| {
                        for (op, label) in OP_OPTIONS {
                            ui.selectable_value(&mut self.filter_op, op, label);
                        }
                    });

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Value").small().weak());
                ui.text_edit_singleline(&mut self.filter_value);

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked()
                        && let Some(field) = self.filter_field.clone()
                    {
                        let values = parse_value(&self.filter_value);
                        if !values.is_empty() || self.filter_op == FilterOp::IsNull {
                            self.session.add_filter(&field, self.filter_op, values);
                            self.filter_open = false;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.filter_open = false;
                    }
                });
            });
        self.filter_open = open;
    }

    fn format_window(&mut self, ctx: &egui::Context) {
        let Some(field) = self.format_field.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Field format")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                let name = self.field_name(&field).unwrap_or_else(|| field.to_string());
                ui.label(egui::RichText::new(name).strong());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Decimals").small().weak());
                    ui.add(egui::DragValue::new(&mut self.format_decimals).range(0..=6));
                    ui.checkbox(&mut self.format_thousands, "Thousands");
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Currency or prefix").small().weak());
                ui.text_edit_singleline(&mut self.format_currency);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let currency = self.format_currency.trim();
                        let format = slickbi_spec::FieldFormat::Number {
                            decimals: self.format_decimals,
                            thousands: self.format_thousands,
                            currency: (!currency.is_empty()).then(|| currency.to_owned()),
                        };
                        self.session.set_field_format(&field, Some(format));
                        self.format_field = None;
                    }
                    if ui.button("Clear").clicked() {
                        self.session.set_field_format(&field, None);
                        self.format_field = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.format_field = None;
                    }
                });
            });
        if !open {
            self.format_field = None;
        }
    }

    fn delete_window(&mut self, ctx: &egui::Context) {
        let Some((index, used)) = self.pending_delete.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Delete worksheet?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "This worksheet is used in {} report(s):",
                    used.len()
                ));
                for name in &used {
                    ui.label(egui::RichText::new(format!("• {name}")).strong());
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Those report elements will show a placeholder until you point them \
                         at another worksheet.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete anyway").clicked() {
                        self.session.remove_worksheet(index);
                        self.pending_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_delete = None;
                    }
                });
            });
        if !open {
            self.pending_delete = None;
        }
    }
}

impl eframe::App for SlickBiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> = ui
            .input(|i| i.raw.dropped_files.clone())
            .into_iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        for path in dropped {
            self.open_path(path);
        }

        egui::Panel::top("top").show(ui, |ui| {
            self.top_bar(ui);
        });

        if self.session.worksheet_count() == 0 {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(egui::RichText::new("SlickBI").heading().strong());
                        ui.add_space(6.0);
                        ui.label("Drop a CSV, Parquet or JSON file anywhere to begin.");
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label("Path:");
                            ui.text_edit_singleline(&mut self.path_input);
                            if ui.button("Load").clicked() && !self.path_input.is_empty() {
                                self.open_path(PathBuf::from(self.path_input.trim().to_owned()));
                            }
                        });
                    });
                });
            });
        } else {
            match self.mode {
                Mode::Charts => self.charts_ui(ui),
                Mode::Reports => self.reports_ui(ui),
            }
        }

        self.calc_window(ui.ctx());
        self.filter_window(ui.ctx());
        self.delete_window(ui.ctx());
        self.format_window(ui.ctx());
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../render/fonts/Inter-Regular.ttf"
        ))),
    );
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "inter".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn install_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = egui::Color32::from_rgb(0xF5, 0xF6, 0xF8);
    visuals.window_fill = egui::Color32::WHITE;
    visuals.extreme_bg_color = egui::Color32::WHITE;
    visuals.selection.bg_fill = egui::Color32::from_rgb(0x0B, 0x6E, 0x99);
    visuals.hyperlink_color = egui::Color32::from_rgb(0x0B, 0x6E, 0x99);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 24.0;
    });
}

fn shelf_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(0xEC, 0xEF, 0xF3))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

fn shelf_id(shelf: Shelf) -> &'static str {
    match shelf {
        Shelf::X => "agg-x",
        Shelf::Y => "agg-y",
        Shelf::Color => "agg-color",
    }
}

fn agg_combo(ui: &mut egui::Ui, salt: &'static str, current: Agg, mut set: impl FnMut(Agg)) {
    const OPTIONS: [(Agg, &str); 8] = [
        (Agg::None, "None"),
        (Agg::Sum, "Sum"),
        (Agg::Average, "Avg"),
        (Agg::Median, "Median"),
        (Agg::Min, "Min"),
        (Agg::Max, "Max"),
        (Agg::Count, "Count"),
        (Agg::CountDistinct, "Distinct"),
    ];
    egui::ComboBox::from_id_salt(salt)
        .width(76.0)
        .selected_text(
            OPTIONS
                .iter()
                .find(|(agg, _)| *agg == current)
                .map(|(_, label)| *label)
                .unwrap_or("None"),
        )
        .show_ui(ui, |ui| {
            for (agg, label) in OPTIONS {
                if ui.selectable_label(agg == current, label).clicked() {
                    set(agg);
                }
            }
        });
}

fn theme_colors(theme: Option<&str>) -> (egui::Color32, egui::Color32) {
    match theme {
        Some("slate") => (
            egui::Color32::from_rgb(0x1B, 0x1F, 0x24),
            egui::Color32::from_rgb(0xEC, 0xEF, 0xF2),
        ),
        Some("warm") => (
            egui::Color32::from_rgb(0xFB, 0xF7, 0xF0),
            egui::Color32::from_rgb(0x2B, 0x26, 0x22),
        ),
        _ => (
            egui::Color32::WHITE,
            egui::Color32::from_rgb(0x24, 0x2C, 0x36),
        ),
    }
}

fn draw_text_block(painter: &egui::Painter, rect: egui::Rect, markdown: &str, ink: egui::Color32) {
    let mut y = rect.min.y;
    for line in markdown.lines() {
        let (text, size) = if let Some(rest) = line.strip_prefix("# ") {
            (rest, 20.0)
        } else if let Some(rest) = line.strip_prefix("## ") {
            (rest, 16.0)
        } else {
            (line, 13.0)
        };
        if !text.is_empty() {
            painter.text(
                egui::pos2(rect.min.x, y),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(size),
                ink,
            );
        }
        y += size + 6.0;
        if y > rect.max.y {
            break;
        }
    }
}

fn describe_filter(op: &FilterOp, values: &[Scalar]) -> String {
    let first = || values.first().map(scalar_text).unwrap_or_default();
    match op {
        FilterOp::Equal => format!("= {}", first()),
        FilterOp::NotEqual => format!("≠ {}", first()),
        FilterOp::Greater => format!("> {}", first()),
        FilterOp::GreaterOrEqual => format!("≥ {}", first()),
        FilterOp::Less => format!("< {}", first()),
        FilterOp::LessOrEqual => format!("≤ {}", first()),
        FilterOp::Contains => format!("contains {}", first()),
        FilterOp::StartsWith => format!("starts with {}", first()),
        FilterOp::EndsWith => format!("ends with {}", first()),
        FilterOp::IsNull => "is null".to_owned(),
        FilterOp::IsNotNull => "is not null".to_owned(),
        FilterOp::In => format!(
            "in ({})",
            values
                .iter()
                .map(scalar_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        FilterOp::NotIn => format!(
            "not in ({})",
            values
                .iter()
                .map(scalar_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        FilterOp::Between => format!(
            "between {} and {}",
            first(),
            values.get(1).map(scalar_text).unwrap_or_default()
        ),
    }
}

fn scalar_text(value: &Scalar) -> String {
    match value {
        Scalar::Boolean(v) => v.to_string(),
        Scalar::Integer(v) => v.to_string(),
        Scalar::Number(v) => v.to_string(),
        Scalar::Text(v) => v.clone(),
    }
}

const OP_OPTIONS: [(FilterOp, &str); 12] = [
    (FilterOp::Equal, "="),
    (FilterOp::NotEqual, "≠"),
    (FilterOp::Greater, ">"),
    (FilterOp::GreaterOrEqual, "≥"),
    (FilterOp::Less, "<"),
    (FilterOp::LessOrEqual, "≤"),
    (FilterOp::Contains, "contains"),
    (FilterOp::StartsWith, "starts with"),
    (FilterOp::EndsWith, "ends with"),
    (FilterOp::IsNull, "is null"),
    (FilterOp::IsNotNull, "is not null"),
    (FilterOp::Between, "between"),
];

fn op_label(op: FilterOp) -> &'static str {
    OP_OPTIONS
        .iter()
        .find(|(o, _)| *o == op)
        .map(|(_, label)| *label)
        .unwrap_or("=")
}

fn parse_value(text: &str) -> Vec<Scalar> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    if let Ok(number) = trimmed.parse::<f64>() {
        return vec![Scalar::Number(number)];
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return vec![Scalar::Integer(integer)];
    }
    vec![Scalar::Text(trimmed.to_owned())]
}

fn draw_scene(ui: &mut egui::Ui, scene: &Scene) {
    let size = egui::vec2(scene.width, scene.height);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    draw_scene_in(ui, scene, rect);
}

fn draw_scene_in(ui: &mut egui::Ui, scene: &Scene, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);

    let plot = egui::Rect::from_min_size(
        rect.min + egui::vec2(scene.plot.x, scene.plot.y),
        egui::vec2(scene.plot.width, scene.plot.height),
    );
    let has_axes = !scene.x_axis.ticks.is_empty() || !scene.y_axis.ticks.is_empty();
    if has_axes {
        let axis = egui::Color32::from_rgb(0xB8, 0xC0, 0xC9);
        for tick in &scene.y_axis.ticks {
            let y = rect.min.y + tick.position;
            painter.line_segment(
                [egui::pos2(plot.min.x, y), egui::pos2(plot.max.x, y)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0xED, 0xF0, 0xF4)),
            );
        }
        painter.rect_stroke(
            plot,
            0.0,
            egui::Stroke::new(1.0, axis),
            egui::StrokeKind::Inside,
        );
    }

    for series in &scene.series {
        let color = to_color(series.color);
        match &series.kind {
            SeriesKind::Bars(bars) => {
                for bar in bars {
                    paint_bar(&painter, rect.min, bar, color);
                }
            }
            SeriesKind::Line(points) => paint_line(&painter, rect.min, points, color),
            SeriesKind::Area(points) => {
                paint_area(&painter, rect.min, plot, points, color);
                paint_line(&painter, rect.min, points, color);
            }
            SeriesKind::Points(points) => {
                for point in points {
                    let center = rect.min + egui::vec2(point.x, point.y);
                    painter.circle_filled(center, 2.5, color);
                }
            }
            SeriesKind::Arc(slice) => paint_arc(&painter, rect.min, slice, color),
        }
    }

    let font = egui::FontId::proportional(11.0);
    // Legend across the top, when the chart splits by colour.
    if !scene.legend.is_empty() {
        let mut x = rect.min.x + scene.plot.x;
        let center_y = rect.min.y + scene.plot.y / 2.0;
        for entry in &scene.legend {
            let color = to_color(entry.color);
            let swatch =
                egui::Rect::from_center_size(egui::pos2(x + 5.0, center_y), egui::vec2(10.0, 10.0));
            painter.rect_filled(swatch, 2.0, color);
            let text_rect = painter.text(
                egui::pos2(x + 16.0, center_y),
                egui::Align2::LEFT_CENTER,
                &entry.label,
                font.clone(),
                egui::Color32::from_rgb(0x3D, 0x47, 0x54),
            );
            x = text_rect.right() + 16.0;
        }
    }
    if has_axes {
        for tick in &scene.y_axis.ticks {
            painter.text(
                egui::pos2(plot.min.x - 6.0, rect.min.y + tick.position),
                egui::Align2::RIGHT_CENTER,
                &tick.text,
                font.clone(),
                egui::Color32::from_rgb(0x3D, 0x47, 0x54),
            );
        }
        for tick in &scene.x_axis.ticks {
            painter.text(
                egui::pos2(rect.min.x + tick.position, plot.max.y + 6.0),
                egui::Align2::CENTER_TOP,
                &tick.text,
                font.clone(),
                egui::Color32::from_rgb(0x3D, 0x47, 0x54),
            );
        }
    }
}

fn paint_bar(painter: &egui::Painter, origin: egui::Pos2, bar: &Bar, color: egui::Color32) {
    let rect = egui::Rect::from_min_size(
        origin + egui::vec2(bar.x, bar.y),
        egui::vec2(bar.width, bar.height),
    );
    painter.rect_filled(rect, 2.0, color);
}

fn paint_line(
    painter: &egui::Painter,
    origin: egui::Pos2,
    points: &[slickbi_scene::Point],
    color: egui::Color32,
) {
    let line: Vec<egui::Pos2> = points
        .iter()
        .map(|p| origin + egui::vec2(p.x, p.y))
        .collect();
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, color)));
}

fn paint_area(
    painter: &egui::Painter,
    origin: egui::Pos2,
    plot: egui::Rect,
    points: &[slickbi_scene::Point],
    color: egui::Color32,
) {
    if points.len() < 2 {
        return;
    }
    let mut polygon: Vec<egui::Pos2> = points
        .iter()
        .map(|p| origin + egui::vec2(p.x, p.y))
        .collect();
    polygon.push(egui::pos2(
        origin.x + points.last().expect("len >= 2").x,
        plot.max.y,
    ));
    polygon.push(egui::pos2(origin.x + points[0].x, plot.max.y));
    let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40);
    painter.add(egui::Shape::convex_polygon(
        polygon,
        fill,
        egui::Stroke::NONE,
    ));
}

/// Fills a pie wedge as a triangle fan, which handles slices wider than a half turn.
fn paint_arc(
    painter: &egui::Painter,
    origin: egui::Pos2,
    slice: &slickbi_scene::ArcSlice,
    color: egui::Color32,
) {
    let sweep = slice.end - slice.start;
    let steps = ((sweep.abs() / std::f32::consts::TAU) * 90.0)
        .ceil()
        .max(2.0) as usize;
    let center = origin + egui::vec2(slice.center.x, slice.center.y);

    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for step in 0..=steps {
        let angle = slice.start + sweep * (step as f32 / steps as f32);
        let x = slice.center.x + slice.radius * angle.sin();
        let y = slice.center.y - slice.radius * angle.cos();
        mesh.colored_vertex(origin + egui::vec2(x, y), color);
    }
    for step in 0..steps {
        mesh.add_triangle(0, (step + 1) as u32, (step + 2) as u32);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn to_color(color: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    )
}
