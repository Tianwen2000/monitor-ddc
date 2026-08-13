//! egui application state and graphical interface.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::ddc::{self, Feature, Monitor};

const WRITE_INTERVAL: Duration = Duration::from_millis(40);
const FALLBACK_MAXIMUM: u32 = 100;
const FALLBACK_VALUE: u8 = 50;
const ACCENT: egui::Color32 = egui::Color32::from_rgb(31, 120, 180);

pub fn run() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/MonitorDDC.png"))
        .expect("embedded application icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MonitorDDC 显示器调节")
            .with_icon(icon)
            .with_inner_size([760.0, 590.0])
            .with_min_inner_size([620.0, 430.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MonitorDDC 显示器调节",
        options,
        Box::new(|context| {
            configure_style(&context.egui_ctx);
            Ok(Box::new(DdcApp::new()))
        }),
    )
}

fn configure_style(context: &egui::Context) {
    install_chinese_font(context);

    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*context.style_of(theme)).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.interact_size.y = 32.0;
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.visuals.widgets.hovered.expansion = 0.0;
        style.visuals.panel_fill = match theme {
            egui::Theme::Dark => egui::Color32::from_rgb(25, 28, 32),
            egui::Theme::Light => egui::Color32::from_rgb(247, 248, 250),
        };
        context.set_style_of(theme, style);
    }
}

fn install_chinese_font(context: &egui::Context) {
    let font_data = [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simhei.ttf"]
        .iter()
        .find_map(|path| std::fs::read(path).ok());

    let Some(font_data) = font_data else {
        return;
    };

    let font_name = "windows_chinese".to_owned();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        Arc::new(egui::FontData::from_owned(font_data)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.clone());
    }
    context.set_fonts(fonts);
}

struct DdcApp {
    monitors: Vec<MonitorState>,
    selected: Option<usize>,
    status: String,
}

struct MonitorState {
    monitor: Monitor,
    brightness: FeatureState,
    contrast: FeatureState,
}

struct FeatureState {
    value: u8,
    maximum: u32,
    read_error: Option<String>,
    write_error: Option<String>,
    pending: Option<u8>,
    last_write: Instant,
}

impl DdcApp {
    fn new() -> Self {
        let mut app = Self {
            monitors: Vec::new(),
            selected: None,
            status: String::new(),
        };
        app.rescan();
        app
    }

    fn rescan(&mut self) {
        // Replacing the vector drops all old physical handles before a fresh scan.
        self.monitors.clear();
        self.selected = None;

        match ddc::enumerate_monitors() {
            Ok(monitors) => {
                self.monitors = monitors.into_iter().map(MonitorState::load).collect();
                self.selected = (!self.monitors.is_empty()).then_some(0);
                let usable = self
                    .monitors
                    .iter()
                    .filter(|monitor| {
                        monitor.brightness.read_error.is_none()
                            || monitor.contrast.read_error.is_none()
                    })
                    .count();
                self.status = format!(
                    "发现 {} 台物理显示器，其中 {} 台可读取 DDC/CI 参数",
                    self.monitors.len(),
                    usable
                );
            }
            Err(error) => self.status = format!("扫描失败：{error}"),
        }
    }

    fn update_pending_writes(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(state) = self.monitors.get_mut(selected) else {
            return;
        };

        flush_feature(
            &state.monitor,
            Feature::Brightness,
            &mut state.brightness,
            &mut self.status,
        );
        flush_feature(
            &state.monitor,
            Feature::Contrast,
            &mut state.contrast,
            &mut self.status,
        );
    }
}

impl MonitorState {
    fn load(monitor: Monitor) -> Self {
        let brightness = load_feature(&monitor, Feature::Brightness);
        let contrast = load_feature(&monitor, Feature::Contrast);
        Self {
            monitor,
            brightness,
            contrast,
        }
    }

    fn ddc_summary(&self) -> String {
        let brightness = if self.brightness.read_error.is_none() {
            "亮度可读写"
        } else {
            "亮度仅可尝试写入"
        };
        let contrast = if self.contrast.read_error.is_none() {
            "对比度可读写"
        } else {
            "对比度仅可尝试写入"
        };
        format!("{brightness}，{contrast}")
    }
}

fn load_feature(monitor: &Monitor, feature: Feature) -> FeatureState {
    match monitor.read(feature) {
        Ok(value) => FeatureState {
            value: value.percent(),
            maximum: value.maximum,
            read_error: None,
            write_error: None,
            pending: None,
            last_write: Instant::now() - WRITE_INTERVAL,
        },
        Err(error) => FeatureState {
            value: FALLBACK_VALUE,
            maximum: FALLBACK_MAXIMUM,
            read_error: Some(error.to_string()),
            write_error: None,
            pending: None,
            last_write: Instant::now() - WRITE_INTERVAL,
        },
    }
}

fn flush_feature(
    monitor: &Monitor,
    feature: Feature,
    state: &mut FeatureState,
    status: &mut String,
) {
    let Some(value) = state.pending else {
        return;
    };
    if state.last_write.elapsed() < WRITE_INTERVAL {
        return;
    }

    state.pending = None;
    state.last_write = Instant::now();
    match monitor.write_percent_with_max(feature, value, state.maximum) {
        Ok(()) => {
            state.write_error = None;
            *status = format!(
                "显示器 {} 的{}已设置为 {}%",
                monitor.index,
                feature.label(),
                value
            );
        }
        Err(error) => {
            let message = error.to_string();
            state.write_error = Some(message.clone());
            *status = format!(
                "显示器 {} 的{}调节失败：{}",
                monitor.index,
                feature.label(),
                message
            );
        }
    }
}

impl eframe::App for DdcApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_pending_writes();
        if self.monitors.iter().any(|monitor| {
            monitor.brightness.pending.is_some() || monitor.contrast.pending.is_some()
        }) {
            context.request_repaint_after(WRITE_INTERVAL);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").exact_size(64.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading("MonitorDDC");
                ui.label(egui::RichText::new("显示器调节").color(ui.visuals().weak_text_color()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳  重新扫描").clicked() {
                        self.rescan();
                    }
                });
            });
        });

        egui::Panel::left("monitor_list")
            .resizable(false)
            .exact_size(235.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("显示器  {}", self.monitors.len()))
                        .strong()
                        .size(15.0),
                );
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if self.monitors.is_empty() {
                    ui.label("未检测到物理显示器");
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (position, monitor) in self.monitors.iter().enumerate() {
                        let selected = self.selected == Some(position);
                        let title = format!(
                            "{}  {}",
                            monitor.monitor.index,
                            monitor.monitor.identity_name()
                        );
                        let title = if selected {
                            egui::RichText::new(title).color(egui::Color32::WHITE)
                        } else {
                            egui::RichText::new(title)
                        };
                        let response = ui
                            .add_sized(
                                [ui.available_width(), 38.0],
                                egui::Button::selectable(selected, title),
                            )
                            .on_hover_ui(|ui| monitor_tooltip(ui, monitor));
                        if response.clicked() {
                            self.selected = Some(position);
                        }
                        let mode = monitor.monitor.info.mode_text();
                        if mode != "未知" {
                            ui.label(
                                egui::RichText::new(format!("    {mode}"))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }
                        ui.add_space(4.0);
                    }
                });
            });

        egui::Panel::bottom("status")
            .exact_size(40.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.colored_label(ACCENT, "●");
                    ui.label(&self.status);
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    let Some(selected) = self.selected else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(90.0);
                            ui.heading("未选择显示器");
                            ui.label("请连接外接显示器后重新扫描");
                        });
                        return;
                    };
                    let Some(monitor) = self.monitors.get_mut(selected) else {
                        return;
                    };

                    monitor_header(ui, monitor);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(12.0);

                    ui.label(egui::RichText::new("DDC/CI 调节").strong().size(15.0));
                    ui.add_space(6.0);
                    feature_slider(ui, "亮度", &mut monitor.brightness);
                    ui.add_space(10.0);
                    feature_slider(ui, "对比度", &mut monitor.contrast);
                    ui.add_space(16.0);

                    ui.label(egui::RichText::new("显示参数").strong().size(15.0));
                    ui.add_space(4.0);
                    display_details(ui, monitor);
                    ui.add_space(12.0);
                });
        });
    }
}

fn monitor_header(ui: &mut egui::Ui, monitor: &MonitorState) {
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(ACCENT)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("▣")
                        .color(egui::Color32::WHITE)
                        .size(22.0),
                );
            });
        ui.vertical(|ui| {
            ui.heading(monitor.monitor.identity_name());
            let suffix = if monitor.monitor.info.is_primary {
                " · 主显示器"
            } else {
                ""
            };
            ui.label(
                egui::RichText::new(format!("显示器 {}{suffix}", monitor.monitor.index))
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
}

fn display_details(ui: &mut egui::Ui, monitor: &MonitorState) {
    let info = &monitor.monitor.info;
    egui::Grid::new("display_details")
        .num_columns(2)
        .min_col_width(105.0)
        .spacing([18.0, 7.0])
        .show(ui, |ui| {
            detail_row(ui, "分辨率与刷新率", &info.mode_text());
            detail_row(ui, "连接接口", info.connector_text());
            detail_row(ui, "制造商", info.manufacturer_name().unwrap_or("未知"));
            detail_row(ui, "型号", info.friendly_name.as_deref().unwrap_or("未知"));
            detail_row(ui, "产品代码", &info.product_code_text());
            if let Some(bit_depth) = info.bit_depth {
                detail_row(ui, "桌面色深", &format!("{bit_depth} 位"));
            }
            detail_row(ui, "DDC/CI", &monitor.ddc_summary());
        });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).color(ui.visuals().weak_text_color()));
    ui.label(value);
    ui.end_row();
}

fn monitor_tooltip(ui: &mut egui::Ui, monitor: &MonitorState) {
    let info = &monitor.monitor.info;
    ui.set_max_width(430.0);
    ui.strong(monitor.monitor.identity_name());
    ui.separator();
    egui::Grid::new(("monitor_tooltip", monitor.monitor.index))
        .num_columns(2)
        .show(ui, |ui| {
            detail_row(ui, "当前模式", &info.mode_text());
            detail_row(ui, "连接接口", info.connector_text());
            detail_row(
                ui,
                "制造商代码",
                info.manufacturer_code.as_deref().unwrap_or("未知"),
            );
            detail_row(ui, "产品代码", &info.product_code_text());
            if !info.gdi_device_name.is_empty() {
                detail_row(ui, "Windows 设备", &info.gdi_device_name);
            }
            detail_row(ui, "DDC/CI", &monitor.ddc_summary());
        });
    if let Some(path) = &info.device_path {
        ui.separator();
        ui.label(egui::RichText::new("设备路径").small().strong());
        ui.label(egui::RichText::new(path).small().monospace());
    }
}

fn feature_slider(ui: &mut egui::Ui, label: &str, state: &mut FeatureState) {
    ui.horizontal(|ui| {
        ui.add_sized([54.0, 28.0], egui::Label::new(label));
        let available = (ui.available_width() - 58.0).max(160.0);
        let response = ui.add_sized(
            [available, 28.0],
            egui::Slider::new(&mut state.value, 0..=100).suffix("%"),
        );
        if response.changed() {
            // A short throttle preserves real-time feedback without flooding slow DDC buses.
            state.pending = Some(state.value);
        }
    });

    if let Some(error) = &state.read_error {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(64.0);
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("无法读取当前{label}，仍可尝试调节（悬停查看详情）"),
            )
            .on_hover_text(error);
        });
    }
    if let Some(error) = &state.write_error {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(64.0);
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("{label}调节失败（悬停查看详情）"),
            )
            .on_hover_text(error);
        });
    }
}
