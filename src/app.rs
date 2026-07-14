use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, Frame, Id, Layout, Margin, RichText,
    ScrollArea, Sense, Stroke, Ui, Vec2,
};

use crate::config::{Accent, ConfigStore};
use crate::monitor::{InputPreset, MonitorInfo, SwitchOutcome, default_inputs, input_name};
use crate::system::{SystemAction, SystemIntegration};
use crate::theme::{self, Theme};
use crate::worker::{MonitorWorker, WorkerEvent};

pub struct MonitorControlApp {
    worker: MonitorWorker,
    system: SystemIntegration,
    config: ConfigStore,
    monitors: Vec<MonitorInfo>,
    busy_monitors: HashSet<String>,
    toasts: VecDeque<Toast>,
    scanning: bool,
    scan_error: Option<String>,
    has_scanned: bool,
    last_scan: Option<Instant>,
    show_settings: bool,
    quit_requested: bool,
    window_chrome_applied: bool,
    applied_accent: Accent,
    custom_input_label: String,
    custom_input_code: String,
}

impl MonitorControlApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let config = ConfigStore::load();
        theme::install(&creation_context.egui_ctx, config.data.accent);

        let mut system = SystemIntegration::new(config.data.global_hotkeys);
        let system_warning = system.take_warning();
        let worker = MonitorWorker::start(creation_context.egui_ctx.clone());
        worker.scan();

        let mut app = Self {
            worker,
            system,
            applied_accent: config.data.accent,
            config,
            monitors: Vec::new(),
            busy_monitors: HashSet::new(),
            toasts: VecDeque::new(),
            scanning: true,
            scan_error: None,
            has_scanned: false,
            last_scan: None,
            show_settings: false,
            quit_requested: false,
            window_chrome_applied: false,
            custom_input_label: String::new(),
            custom_input_code: String::new(),
        };

        if let Some(warning) = app.config.load_warning.take() {
            app.toast(ToastKind::Warning, warning);
        }
        if let Some(warning) = system_warning {
            app.toast(ToastKind::Warning, warning);
        }

        app
    }

    fn request_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.scan_error = None;
        self.worker.scan();
    }

    fn process_worker_events(&mut self) {
        let events: Vec<_> = self.worker.events().collect();
        for event in events {
            match event {
                WorkerEvent::ScanFinished(Ok(monitors)) => {
                    self.scanning = false;
                    self.scan_error = None;
                    self.last_scan = Some(Instant::now());

                    let primary_exists = self
                        .config
                        .data
                        .primary_monitor_id
                        .as_ref()
                        .is_some_and(|id| monitors.iter().any(|monitor| &monitor.id == id));
                    if !primary_exists {
                        self.config.data.primary_monitor_id =
                            monitors.first().map(|monitor| monitor.id.clone());
                        self.save_config();
                    }

                    let count = monitors.len();
                    self.monitors = monitors;
                    if self.has_scanned {
                        let message = if count == 1 {
                            "1 monitor refreshed".to_owned()
                        } else {
                            format!("{count} monitors refreshed")
                        };
                        self.toast(ToastKind::Success, message);
                    }
                    self.has_scanned = true;
                }
                WorkerEvent::ScanFinished(Err(error)) => {
                    self.scanning = false;
                    self.has_scanned = true;
                    self.scan_error = Some(error.clone());
                    self.toast(ToastKind::Error, error);
                }
                WorkerEvent::SwitchFinished {
                    monitor_id,
                    input,
                    result,
                } => {
                    self.busy_monitors.remove(&monitor_id);
                    let input_label = self
                        .monitors
                        .iter()
                        .find(|monitor| monitor.id == monitor_id)
                        .map(|monitor| input_name_for_model(input, monitor.model.as_deref()))
                        .unwrap_or_else(|| input_name(input));
                    match result {
                        Ok(SwitchOutcome::Verified) => {
                            if let Some(monitor) = self
                                .monitors
                                .iter_mut()
                                .find(|monitor| monitor.id == monitor_id)
                            {
                                monitor.current_input = Some(input);
                            }
                            self.toast(
                                ToastKind::Success,
                                format!("Verified switch to {input_label}"),
                            );
                        }
                        Ok(SwitchOutcome::CommandAccepted) => {
                            if let Some(monitor) = self
                                .monitors
                                .iter_mut()
                                .find(|monitor| monitor.id == monitor_id)
                            {
                                monitor.current_input = None;
                            }
                            self.toast(
                                ToastKind::Info,
                                format!(
                                    "{input_label} command accepted; the monitor stopped responding before verification"
                                ),
                            );
                        }
                        Ok(SwitchOutcome::AlternateCommandSent) => {
                            if let Some(monitor) = self
                                .monitors
                                .iter_mut()
                                .find(|monitor| monitor.id == monitor_id)
                            {
                                monitor.current_input = None;
                            }
                            self.toast(
                                ToastKind::Success,
                                format!(
                                    "Sent LG alternate command for {input_label}; this private channel cannot be read back"
                                ),
                            );
                        }
                        Err(error) => self.toast(ToastKind::Error, error),
                    }
                }
            }
        }
    }

    fn process_system_actions(&mut self, context: &egui::Context, frame: &eframe::Frame) {
        for action in self.system.poll_actions() {
            match action {
                SystemAction::ShowWindow => self.show_window(context),
                SystemAction::ToggleWindow => {
                    if crate::system::is_window_visible(frame) {
                        self.hide_window(context);
                    } else {
                        self.show_window(context);
                    }
                }
                SystemAction::Refresh => {
                    self.show_window(context);
                    self.request_scan();
                }
                SystemAction::Quit => {
                    self.quit_requested = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                SystemAction::SwitchSlot(index) => {
                    self.switch_primary_slot(index);
                }
            }
        }
    }

    fn process_keyboard_shortcuts(&mut self, context: &egui::Context) {
        if context.egui_wants_keyboard_input() {
            return;
        }

        if context.input(|input| input.key_pressed(egui::Key::R)) {
            self.request_scan();
        }
        if context.input(|input| input.key_pressed(egui::Key::S)) {
            self.show_settings = !self.show_settings;
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.show_settings = false;
        }

        let key_index = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
        ]
        .into_iter()
        .position(|key| context.input(|input| input.key_pressed(key)));

        if let Some(index) = key_index {
            self.switch_primary_slot(index);
        }
    }

    fn switch_primary_slot(&mut self, index: usize) {
        let Some(monitor) = self
            .primary_monitor_id()
            .and_then(|monitor_id| {
                self.monitors
                    .iter()
                    .find(|monitor| monitor.id == monitor_id)
            })
            .cloned()
        else {
            self.toast(ToastKind::Warning, "No monitor is available yet");
            return;
        };
        let Some(input) = self.inputs_for_monitor(&monitor).get(index).cloned() else {
            self.toast(ToastKind::Warning, "That input shortcut is not configured");
            return;
        };
        self.begin_switch(&monitor.id, input.value);
    }

    fn inputs_for_monitor(&self, monitor: &MonitorInfo) -> Vec<InputPreset> {
        available_inputs(
            self.config.data.inputs_for(&monitor.id),
            monitor.supported_inputs.as_deref(),
            monitor.model.as_deref(),
        )
    }

    fn begin_switch(&mut self, monitor_id: &str, input: u32) {
        if self.busy_monitors.contains(monitor_id) {
            return;
        }

        let Some(monitor) = self
            .monitors
            .iter()
            .find(|monitor| monitor.id == monitor_id)
        else {
            self.toast(
                ToastKind::Error,
                "The selected monitor is no longer available",
            );
            return;
        };
        if !monitor.ddc_supported {
            self.toast(
                ToastKind::Error,
                "This monitor did not respond to DDC/CI. Enable DDC/CI in its on-screen menu.",
            );
            return;
        }
        let input_label = input_name_for_model(input, monitor.model.as_deref());
        if monitor.current_input == Some(input) {
            self.toast(ToastKind::Info, format!("Already using {input_label}"));
            return;
        }

        self.config.data.primary_monitor_id = Some(monitor_id.to_owned());
        self.save_config();
        self.busy_monitors.insert(monitor_id.to_owned());
        self.worker.switch_input(monitor_id, input);
        self.toast(ToastKind::Info, format!("Switching to {input_label}…"));
    }

    fn primary_monitor_id(&self) -> Option<&str> {
        self.config
            .data
            .primary_monitor_id
            .as_deref()
            .or_else(|| self.monitors.first().map(|monitor| monitor.id.as_str()))
    }

    fn set_primary(&mut self, monitor_id: &str) {
        if self.config.data.primary_monitor_id.as_deref() == Some(monitor_id) {
            return;
        }
        self.config.data.primary_monitor_id = Some(monitor_id.to_owned());
        self.save_config();
        self.toast(ToastKind::Success, "Primary monitor updated");
    }

    fn save_config(&mut self) {
        if let Err(error) = self.config.save() {
            self.toast(ToastKind::Error, error.to_string());
        }
    }

    fn hide_window(&mut self, context: &egui::Context) {
        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn show_window(&mut self, context: &egui::Context) {
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        if self.toasts.len() >= 5 {
            self.toasts.pop_front();
        }
        self.toasts.push_back(Toast {
            kind,
            message: message.into(),
            created_at: Instant::now(),
        });
    }

    fn show_settings_panel(&mut self, root_ui: &mut Ui) {
        let context = root_ui.ctx().clone();
        let theme = Theme::for_accent(self.config.data.accent);
        let mut needs_save = false;
        let mut hotkeys_changed = None;
        let mut validation_message = None;
        let previous_accent = self.config.data.accent;
        let primary_id = self.primary_monitor_id().map(str::to_owned);

        egui::Panel::right("settings-panel")
            .exact_size(350.0)
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(16, 19, 33))
                    .stroke(Stroke::new(1.0, theme.border)),
            )
            .show(root_ui, |ui| {
                let content_rect = ui.max_rect().shrink2(Vec2::new(22.0, 20.0));
                let content_width = content_rect.width();
                let mut content_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(Layout::top_down(Align::Min)),
                );
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(&mut content_ui, |ui| {
                        ui.set_width(content_width);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Settings").size(24.0).strong());
                        ui.label(
                            RichText::new("Make the switcher feel like yours")
                                .size(12.0)
                                .color(theme.muted),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                Button::new(RichText::new("Close").color(theme.muted))
                                    .fill(theme.surface)
                                    .corner_radius(10),
                            )
                            .clicked()
                        {
                            self.show_settings = false;
                        }
                    });
                });

                ui.add_space(18.0);
                section_label(ui, "BEHAVIOR", theme);
                needs_save |= ui
                    .checkbox(&mut self.config.data.close_to_tray, "Close window to system tray")
                    .changed();

                let response = ui.checkbox(
                    &mut self.config.data.global_hotkeys,
                    "Enable global shortcuts",
                );
                if response.changed() {
                    hotkeys_changed = Some(self.config.data.global_hotkeys);
                    needs_save = true;
                }
                ui.label(
                    RichText::new("Ctrl+Alt+M opens the app; Ctrl+Alt+1…4 switch inputs.")
                        .size(11.0)
                        .color(theme.muted),
                );

                needs_save |= ui
                    .checkbox(&mut self.config.data.compact_cards, "Use compact monitor cards")
                    .changed();

                ui.add_space(18.0);
                section_label(ui, "ACCENT", theme);
                ui.horizontal_wrapped(|ui| {
                    for (accent, label) in [
                        (Accent::Violet, "Violet"),
                        (Accent::Cyan, "Cyan"),
                        (Accent::Rose, "Rose"),
                    ] {
                        let selected = self.config.data.accent == accent;
                        if ui
                            .add(
                                Button::new(label)
                                    .fill(if selected { theme.accent } else { theme.elevated })
                                    .stroke(Stroke::new(
                                        1.0,
                                        if selected { theme.accent_bright } else { theme.border },
                                    ))
                                    .corner_radius(10),
                            )
                            .clicked()
                        {
                            self.config.data.accent = accent;
                            needs_save = true;
                        }
                    }
                });

                ui.add_space(18.0);
                section_label(ui, "PRIMARY MONITOR", theme);
                for monitor in self.monitors.clone() {
                    let selected = primary_id.as_deref() == Some(monitor.id.as_str());
                    let detected_name = monitor
                        .model
                        .as_deref()
                        .map(|model| format!("LG {model}"))
                        .unwrap_or_else(|| monitor.description.clone());
                    let display_name = self
                        .config
                        .data
                        .alias_for(&monitor.id, &detected_name)
                        .to_owned();
                    if ui
                        .add(
                            Button::new(
                                RichText::new(if selected {
                                    format!("●  {display_name}")
                                } else {
                                    display_name
                                })
                                .color(if selected { theme.text } else { theme.muted }),
                            )
                            .fill(if selected { theme.accent_soft } else { theme.surface })
                            .stroke(Stroke::new(
                                1.0,
                                if selected { theme.accent } else { theme.border },
                            ))
                            .corner_radius(10)
                            .min_size(Vec2::new(ui.available_width(), 38.0)),
                        )
                        .clicked()
                    {
                        self.config.data.primary_monitor_id = Some(monitor.id);
                        needs_save = true;
                    }
                }

                if let Some(monitor_id) = primary_id {
                    ui.add_space(18.0);
                    section_label(ui, "MONITOR PROFILE", theme);
                    let preferences = self.config.data.preferences_mut(&monitor_id);

                    ui.label(RichText::new("Display name").size(12.0).color(theme.muted));
                    needs_save |= ui
                        .add(
                            egui::TextEdit::singleline(&mut preferences.alias)
                                .hint_text("e.g. Main OLED")
                                .desired_width(f32::INFINITY),
                        )
                        .changed();

                    ui.add_space(10.0);
                    ui.label(RichText::new("Input presets").size(12.0).color(theme.muted));
                    let mut remove_index = None;
                    for (index, input) in preferences.inputs.iter_mut().enumerate() {
                        Frame::new()
                            .fill(theme.surface)
                            .stroke(Stroke::new(1.0, theme.border))
                            .corner_radius(10)
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    needs_save |= ui
                                        .checkbox(&mut input.favorite, "")
                                        .on_hover_text("Favorite")
                                        .changed();
                                    needs_save |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut input.label)
                                                .desired_width(145.0),
                                        )
                                        .changed();
                                    ui.label(
                                        RichText::new(format!("0x{:02X}", input.value))
                                            .monospace()
                                            .size(11.0)
                                            .color(theme.muted),
                                    );
                                    if ui.small_button("Remove").clicked() {
                                        remove_index = Some(index);
                                    }
                                });
                            });
                    }
                    if let Some(index) = remove_index {
                        preferences.inputs.remove(index);
                        needs_save = true;
                    }

                    ui.add_space(8.0);
                    Frame::new()
                        .fill(theme.accent_soft)
                        .stroke(Stroke::new(1.0, theme.accent))
                        .corner_radius(12)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(RichText::new("Add a custom input").strong().size(12.0));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.custom_input_label)
                                        .hint_text("Label")
                                        .desired_width(140.0),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.custom_input_code)
                                        .hint_text("0x11")
                                        .desired_width(72.0),
                                );
                                if ui.button("Add").clicked() {
                                    match parse_input_code(&self.custom_input_code) {
                                        Some(value)
                                            if !self.custom_input_label.trim().is_empty()
                                                && !preferences
                                                    .inputs
                                                    .iter()
                                                    .any(|input| input.value == value) =>
                                        {
                                            preferences.inputs.push(InputPreset::new(
                                                value,
                                                self.custom_input_label.trim(),
                                                false,
                                            ));
                                            self.custom_input_label.clear();
                                            self.custom_input_code.clear();
                                            needs_save = true;
                                        }
                                        Some(_) => {
                                            validation_message = Some(
                                                "Enter a unique code and a non-empty label."
                                                    .to_owned(),
                                            );
                                        }
                                        None => {
                                            validation_message = Some(
                                                "Input code must be decimal or hexadecimal, such as 0x11."
                                                    .to_owned(),
                                            );
                                        }
                                    }
                                }
                            });
                        });
                }

                                ui.add_space(18.0);
                                section_label(ui, "SETTINGS FILE", theme);
                                ui.label(
                                    RichText::new(self.config.path().display().to_string())
                                        .monospace()
                                        .size(10.0)
                                        .color(theme.muted),
                                );
                            });
            });

        if previous_accent != self.config.data.accent {
            self.applied_accent = self.config.data.accent;
            theme::install(&context, self.config.data.accent);
        }
        if let Some(enabled) = hotkeys_changed
            && let Err(error) = self.system.set_hotkeys_enabled(enabled)
        {
            validation_message = Some(error);
        }
        if needs_save {
            self.save_config();
        }
        if let Some(message) = validation_message {
            self.toast(ToastKind::Warning, message);
        }
    }

    fn show_main_panel(&mut self, root_ui: &mut Ui) {
        let theme = Theme::for_accent(self.config.data.accent);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme.background))
            .show(root_ui, |ui| {
                paint_backdrop(ui, theme);
                let content_width = (ui.available_width() - 48.0).max(1.0);
                let content_height = (ui.available_height() - 36.0).max(1.0);
                Frame::new()
                    .inner_margin(Margin::symmetric(24, 18))
                    .show(ui, |ui| {
                        ui.set_width(content_width);
                        ui.set_height(content_height);
                        self.show_top_bar(ui, theme);
                        ui.add_space(18.0);

                        let content_width = ui.available_width();
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(content_width);
                                if self.scanning && self.monitors.is_empty() {
                                    self.show_loading_state(ui, theme);
                                } else if self.monitors.is_empty() {
                                    self.show_empty_state(ui, theme);
                                } else {
                                    let monitors = self.monitors.clone();
                                    for monitor in monitors {
                                        self.show_monitor_card(ui, theme, &monitor);
                                        ui.add_space(14.0);
                                    }
                                }
                                ui.add_space(10.0);
                                self.show_footer(ui, theme);
                                ui.add_space(20.0);
                            });
                    });
            });
    }

    fn show_top_bar(&mut self, ui: &mut Ui, theme: Theme) {
        let total_width = ui.available_width();
        let actions_width = 330.0_f32.min(total_width * 0.46);
        let title_width = (total_width - actions_width).max(280.0);

        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(title_width, 60.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    paint_brand_mark(ui, theme);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("LG Monitor Control").size(25.0).strong());
                        ui.label(
                            RichText::new("Switch inputs without touching the monitor")
                                .size(12.0)
                                .color(theme.muted),
                        );
                    });
                },
            );

            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 60.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    if ui
                        .add(
                            Button::new("Settings")
                                .fill(if self.show_settings {
                                    theme.accent_soft
                                } else {
                                    theme.surface
                                })
                                .stroke(Stroke::new(1.0, theme.border))
                                .corner_radius(12),
                        )
                        .on_hover_text("Settings (S)")
                        .clicked()
                    {
                        self.show_settings = !self.show_settings;
                    }

                    let refresh = ui
                        .add_enabled(
                            !self.scanning,
                            Button::new(if self.scanning {
                                "Scanning…"
                            } else {
                                "Refresh"
                            })
                            .fill(theme.surface)
                            .stroke(Stroke::new(1.0, theme.border))
                            .corner_radius(12),
                        )
                        .on_hover_text("Refresh monitors (R)");
                    if refresh.clicked() {
                        self.request_scan();
                    }

                    let count_text = match self.monitors.len() {
                        0 => "No monitors".to_owned(),
                        1 => "1 monitor".to_owned(),
                        count => format!("{count} monitors"),
                    };
                    Frame::new()
                        .fill(theme.accent_soft)
                        .stroke(Stroke::new(1.0, theme.accent))
                        .corner_radius(20)
                        .inner_margin(Margin::symmetric(12, 7))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(count_text)
                                    .size(11.0)
                                    .color(theme.accent_bright),
                            );
                        });
                },
            );
        });
    }

    fn show_monitor_card(&mut self, ui: &mut Ui, theme: Theme, monitor: &MonitorInfo) {
        let primary = self.primary_monitor_id() == Some(monitor.id.as_str());
        let busy = self.busy_monitors.contains(&monitor.id);
        let compact = self.config.data.compact_cards;
        let detected_name = monitor
            .model
            .as_deref()
            .map(|model| format!("LG {model}"))
            .unwrap_or_else(|| monitor.description.clone());
        let display_name = self
            .config
            .data
            .alias_for(&monitor.id, &detected_name)
            .to_owned();
        let current_input = monitor
            .current_input
            .map(input_name)
            .unwrap_or_else(|| "Unavailable".into());
        let inputs = self.inputs_for_monitor(monitor);

        Frame::new()
            .fill(theme.surface)
            .stroke(Stroke::new(
                if primary { 1.5 } else { 1.0 },
                if primary { theme.accent } else { theme.border },
            ))
            .corner_radius(22)
            .inner_margin(Margin::same(if compact { 16 } else { 22 }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    paint_monitor_icon(ui, theme, monitor.ddc_supported);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&display_name).size(19.0).strong());
                            if primary {
                                status_chip(ui, "PRIMARY", theme.accent_bright, theme.accent_soft);
                            }
                            status_chip(
                                ui,
                                if monitor.ddc_supported { "DDC/CI READY" } else { "NO DDC/CI" },
                                if monitor.ddc_supported { theme.success } else { theme.danger },
                                if monitor.ddc_supported {
                                    theme::with_alpha(theme.success, 24)
                                } else {
                                    theme::with_alpha(theme.danger, 24)
                                },
                            );
                        });
                        ui.label(
                            RichText::new(format!("{}  ·  {}", monitor.device_name, monitor.description))
                                .size(11.0)
                                .color(theme.muted),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if busy {
                            ui.spinner();
                        } else if !primary
                            && ui
                                .add(
                                    Button::new("Make primary")
                                        .fill(theme.elevated)
                                        .stroke(Stroke::new(1.0, theme.border))
                                        .corner_radius(10),
                                )
                                .clicked()
                        {
                            self.set_primary(&monitor.id);
                        }

                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("CURRENT INPUT")
                                    .size(9.0)
                                    .color(theme.muted),
                            );
                            let vcp_details = match (monitor.current_input, monitor.maximum_input) {
                                (Some(current), Some(maximum)) => format!(
                                    "VCP 0x60 · current 0x{current:02X} · reported maximum 0x{maximum:02X}"
                                ),
                                (Some(current), None) => {
                                    format!("VCP 0x60 · current 0x{current:02X}")
                                }
                                _ => "VCP 0x60 did not return a readable value".to_owned(),
                            };
                            ui.label(
                                RichText::new(&current_input)
                                    .size(15.0)
                                    .strong()
                                    .color(if monitor.ddc_supported {
                                        theme.text
                                    } else {
                                        theme.muted
                                    }),
                            )
                            .on_hover_text(vcp_details);
                        });
                    });
                });

                if !compact {
                    ui.add_space(18.0);
                } else {
                    ui.add_space(10.0);
                }
                ui.label(
                    RichText::new("CHOOSE AN INPUT")
                        .size(10.0)
                        .color(theme.muted),
                );
                ui.add_space(4.0);

                ui.horizontal_wrapped(|ui| {
                    for (index, input) in inputs.iter().enumerate() {
                        let active = monitor.current_input == Some(input.value);
                        let fill = if active { theme.accent_soft } else { theme.elevated };
                        let stroke = if active { theme.accent_bright } else { theme.border };
                        let prefix = if input.favorite { "• " } else { "" };
                        let response = ui.add_enabled(
                            monitor.ddc_supported && !busy,
                            Button::new(
                                RichText::new(format!("{prefix}{}", input.label))
                                    .size(13.0)
                                    .color(if active { theme.accent_bright } else { theme.text }),
                            )
                            .fill(fill)
                            .stroke(Stroke::new(1.0, stroke))
                            .corner_radius(13)
                            .min_size(Vec2::new(128.0, 42.0)),
                        );
                        let shortcut = if primary && index < 6 {
                            format!("Shortcut: {}", index + 1)
                        } else {
                            format!("VCP value: 0x{:02X}", input.value)
                        };
                        if response.on_hover_text(shortcut).clicked() {
                            self.begin_switch(&monitor.id, input.value);
                        }
                    }
                });

                if !monitor.ddc_supported {
                    ui.add_space(10.0);
                    Frame::new()
                        .fill(theme::with_alpha(theme.danger, 18))
                        .stroke(Stroke::new(1.0, theme::with_alpha(theme.danger, 90)))
                        .corner_radius(10)
                        .inner_margin(Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    "Enable DDC/CI in the monitor menu, then refresh. Some docks and adapters block DDC commands.",
                                )
                                .size(11.0)
                                .color(theme.danger),
                            );
                        });
                }
            });
    }

    fn show_loading_state(&self, ui: &mut Ui, theme: Theme) {
        Frame::new()
            .fill(theme.surface)
            .stroke(Stroke::new(1.0, theme.border))
            .corner_radius(22)
            .inner_margin(Margin::same(28))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Looking for physical monitors…")
                            .size(17.0)
                            .strong(),
                    );
                    ui.label(
                        RichText::new("DDC/CI queries can take a moment on some displays.")
                            .size(12.0)
                            .color(theme.muted),
                    );
                });
            });
    }

    fn show_empty_state(&mut self, ui: &mut Ui, theme: Theme) {
        Frame::new()
            .fill(theme.surface)
            .stroke(Stroke::new(1.0, theme.border))
            .corner_radius(22)
            .inner_margin(Margin::same(32))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    paint_empty_monitor(ui, theme);
                    ui.add_space(12.0);
                    ui.label(RichText::new("No controllable monitor found").size(20.0).strong());
                    ui.label(
                        RichText::new(
                            self.scan_error.as_deref().unwrap_or(
                                "Connect the monitor directly, enable DDC/CI in its on-screen menu, and try again.",
                            ),
                        )
                        .size(12.0)
                        .color(theme.muted),
                    );
                    ui.add_space(12.0);
                    if ui
                        .add(
                            Button::new("Scan again")
                                .fill(theme.accent)
                                .stroke(Stroke::new(1.0, theme.accent_bright))
                                .corner_radius(12),
                        )
                        .clicked()
                    {
                        self.request_scan();
                    }
                });
            });
    }

    fn show_footer(&self, ui: &mut Ui, theme: Theme) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Ctrl+Alt+M  Toggle window")
                    .size(10.0)
                    .color(theme.muted),
            );
            ui.separator();
            ui.label(
                RichText::new("Ctrl+Alt+1…4  Quick switch")
                    .size(10.0)
                    .color(theme.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let status = self
                    .last_scan
                    .map(|instant| format!("Updated {}s ago", instant.elapsed().as_secs()))
                    .unwrap_or_else(|| "Waiting for first scan".into());
                ui.label(RichText::new(status).size(10.0).color(theme.muted));
            });
        });
    }

    fn show_toasts(&mut self, context: &egui::Context) {
        self.toasts
            .retain(|toast| toast.created_at.elapsed() < Duration::from_secs(5));
        if self.toasts.is_empty() {
            return;
        }

        let theme = Theme::for_accent(self.config.data.accent);
        egui::Area::new(Id::new("toast-area"))
            .anchor(Align2::RIGHT_TOP, egui::vec2(-18.0, 18.0))
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                ui.set_width(350.0);
                for toast in self.toasts.iter().rev() {
                    let color = match toast.kind {
                        ToastKind::Success => theme.success,
                        ToastKind::Error => theme.danger,
                        ToastKind::Warning => Color32::from_rgb(250, 188, 82),
                        ToastKind::Info => theme.accent_bright,
                    };
                    Frame::new()
                        .fill(Color32::from_rgb(25, 29, 47))
                        .stroke(Stroke::new(1.0, theme::with_alpha(color, 150)))
                        .corner_radius(14)
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 7],
                            blur: 22,
                            spread: 0,
                            color: Color32::from_black_alpha(100),
                        })
                        .inner_margin(Margin::symmetric(14, 12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                                ui.painter().circle_filled(rect.center(), 4.5, color);
                                ui.label(RichText::new(&toast.message).size(12.0));
                            });
                        });
                    ui.add_space(8.0);
                }
            });
    }
}

impl eframe::App for MonitorControlApp {
    fn logic(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        if !self.window_chrome_applied {
            self.window_chrome_applied = crate::system::apply_dark_window_chrome(frame);
        }

        if context.input(|input| input.viewport().close_requested())
            && self.config.data.close_to_tray
            && !self.quit_requested
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(context);
        }

        self.process_worker_events();
        self.process_system_actions(context, frame);
        self.process_keyboard_shortcuts(context);

        if self.applied_accent != self.config.data.accent {
            self.applied_accent = self.config.data.accent;
            theme::install(context, self.config.data.accent);
        }

        context.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, root_ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.show_settings {
            self.show_settings_panel(root_ui);
        }
        self.show_main_panel(root_ui);
        let context = root_ui.ctx().clone();
        self.show_toasts(&context);
    }
}

#[derive(Clone, Copy)]
enum ToastKind {
    Success,
    Error,
    Warning,
    Info,
}

struct Toast {
    kind: ToastKind,
    message: String,
    created_at: Instant,
}

fn available_inputs(
    mut inputs: Vec<InputPreset>,
    supported_inputs: Option<&[u32]>,
    model: Option<&str>,
) -> Vec<InputPreset> {
    if let Some(supported) = supported_inputs {
        let default_values: Vec<_> = default_inputs()
            .into_iter()
            .map(|input| input.value)
            .collect();
        inputs.retain(|input| {
            supported.contains(&input.value) || !default_values.contains(&input.value)
        });
        for value in supported {
            if !inputs.iter().any(|input| input.value == *value) {
                inputs.push(InputPreset::new(
                    *value,
                    input_name_for_model(*value, model),
                    false,
                ));
            }
        }
    }

    for input in &mut inputs {
        if input.value == 0x10
            && matches!(
                input.label.as_str(),
                "DisplayPort 2" | "DisplayPort 2 / USB-C"
            )
        {
            input.label = input_name_for_model(input.value, model);
        }
    }
    inputs.sort_by_key(|input| input_sort_key(input.value));
    inputs
}

fn input_name_for_model(value: u32, model: Option<&str>) -> String {
    let is_wk95u_family = model.is_some_and(|model| {
        let model = model.to_ascii_uppercase();
        model.contains("WK95U") || model.contains("BK95U")
    });

    if value == 0x10 && is_wk95u_family {
        "Thunderbolt / USB-C".into()
    } else {
        input_name(value)
    }
}

fn input_sort_key(value: u32) -> (u8, u32) {
    let priority = match value {
        0x11 => 0,
        0x12 => 1,
        0x0f => 2,
        0x10 | 0x1b => 3,
        _ => 4,
    };
    (priority, value)
}

fn parse_input_code(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn section_label(ui: &mut Ui, label: &str, theme: Theme) {
    ui.label(RichText::new(label).size(10.0).strong().color(theme.muted));
    ui.add_space(4.0);
}

fn status_chip(ui: &mut Ui, label: &str, color: Color32, fill: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(20)
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(8.0).strong().color(color));
        });
}

fn paint_backdrop(ui: &mut Ui, theme: Theme) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, theme.background);

    for layer in (1..=7).rev() {
        let factor = layer as f32 / 7.0;
        painter.circle_filled(
            rect.left_top() + egui::vec2(115.0, 90.0),
            190.0 * factor,
            theme::with_alpha(theme.accent, (3.0 + 5.0 * (1.0 - factor)) as u8),
        );
        painter.circle_filled(
            rect.right_bottom() - egui::vec2(80.0, 80.0),
            240.0 * factor,
            theme::with_alpha(theme.accent_bright, (2.0 + 4.0 * (1.0 - factor)) as u8),
        );
    }
}

fn paint_brand_mark(ui: &mut Ui, theme: Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(14), theme.accent);
    let screen = egui::Rect::from_min_max(
        rect.min + egui::vec2(10.0, 11.0),
        rect.max - egui::vec2(10.0, 15.0),
    );
    ui.painter()
        .rect_filled(screen, CornerRadius::same(4), Color32::from_rgb(20, 24, 43));
    ui.painter().rect_stroke(
        screen,
        CornerRadius::same(4),
        Stroke::new(2.0, Color32::from_rgb(222, 249, 255)),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            screen.center_bottom(),
            screen.center_bottom() + egui::vec2(0.0, 6.0),
        ],
        Stroke::new(2.0, Color32::from_rgb(222, 249, 255)),
    );
    ui.painter().line_segment(
        [
            screen.center_bottom() + egui::vec2(-7.0, 6.0),
            screen.center_bottom() + egui::vec2(7.0, 6.0),
        ],
        Stroke::new(2.0, Color32::from_rgb(222, 249, 255)),
    );
}

fn paint_monitor_icon(ui: &mut Ui, theme: Theme, online: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(14), theme.elevated);
    let screen = egui::Rect::from_min_max(
        rect.min + egui::vec2(9.0, 10.0),
        rect.max - egui::vec2(9.0, 15.0),
    );
    ui.painter().rect_stroke(
        screen,
        CornerRadius::same(4),
        Stroke::new(
            1.6,
            if online {
                theme.accent_bright
            } else {
                theme.muted
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            screen.center_bottom(),
            screen.center_bottom() + egui::vec2(0.0, 5.0),
        ],
        Stroke::new(1.6, theme.muted),
    );
    ui.painter().line_segment(
        [
            screen.center_bottom() + egui::vec2(-6.0, 5.0),
            screen.center_bottom() + egui::vec2(6.0, 5.0),
        ],
        Stroke::new(1.6, theme.muted),
    );
}

fn paint_empty_monitor(ui: &mut Ui, theme: Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(110.0, 78.0), Sense::hover());
    let screen = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 16.0));
    ui.painter().rect_stroke(
        screen,
        CornerRadius::same(14),
        Stroke::new(2.0, theme.border),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .circle_filled(screen.center(), 6.0, theme::with_alpha(theme.accent, 120));
    ui.painter().line_segment(
        [
            screen.center_bottom(),
            screen.center_bottom() + egui::vec2(0.0, 11.0),
        ],
        Stroke::new(2.0, theme.border),
    );
    ui.painter().line_segment(
        [
            screen.center_bottom() + egui::vec2(-20.0, 11.0),
            screen.center_bottom() + egui::vec2(20.0, 11.0),
        ],
        Stroke::new(2.0, theme.border),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_and_hex_input_codes() {
        assert_eq!(parse_input_code("17"), Some(17));
        assert_eq!(parse_input_code("0x11"), Some(17));
        assert_eq!(parse_input_code("0X1B"), Some(27));
        assert_eq!(parse_input_code("HDMI"), None);
    }

    #[test]
    fn filters_default_inputs_to_monitor_capabilities() {
        let inputs = available_inputs(
            default_inputs(),
            Some(&[0x11, 0x12, 0x0f, 0x10]),
            Some("WK95U"),
        );

        assert_eq!(
            inputs.iter().map(|input| input.value).collect::<Vec<_>>(),
            vec![0x11, 0x12, 0x0f, 0x10]
        );
        assert_eq!(inputs[3].label, "Thunderbolt / USB-C");
    }

    #[test]
    fn preserves_user_defined_inputs_outside_capabilities() {
        let mut inputs = default_inputs();
        inputs.push(InputPreset::new(0xd0, "KVM input", true));

        let inputs = available_inputs(inputs, Some(&[0x11, 0x0f]), None);

        assert!(inputs.iter().any(|input| input.value == 0xd0));
        assert!(!inputs.iter().any(|input| input.value == 0x1b));
    }
}
