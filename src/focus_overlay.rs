//! Small desktop overlay shown after consecutive off-task decisions.

use std::{
    io::BufRead,
    sync::Arc,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use amber::{Rgb, focus::OFF_TASK_BODY};
use eframe::egui::{self, Color32, ViewportBuilder};

const DIM_ALPHA: u8 = 200;
const SCRIM_ALPHA: u8 = 140;
const BORDER_WIDTH: f32 = 6.0;
const BODY_PREFIX: &str = "Please navigate away from the distraction.\nStay focused on ";
const BODY_COLOR: Color32 = Color32::from_gray(220);

pub fn run(message: String, accent: Option<Rgb>, task: Option<String>) -> eframe::Result {
    // The parent can close stdin when it decides the screen is back on task.
    // Keeping this read off the UI thread leaves the window responsive.
    let (close_sender, close_receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let _ = stdin.lock().read_line(&mut String::new());
        let _ = close_sender.send(());
    });
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Amber focus reminder")
            .with_decorations(false)
            .with_has_shadow(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_active(false)
            .with_mouse_passthrough(true)
            .with_taskbar(false),
        ..Default::default()
    };
    eframe::run_native(
        "Amber focus reminder",
        options,
        Box::new(move |creation_context| {
            install_font(&creation_context.egui_ctx);
            Ok(Box::new(Overlay {
                message,
                task,
                accent: accent.map_or(Color32::WHITE, |[red, green, blue]| {
                    Color32::from_rgb(red, green, blue)
                }),
                close_receiver,
                covers_monitor: false,
            }))
        }),
    )
}

fn install_font(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter Bold".into(),
        Arc::new(
            egui::FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")).tweak(
                egui::FontTweak {
                    coords: egui::epaint::text::VariationCoords::new([
                        (b"wght", 700.0),
                        (b"opsz", 32.0),
                    ]),
                    ..Default::default()
                },
            ),
        ),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    fonts.families.insert(
        egui::FontFamily::Name("Amber Inter Bold".into()),
        vec!["Inter Bold".into(), "Inter".into()],
    );
    context.set_fonts(fonts);
}

struct Overlay {
    message: String,
    task: Option<String>,
    accent: Color32,
    close_receiver: Receiver<()>,
    covers_monitor: bool,
}

impl Overlay {
    fn subtitle(&self, max_width: f32) -> egui::text::LayoutJob {
        let font = egui::FontId::new(30.0, egui::FontFamily::Proportional);
        let format = |color| egui::TextFormat::simple(font.clone(), color);
        let mut job = egui::text::LayoutJob::default();
        match &self.task {
            Some(task) => {
                job.append(BODY_PREFIX, 0.0, format(BODY_COLOR));
                job.append(task, 0.0, format(self.accent));
            }
            None => job.append(OFF_TASK_BODY, 0.0, format(BODY_COLOR)),
        }
        job.wrap = egui::text::TextWrapping {
            max_rows: 2,
            ..egui::text::TextWrapping::truncate_at_width(max_width)
        };
        job
    }
}

impl eframe::App for Overlay {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !matches!(self.close_receiver.try_recv(), Err(TryRecvError::Empty)) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.covers_monitor {
            if let Some(monitor_size) = ui.ctx().input(|input| input.viewport().monitor_size) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::Pos2::ZERO));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(monitor_size));
                self.covers_monitor = true;
            }
        }
        let painter = ui.painter();
        let title = painter.layout_no_wrap(
            self.message.clone(),
            egui::FontId::new(68.0, egui::FontFamily::Name("Amber Inter Bold".into())),
            Color32::WHITE,
        );
        let subtitle = painter.layout_job(self.subtitle(ui.max_rect().width() * 0.8));
        let padding = egui::vec2(36.0, 28.0);
        let gap = 20.0;
        let content_width = title.size().x.max(subtitle.size().x);
        let content_height = title.size().y + gap + subtitle.size().y;
        let panel_size = content_width + padding.x * 2.0;
        let panel_height = content_height + padding.y * 2.0;
        let panel = egui::Rect::from_center_size(
            ui.max_rect().center(),
            egui::vec2(panel_size, panel_height),
        );
        painter.rect_filled(ui.max_rect(), 0.0, Color32::from_black_alpha(SCRIM_ALPHA));
        painter.rect_filled(panel, 16.0, Color32::from_black_alpha(DIM_ALPHA));
        painter.rect_stroke(
            panel,
            16.0,
            egui::Stroke::new(BORDER_WIDTH, self.accent),
            egui::StrokeKind::Inside,
        );
        let content_top = panel.center().y - content_height * 0.5;
        painter.galley(
            egui::pos2(panel.center().x - title.size().x * 0.5, content_top),
            title.clone(),
            Color32::WHITE,
        );
        painter.galley(
            egui::pos2(
                panel.center().x - subtitle.size().x * 0.5,
                content_top + title.size().y + gap,
            ),
            subtitle,
            BODY_COLOR,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}
