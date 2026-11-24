//! The SlickBI desktop window.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use eframe::egui;
use slickbi_app::{Session, Shelf};
use slickbi_engine::SourceFormat;
use slickbi_scene::{Bar, Scene, SeriesKind};
use slickbi_spec::{FieldId, Mark};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("SlickBI"),
        ..Default::default()
    };
    eframe::run_native(
        "SlickBI",
        options,
        Box::new(|cc| Ok(Box::new(SlickBiApp::new(cc)))),
    )
}

struct SlickBiApp {
    session: Session,
    path_input: String,
}

impl SlickBiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // A calm, light, native-feeling default.
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(0xFA, 0xFB, 0xFC);
        visuals.window_fill = egui::Color32::from_rgb(0xFF, 0xFF, 0xFF);
        cc.egui_ctx.set_visuals(visuals);

        SlickBiApp {
            session: Session::new(),
            path_input: String::new(),
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

            if ui.button("Open…").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.open_path(path);
                }
            }
            if ui.button("Save").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("SlickBI workbook", &["scz"])
                    .save_file()
                {
                    if let Err(error) = self.session.save(&path) {
                        eprintln!("failed to save: {error}");
                    }
                }
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
            ui.label("Drop a CSV, Parquet or JSON file to begin.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Path:");
                ui.text_edit_singleline(&mut self.path_input);
                if ui.button("Load").clicked() && !self.path_input.is_empty() {
                    let path = PathBuf::from(self.path_input.trim().to_owned());
                    self.open_path(path);
                }
            });
            return;
        }

        ui.separator();
        ui.label(egui::RichText::new("Dimensions").weak());
        for (id, name) in dimensions {
            self.field_row(ui, id, name);
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Measures").weak());
        for (id, name) in measures {
            self.field_row(ui, id, name);
        }
    }

    fn field_row(&mut self, ui: &mut egui::Ui, id: FieldId, name: String) {
        ui.horizontal(|ui| {
            ui.label(&name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Y").clicked() {
                    self.session.assign(Shelf::Y, &id);
                }
                if ui.small_button("X").clicked() {
                    self.session.assign(Shelf::X, &id);
                }
            });
        });
    }

    fn shelves(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Shelves").strong());
        for (shelf, label) in [(Shelf::X, "Columns"), (Shelf::Y, "Rows")] {
            let name = self.shelf_name(shelf);
            ui.horizontal(|ui| {
                ui.label(label);
                if let Some(name) = &name {
                    ui.label(egui::RichText::new(name).monospace());
                }
                if name.is_some() && ui.small_button("×").clicked() {
                    self.session.clear(shelf);
                }
            });
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
}

impl eframe::App for SlickBiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Files dropped onto the window are opened directly.
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
        egui::Panel::left("data")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                self.data_pane(ui);
                self.shelves(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.chart_panel(ui);
        });
    }
}

fn draw_scene(ui: &mut egui::Ui, scene: &Scene) {
    let size = egui::vec2(scene.width, scene.height);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);

    // Axis gridlines and border.
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

    // Marks.
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

    // Axis labels.
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
