#![windows_subsystem = "windows"]

mod bypass;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Id, Key,
    Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextEdit, TextStyle, Vec2,
};

use bypass::Event;

const BG: Color32 = Color32::from_rgb(0x11, 0x11, 0x13);
const SURFACE: Color32 = Color32::from_rgb(0x18, 0x18, 0x1b);
const SURFACE_HOVER: Color32 = Color32::from_rgb(0x20, 0x20, 0x24);
const CONTROL: Color32 = Color32::from_rgb(0x22, 0x22, 0x26);
const CONTROL_HOVER: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x30);
const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2f);
const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xee);
const PRIMARY_HOVER: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
const MUTED: Color32 = Color32::from_rgb(0x8b, 0x8b, 0x93);
const FAINT: Color32 = Color32::from_rgb(0x5c, 0x5c, 0x64);
const OK: Color32 = Color32::from_rgb(0x5b, 0xb9, 0x8c);
const ERR: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

const RADIUS: u8 = 8;
const ROW_FADE: f32 = 0.28;
const TOAST_TIME: f64 = 1.8;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BlazeBypass")
            .with_inner_size([780.0, 680.0])
            .with_min_inner_size([560.0, 520.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("BlazeBypass", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

enum State {
    Pending,
    Ok(String),
    Err(String),
}

struct Row {
    link: String,
    state: State,
    shown_at: f64,
}

#[derive(Default)]
struct App {
    input: String,
    refresh: bool,
    rows: Vec<Row>,
    rx: Option<Receiver<Event>>,
    saved: Option<PathBuf>,
    save_error: Option<String>,
    toast: Option<(String, f64)>,
    api_key: String,
    key_input: String,
    key_open: bool,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        setup_fonts(ctx);
        setup_style(ctx);
        let api_key = load_api_key();
        Self { key_input: api_key.clone(), key_open: api_key.is_empty(), api_key, ..Default::default() }
    }

    fn parsed_links(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut links: Vec<String> = Vec::new();
        for token in self.input.split_whitespace() {
            // Skip stray words like "Link:" that can't be a URL.
            if !token.contains('.') {
                continue;
            }
            let link = if token.contains("://") { token.to_string() } else { format!("https://{token}") };
            if seen.insert(link.clone()) {
                links.push(link);
            }
        }
        links
    }

    fn running(&self) -> bool {
        self.rx.is_some()
    }

    fn successes(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter_map(|r| match &r.state {
                State::Ok(url) => Some(url.as_str()),
                _ => None,
            })
            .collect()
    }

    fn start(&mut self, ctx: &egui::Context) {
        let links = self.parsed_links();
        if links.is_empty() || self.running() {
            return;
        }
        if self.api_key.is_empty() {
            self.key_open = true;
            self.show_toast(ctx, String::from("Add your API key first"));
            return;
        }
        // Reset progress and row animations left over from the previous run.
        ctx.clear_animations();
        let now = ctx.input(|i| i.time);
        self.rows = links
            .iter()
            .enumerate()
            .map(|(i, link)| Row { link: link.clone(), state: State::Pending, shown_at: now + (i.min(20) as f64) * 0.025 })
            .collect();
        self.saved = None;
        self.save_error = None;

        let (tx, rx) = mpsc::channel();
        let repaint_ctx = ctx.clone();
        bypass::spawn(links, self.api_key.clone(), self.refresh, tx, move || repaint_ctx.request_repaint());
        self.rx = Some(rx);
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut finished = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Done(index, result) => {
                    if let Some(row) = self.rows.get_mut(index) {
                        row.state = match result {
                            Ok(url) => State::Ok(url),
                            Err(e) => State::Err(e),
                        };
                    }
                }
                Event::Finished => finished = true,
            }
        }
        if finished {
            self.rx = None;
            self.save_results();
        }
    }

    fn save_results(&mut self) {
        let urls = self.successes();
        if urls.is_empty() {
            return;
        }
        let dir = results_dir();
        let stem = chrono::Local::now().format("bypass_%Y-%m-%d_%H-%M-%S").to_string();
        let mut path = dir.join(format!("{stem}.txt"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem}_{n}.txt"));
            n += 1;
        }
        let mut content = urls.join("\r\n");
        content.push_str("\r\n");
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, content)) {
            Ok(()) => self.saved = Some(path),
            Err(e) => self.save_error = Some(format!("Could not save results: {e}")),
        }
    }

    fn copy_all(&mut self, ctx: &egui::Context) {
        let urls = self.successes();
        if urls.is_empty() {
            return;
        }
        let count = urls.len();
        ctx.copy_text(urls.join("\n"));
        self.show_toast(ctx, format!("Copied {count} link{}", if count == 1 { "" } else { "s" }));
    }

    fn show_toast(&mut self, ctx: &egui::Context, text: String) {
        self.toast = Some((text, ctx.input(|i| i.time)));
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(BG).to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();

        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
            self.start(&ctx);
        }

        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin::same(24))).show(ui, |ui| {
            self.header(ui);
            self.key_section(ui);
            ui.add_space(18.0);
            self.input_section(ui);
            ui.add_space(14.0);
            self.controls(ui);
            ui.add_space(18.0);
            self.results(ui);
        });

        self.paint_toast(&ctx);
    }
}

// ---------------------------------------------------------------------------
// Sections

impl App {
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("BlazeBypass").font(semibold(19.0)).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.api_key.is_empty() { "Set API key" } else { "API key" };
                let response = ui.add(
                    egui::Label::new(RichText::new(label).size(12.5).color(if self.key_open { TEXT } else { FAINT }))
                        .sense(Sense::click()),
                );
                if response.clicked() {
                    self.key_open = !self.key_open;
                    self.key_input = self.api_key.clone();
                }
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
            });
        });
        ui.add_space(2.0);
        ui.label(RichText::new("Resolve ad-gated links into direct URLs ready for JDownloader.").color(MUTED));
    }

    fn key_section(&mut self, ui: &mut egui::Ui) {
        let open = ui.ctx().animate_bool_with_time(Id::unique("key_open"), self.key_open, 0.2);
        if open <= 0.0 {
            return;
        }
        if open < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.add_space(14.0 * open);
        ui.scope(|ui| {
            ui.set_opacity(ease_out(open));
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(RADIUS))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new("bypass.vip API key").font(semibold(13.0)).color(TEXT));
                    ui.label(
                        RichText::new("Stored in api_key.txt next to the exe. Sent as the x-api-key header.")
                            .size(12.0)
                            .color(FAINT),
                    );
                    ui.add_space(6.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 34.0),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            let changed = self.key_input.trim() != self.api_key;
                            if button(ui, "Save", true, changed && !self.key_input.trim().is_empty()).clicked() {
                                self.api_key = self.key_input.trim().to_string();
                                match save_api_key(&self.api_key) {
                                    Ok(()) => {
                                        self.key_open = false;
                                        self.show_toast(ui.ctx(), String::from("API key saved"));
                                    }
                                    Err(e) => self.show_toast(ui.ctx(), format!("Could not save key: {e}")),
                                }
                            }
                            Frame::new()
                                .fill(BG)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(10, 7))
                                .show(ui, |ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut self.key_input)
                                            .password(true)
                                            .hint_text(RichText::new("Paste your API key").color(FAINT))
                                            .font(TextStyle::Monospace)
                                            .frame(Frame::NONE)
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                        },
                    );
                });
        });
    }

    fn input_section(&mut self, ui: &mut egui::Ui) {
        let count = self.parsed_links().len();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Links").font(semibold(13.0)).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = match count {
                    0 => String::from("One link per line"),
                    1 => String::from("1 link"),
                    n => format!("{n} links"),
                };
                ui.label(RichText::new(text).size(12.5).color(FAINT));
            });
        });
        ui.add_space(6.0);

        let focused = ui.memory(|m| m.focused()) == Some(Id::unique("links_input"));
        let focus_t = ui.ctx().animate_bool_with_time(Id::unique("input_focus"), focused, 0.15);
        let stroke = Stroke::new(1.0, lerp_color(BORDER, Color32::from_rgb(0x4a, 0x4a, 0x52), focus_t));

        Frame::new()
            .fill(SURFACE)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(RADIUS))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("input_scroll").max_height(150.0).show(ui, |ui| {
                    ui.add_enabled(
                        !self.running(),
                        TextEdit::multiline(&mut self.input)
                            .id(Id::unique("links_input"))
                            .hint_text(RichText::new("https://linkvertise.com/…\nhttps://lootlabs.gg/…").color(FAINT))
                            .font(TextStyle::Monospace)
                            .frame(Frame::NONE)
                            .desired_rows(7)
                            .desired_width(f32::INFINITY)
                            .lock_focus(true),
                    );
                });
            });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let count = self.parsed_links().len();
        let running = self.running();
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 34.0), Layout::left_to_right(Align::Center), |ui| {
            toggle(ui, &mut self.refresh, "Refresh mode", "Skip cached results. Use only for links that change often.");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if running { String::from("Bypassing…") } else { String::from("Bypass") };
                if button(ui, &label, true, count > 0 && !running).clicked() {
                    self.start(ui.ctx());
                }
                if button(ui, "Clear", false, !running && (!self.input.is_empty() || !self.rows.is_empty())).clicked()
                {
                    self.input.clear();
                    self.rows.clear();
                    self.saved = None;
                    self.save_error = None;
                }
                ui.label(RichText::new("Ctrl + Enter").size(12.0).color(FAINT));
            });
        });
    }

    fn results(&mut self, ui: &mut egui::Ui) {
        let total = self.rows.len();
        let ok = self.successes().len();
        let failed = self.rows.iter().filter(|r| matches!(r.state, State::Err(_))).count();
        let done = ok + failed;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Results").font(semibold(13.0)).color(TEXT));
            if total > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if failed > 0 {
                        ui.label(RichText::new(format!("{failed} failed")).size(12.5).color(ERR));
                        ui.label(RichText::new("·").size(12.5).color(FAINT));
                    }
                    ui.label(RichText::new(format!("{done} / {total}")).size(12.5).color(MUTED));
                });
            }
        });
        ui.add_space(8.0);

        let progress = if total == 0 { 0.0 } else { done as f32 / total as f32 };
        progress_bar(ui, progress, total > 0);
        ui.add_space(10.0);

        let footer_height = 40.0;
        let list_height = (ui.available_height() - footer_height - 12.0).max(80.0);
        let now = ui.input(|i| i.time);
        let mut copied: Option<String> = None;

        Frame::new()
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(RADIUS))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_height(list_height);
                ui.set_width(ui.available_width());
                if self.rows.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Bypassed links will appear here").color(FAINT));
                    });
                    return;
                }
                egui::ScrollArea::vertical().id_salt("results_scroll").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, row) in self.rows.iter().enumerate() {
                        let t = ((now - row.shown_at) as f32 / ROW_FADE).clamp(0.0, 1.0);
                        if t < 1.0 {
                            ui.ctx().request_repaint();
                        }
                        if let Some(url) = result_row(ui, i, row, ease_out(t)) {
                            copied = Some(url);
                        }
                    }
                });
            });

        if let Some(url) = copied {
            ui.ctx().copy_text(url);
            self.show_toast(ui.ctx(), String::from("Link copied"));
        }

        ui.add_space(12.0);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 34.0), Layout::left_to_right(Align::Center), |ui| {
            if let Some(err) = &self.save_error {
                ui.label(RichText::new(err).size(12.5).color(ERR));
            } else if let Some(path) = &self.saved {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("Saved to results\\{name}")).size(12.5).color(MUTED));
            } else {
                ui.label(RichText::new("Results are saved to the results folder").size(12.5).color(FAINT));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if ok > 0 { format!("Copy all ({ok})") } else { String::from("Copy all") };
                if button(ui, &label, true, ok > 0).clicked() {
                    self.copy_all(ui.ctx());
                }
                if button(ui, "Open folder", false, true).clicked() {
                    let dir = results_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            });
        });
    }

    fn paint_toast(&mut self, ctx: &egui::Context) {
        let Some((text, started)) = &self.toast else { return };
        let elapsed = ctx.input(|i| i.time) - started;
        if elapsed > TOAST_TIME {
            self.toast = None;
            return;
        }
        ctx.request_repaint();
        let fade = 0.18;
        let alpha = if elapsed < fade {
            ease_out((elapsed / fade) as f32)
        } else if elapsed > TOAST_TIME - fade {
            ease_out(((TOAST_TIME - elapsed) / fade) as f32)
        } else {
            1.0
        };

        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("toast")));
        let galley = painter.layout_no_wrap(text.clone(), FontId::proportional(13.5), TEXT.gamma_multiply(alpha));
        let screen = ctx.content_rect();
        let size = galley.size() + Vec2::new(32.0, 18.0);
        let center = Pos2::new(screen.center().x, screen.bottom() - 84.0 + (1.0 - alpha) * 8.0);
        let rect = Rect::from_center_size(center, size);
        painter.rect(
            rect,
            CornerRadius::same(255),
            Color32::from_rgb(0x26, 0x26, 0x2b).gamma_multiply(alpha),
            Stroke::new(1.0, Color32::from_rgb(0x38, 0x38, 0x3e).gamma_multiply(alpha)),
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, TEXT);
    }
}

// ---------------------------------------------------------------------------
// Widgets

/// Draws one result row. Returns the bypassed URL if the row was clicked.
fn result_row(ui: &mut egui::Ui, index: usize, row: &Row, appear: f32) -> Option<String> {
    let width = ui.available_width();
    let height = 50.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let clickable = matches!(row.state, State::Ok(_));
    let hover_t = ui.ctx().animate_bool_with_time(Id::unique(("row_hover", index)), response.hovered() && clickable, 0.12);
    let done_t = ui.ctx().animate_bool_with_time(Id::unique(("row_done", index)), !matches!(row.state, State::Pending), 0.3);

    let painter = ui.painter_at(rect);
    let rect = rect.translate(Vec2::new((1.0 - appear) * 10.0, 0.0));
    let a = appear;

    painter.rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), CornerRadius::same(6), SURFACE_HOVER.gamma_multiply(hover_t * a));
    if index > 0 {
        painter.hline(rect.x_range().shrink(12.0), rect.top(), Stroke::new(1.0, BORDER.gamma_multiply(0.6 * a)));
    }

    // Status indicator
    let dot = Pos2::new(rect.left() + 20.0, rect.center().y);
    match &row.state {
        State::Pending => {
            let t = ui.input(|i| i.time) as f32;
            let start = t * 5.0;
            let points: Vec<Pos2> = (0..=24)
                .map(|k| {
                    let ang = start + k as f32 / 24.0 * std::f32::consts::PI * 1.4;
                    dot + Vec2::angled(ang) * 6.0
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(1.6, MUTED.gamma_multiply(a))));
            ui.ctx().request_repaint();
        }
        State::Ok(_) => {
            painter.circle_filled(dot, 3.0 + 1.5 * done_t, OK.gamma_multiply(a));
            painter.circle_stroke(dot, 4.5 + 4.0 * (1.0 - done_t), Stroke::new(1.0, OK.gamma_multiply((1.0 - done_t) * a)));
        }
        State::Err(_) => {
            painter.circle_filled(dot, 3.0 + 1.5 * done_t, ERR.gamma_multiply(a));
        }
    }

    let text_left = rect.left() + 40.0;
    let text_width = (rect.right() - text_left - 70.0).max(40.0);
    let (primary, primary_color, primary_font) = match &row.state {
        State::Pending => (row.link.as_str(), MUTED, FontId::monospace(12.5)),
        State::Ok(url) => (url.as_str(), lerp_color(MUTED, TEXT, done_t), FontId::monospace(12.5)),
        State::Err(e) => (e.as_str(), lerp_color(MUTED, ERR, done_t), FontId::proportional(13.0)),
    };
    let secondary = match &row.state {
        State::Pending => "Resolving…",
        _ => row.link.as_str(),
    };

    let line = |text: &str, font: FontId, color: Color32, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color.gamma_multiply(a));
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        let galley = painter.layout_job(job);
        painter.galley(Pos2::new(text_left, y - galley.size().y / 2.0), galley, color);
    };
    line(primary, primary_font, primary_color, rect.top() + 18.0);
    line(secondary, FontId::proportional(11.5), FAINT, rect.top() + 34.0);

    if clickable {
        let hint_color = MUTED.gamma_multiply(hover_t * a);
        painter.text(rect.right_center() - Vec2::new(16.0, 0.0), Align2::RIGHT_CENTER, "Copy", FontId::proportional(12.0), hint_color);
    }

    if clickable && response.clicked() {
        if let State::Ok(url) = &row.state {
            return Some(url.clone());
        }
    }
    if clickable {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    None
}

fn button(ui: &mut egui::Ui, text: &str, primary: bool, enabled: bool) -> egui::Response {
    let font = semibold(13.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), TEXT);
    let size = Vec2::new(galley.size().x + 32.0, 34.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    let id = response.id;
    let hover = ui.ctx().animate_bool_with_time(id.with("hover"), enabled && response.hovered(), 0.12);
    let press = ui.ctx().animate_bool_with_time(id.with("press"), enabled && response.is_pointer_button_down_on(), 0.08);
    let enabled_t = ui.ctx().animate_bool_with_time(id.with("enabled"), enabled, 0.2);

    let (fill, text_color, stroke) = if primary {
        (lerp_color(TEXT, PRIMARY_HOVER, hover), BG, Color32::TRANSPARENT)
    } else {
        (lerp_color(CONTROL, CONTROL_HOVER, hover), TEXT, BORDER)
    };
    let opacity = 0.35 + 0.65 * enabled_t;
    let rect = rect.shrink(press * 1.0);

    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(7),
        fill.gamma_multiply(opacity),
        Stroke::new(1.0, stroke.gamma_multiply(opacity)),
        StrokeKind::Inside,
    );
    let text_color = if primary { text_color.gamma_multiply(opacity.max(0.6)) } else { lerp_color(FAINT, text_color, enabled_t) };
    let galley = painter.layout_no_wrap(text.to_string(), font, text_color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, tooltip: &str) {
    let size = Vec2::new(32.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.16);
    let t = ease_out(t);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(255),
        lerp_color(CONTROL, TEXT, t),
        Stroke::new(1.0, lerp_color(BORDER, TEXT, t)),
        StrokeKind::Inside,
    );
    let radius = rect.height() / 2.0 - 3.0;
    let x = egui::lerp((rect.left() + radius + 3.0)..=(rect.right() - radius - 3.0), t);
    painter.circle_filled(Pos2::new(x, rect.center().y), radius, lerp_color(MUTED, BG, t));

    let label_response = ui.add(egui::Label::new(RichText::new(label).color(lerp_color(MUTED, TEXT, t))).sense(Sense::click()));
    if label_response.clicked() {
        *on = !*on;
    }
    response.union(label_response).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip);
}

fn progress_bar(ui: &mut egui::Ui, progress: f32, visible: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
    let shown = ui.ctx().animate_bool_with_time(Id::unique("progress_visible"), visible, 0.25);
    let value = ui.ctx().animate_value_with_time(Id::unique("progress_value"), progress, 0.35);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), BORDER.gamma_multiply(shown));
    if value > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * value, rect.height()));
        let color = if progress >= 1.0 { lerp_color(TEXT, OK, ui.ctx().animate_bool_with_time(Id::unique("progress_done"), true, 0.4)) } else { TEXT };
        painter.rect_filled(fill, CornerRadius::same(2), color.gamma_multiply(shown));
    }
    if progress < 1.0 {
        ui.ctx().animate_bool_with_time(Id::unique("progress_done"), false, 0.0);
    }
}

// ---------------------------------------------------------------------------
// Setup & helpers

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn results_dir() -> PathBuf {
    exe_dir().join("results")
}

/// Reads the key from `BYPASS_VIP_API_KEY`, falling back to `api_key.txt` next to the exe.
fn load_api_key() -> String {
    // Notepad can save with a UTF-8 BOM, which would make the header value invalid.
    let clean = |k: String| k.trim().trim_start_matches('\u{feff}').trim().to_string();
    std::env::var("BYPASS_VIP_API_KEY")
        .ok()
        .map(clean)
        .filter(|k| !k.is_empty())
        .or_else(|| std::fs::read_to_string(exe_dir().join("api_key.txt")).ok().map(clean))
        .unwrap_or_default()
}

fn save_api_key(key: &str) -> std::io::Result<()> {
    std::fs::write(exe_dir().join("api_key.txt"), key)
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| String::from("C:\\Windows"));
    let load = |file: &str| std::fs::read(PathBuf::from(&windir).join("Fonts").join(file)).ok();

    let add = |fonts: &mut FontDefinitions, name: &str, files: &[&str], family: FontFamily| {
        if let Some(bytes) = files.iter().find_map(|f| load(f)) {
            fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.to_string());
        }
    };
    add(&mut fonts, "ui", &["segoeui.ttf"], FontFamily::Proportional);
    add(&mut fonts, "mono", &["CascadiaMono.ttf", "consola.ttf"], FontFamily::Monospace);

    let semibold_family = FontFamily::Name("semibold".into());
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(semibold_family.clone(), fallback);
    add(&mut fonts, "ui-semibold", &["seguisb.ttf"], semibold_family);

    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(19.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.extreme_bg_color = SURFACE;
        v.override_text_color = None;
        v.selection.bg_fill = Color32::from_rgb(0x3a, 0x3f, 0x4b);
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.text_cursor.stroke = Stroke::new(1.5, TEXT);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.inactive.bg_fill = CONTROL;
        v.widgets.inactive.weak_bg_fill = CONTROL;
        v.widgets.hovered.bg_fill = CONTROL_HOVER;
        v.widgets.hovered.weak_bg_fill = CONTROL_HOVER;
        v.widgets.active.bg_fill = CONTROL_HOVER;
        v.window_corner_radius = CornerRadius::same(RADIUS);
    });
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}
