//! The SlickBI desktop window.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use eframe::egui;
use slickbi_app::{Session, Shelf};
use slickbi_engine::SourceFormat;
use slickbi_scene::{Bar, Scene, SeriesKind};
use slickbi_spec::{FieldId, FilterOp, Mark, Role, Scalar};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([760.0, 520.0])
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
}

impl SlickBiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(0xF6, 0xF7, 0xF9);
        visuals.window_fill = egui::Color32::WHITE;
        cc.egui_ctx.set_visuals(visuals);

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
            ui.heading("SlickBI");
            ui.separator();

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
            if self.session.has_source()
                && ui.button("Export CSV").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .save_file()
                && let Err(error) = self.session.export_csv(&path)
            {
                eprintln!("failed to export: {error}");
            }

            ui.separator();
            ui.selectable_value(&mut self.mode, Mode::Charts, "Charts");
            ui.selectable_value(&mut self.mode, Mode::Reports, "Reports");
        });
    }

    fn charts_ui(&mut self, ui: &mut egui::Ui) {
        self.worksheet_tabs(ui);
        egui::Panel::left("data")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                self.data_pane(ui);
                self.shelves(ui);
                self.filters(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.chart_panel(ui);
        });
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
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.heading("Report");
                ui.horizontal(|ui| {
                    if ui.button("＋ Add chart").clicked() {
                        let id = self
                            .session
                            .active_worksheet()
                            .expect("has worksheet")
                            .id
                            .clone();
                        self.session.add_chart_to_report(&id);
                    }
                    if ui.button("＋ Text").clicked() {
                        self.session.add_text_to_report("# New section");
                    }
                });
                ui.separator();

                for i in 0..self.session.worksheet_count() {
                    let name = self
                        .session
                        .workbook()
                        .worksheets
                        .get(i)
                        .map(|w| w.name.clone())
                        .unwrap_or_default();
                    if ui.button(name).clicked() {
                        let id = self
                            .session
                            .workbook()
                            .worksheets
                            .get(i)
                            .expect("worksheet")
                            .id
                            .clone();
                        self.session.add_chart_to_report(&id);
                    }
                }
            });

        egui::CentralPanel::default().show(ui, |ui| {
            self.report_canvas(ui);
        });
    }

    fn worksheet_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for i in 0..self.session.worksheet_count() {
                let name = self
                    .session
                    .workbook()
                    .worksheets
                    .get(i)
                    .map(|w| w.name.clone())
                    .unwrap_or_default();
                let selected = i == self.session.active();
                if ui.selectable_label(selected, name).clicked() {
                    self.session.set_active(i);
                }
            }
            if ui.small_button("＋").clicked() {
                self.session.add_worksheet();
            }
            ui.separator();
            ui.label("Mark:");
            for (mark, label) in [
                (Mark::Bar, "Bar"),
                (Mark::Line, "Line"),
                (Mark::Area, "Area"),
                (Mark::Point, "Point"),
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
    }

    fn data_pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Data");

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

        ui.label("Drag a field onto a shelf, or click it.");
        ui.add_space(2.0);
        ui.separator();
        ui.label(egui::RichText::new("Dimensions").weak());
        for (id, name) in dimensions {
            self.field_card(ui, id, name, Role::Dimension);
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Measures").weak());
        for (id, name) in measures {
            self.field_card(ui, id, name, Role::Measure);
        }

        ui.add_space(10.0);
        ui.separator();
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
            // A click auto-places the field: dimensions on columns, measures on rows.
            let shelf = if role == Role::Measure {
                Shelf::Y
            } else {
                Shelf::X
            };
            self.session.assign(shelf, &id);
        }
    }

    fn shelves(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Shelves").strong());
        for (shelf, label, hint) in [
            (Shelf::X, "Columns", "Drop a dimension here"),
            (Shelf::Y, "Rows", "Drop a measure here"),
        ] {
            ui.label(egui::RichText::new(label).small());
            let assigned = self.shelf_name(shelf);
            let (_, payload) = ui.dnd_drop_zone::<DragField, _>(shelf_frame(), |ui| {
                if let Some(name) = &assigned {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(name).monospace().strong());
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
            ui.add_space(2.0);
        }
    }

    fn shelf_name(&self, shelf: Shelf) -> Option<String> {
        let worksheet = self.session.active_worksheet()?;
        let channel = match shelf {
            Shelf::X => worksheet.encoding.x.as_ref(),
            Shelf::Y => worksheet.encoding.y.as_ref(),
        }?;
        let model = self
            .session
            .workbook()
            .models
            .iter()
            .find(|m| m.id == worksheet.model)?;
        model
            .fields
            .iter()
            .find(|f| f.id == channel.field)
            .map(|f| f.name.clone())
    }

    fn filters(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Filters").strong());
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
        for (index, filter) in filters.iter().enumerate() {
            ui.horizontal(|ui| {
                let name = self
                    .field_name(&filter.field)
                    .unwrap_or_else(|| filter.field.to_string());
                let description = describe_filter(&filter.op, &filter.values);
                ui.label(egui::RichText::new(format!("{name} {description}")).small());
                if ui.small_button("×").clicked() {
                    self.session.remove_filter(index);
                }
            });
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

    fn chart_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.session.error() {
            ui.colored_label(egui::Color32::from_rgb(0xB0, 0x30, 0x20), error);
        }
        match self.session.scene() {
            Some(scene) => draw_scene(ui, scene),
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "Put a dimension on Columns and a measure on Rows to draw a chart.",
                        )
                        .weak(),
                    );
                });
            }
        }
    }

    fn report_canvas(&mut self, ui: &mut egui::Ui) {
        if self.session.reports().is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No reports yet — add charts from the panel on the left.")
                        .weak(),
                );
            });
            return;
        }

        let Some(report) = self.session.active_report_doc().cloned() else {
            return;
        };

        // Fit the report canvas to the available width.
        let available = ui.available_size();
        let scale = (available.x / report.canvas.width as f32).clamp(0.2, 2.0);

        let mut scenes: Vec<(usize, Option<Scene>)> = Vec::new();
        for (index, element) in report.elements.iter().enumerate() {
            if let Some(worksheet) = element.worksheet() {
                let size = (element.frame.width as f32, element.frame.height as f32);
                let scene = self.session.scene_for(worksheet, size).ok().flatten();
                scenes.push((index, scene));
            }
        }

        for (index, scene) in scenes {
            let element = &report.elements[index];
            let rect = egui::Rect::from_min_size(
                ui.min_rect().min
                    + egui::vec2(
                        element.frame.x as f32 * scale,
                        element.frame.y as f32 * scale,
                    ),
                egui::vec2(
                    element.frame.width as f32 * scale,
                    element.frame.height as f32 * scale,
                ),
            );
            if let Some(scene) = scene {
                draw_scene_in(ui, &scene, rect);
            } else {
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(0xFA, 0xFB, 0xFC));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No shelves yet",
                    egui::FontId::proportional(13.0),
                    egui::Color32::GRAY,
                );
            }
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
            .show(ctx, |ui| {
                ui.label("Field name");
                ui.text_edit_singleline(&mut self.calc_name);
                ui.add_space(4.0);
                ui.label("Expression");
                ui.text_edit_multiline(&mut self.calc_expr);
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Fields are written [Like This]. Try: [amount] / [units], or SUM([amount]).",
                    )
                    .weak()
                    .small(),
                );
                if let Some(error) = &self.calc_error {
                    ui.colored_label(egui::Color32::from_rgb(0xB0, 0x30, 0x20), error);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() && !self.calc_name.is_empty() {
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
            .show(ctx, |ui| {
                let fields: Vec<(FieldId, String)> = {
                    let (d, m) = self.session.field_groups();
                    d.into_iter().chain(m).collect()
                };
                if !fields.is_empty() {
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
                }

                egui::ComboBox::from_id_salt("filter-op")
                    .selected_text(op_label(self.filter_op))
                    .show_ui(ui, |ui| {
                        for (op, label) in OP_OPTIONS {
                            ui.selectable_value(&mut self.filter_op, op, label);
                        }
                    });

                ui.label("Value");
                ui.text_edit_singleline(&mut self.filter_value);

                ui.add_space(6.0);
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
                        ui.heading("SlickBI");
                        ui.add_space(8.0);
                        ui.label("Drop a CSV, Parquet or JSON file anywhere to begin.");
                        ui.add_space(8.0);
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
    }
}

fn shelf_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(0xF0, 0xF2, 0xF5))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
}

fn describe_filter(op: &FilterOp, values: &[Scalar]) -> String {
    match op {
        FilterOp::Equal => format!("= {}", values.first().map(scalar_text).unwrap_or_default()),
        FilterOp::NotEqual => format!("≠ {}", values.first().map(scalar_text).unwrap_or_default()),
        FilterOp::Greater => format!("> {}", values.first().map(scalar_text).unwrap_or_default()),
        FilterOp::GreaterOrEqual => {
            format!("≥ {}", values.first().map(scalar_text).unwrap_or_default())
        }
        FilterOp::Less => format!("< {}", values.first().map(scalar_text).unwrap_or_default()),
        FilterOp::LessOrEqual => {
            format!("≤ {}", values.first().map(scalar_text).unwrap_or_default())
        }
        FilterOp::Contains => format!(
            "contains {}",
            values.first().map(scalar_text).unwrap_or_default()
        ),
        FilterOp::StartsWith => format!(
            "starts with {}",
            values.first().map(scalar_text).unwrap_or_default()
        ),
        FilterOp::EndsWith => format!(
            "ends with {}",
            values.first().map(scalar_text).unwrap_or_default()
        ),
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
            values.first().map(scalar_text).unwrap_or_default(),
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

const OP_OPTIONS: [(FilterOp, &str); 10] = [
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
        }
    }

    let font = egui::FontId::proportional(11.0);
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

fn to_color(color: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    )
}
