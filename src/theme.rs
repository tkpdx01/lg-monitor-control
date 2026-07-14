use eframe::egui::{self, Color32, Stroke};

use crate::config::Accent;

#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub elevated: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_bright: Color32,
    pub success: Color32,
    pub danger: Color32,
}

impl Theme {
    pub fn for_accent(accent: Accent) -> Self {
        let (accent, accent_bright) = match accent {
            Accent::Violet => (
                Color32::from_rgb(124, 92, 246),
                Color32::from_rgb(174, 150, 255),
            ),
            Accent::Cyan => (
                Color32::from_rgb(35, 184, 210),
                Color32::from_rgb(103, 232, 249),
            ),
            Accent::Rose => (
                Color32::from_rgb(225, 79, 142),
                Color32::from_rgb(251, 145, 190),
            ),
        };

        Self {
            background: Color32::from_rgb(10, 13, 24),
            surface: Color32::from_rgb(20, 24, 40),
            surface_hover: Color32::from_rgb(28, 33, 53),
            elevated: Color32::from_rgb(31, 35, 55),
            border: Color32::from_rgb(54, 60, 82),
            text: Color32::from_rgb(241, 244, 255),
            muted: Color32::from_rgb(145, 153, 180),
            accent,
            accent_soft: with_alpha(accent, 42),
            accent_bright,
            success: Color32::from_rgb(80, 214, 154),
            danger: Color32::from_rgb(255, 105, 124),
        }
    }
}

pub fn install(context: &egui::Context, accent: Accent) {
    let theme = Theme::for_accent(accent);
    context.set_theme(egui::Theme::Dark);
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.animation_time = 0.16;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style.spacing.interact_size.y = 36.0;

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = theme.background;
    visuals.window_fill = theme.surface;
    visuals.extreme_bg_color = Color32::from_rgb(13, 16, 29);
    visuals.faint_bg_color = Color32::from_rgb(24, 28, 45);
    visuals.override_text_color = Some(theme.text);
    visuals.selection.bg_fill = theme.accent;
    visuals.selection.stroke = Stroke::new(1.0, theme.accent_bright);
    visuals.hyperlink_color = theme.accent_bright;
    visuals.widgets.noninteractive.bg_fill = theme.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme.border);
    visuals.widgets.inactive.bg_fill = theme.elevated;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, theme.border);
    visuals.widgets.hovered.bg_fill = theme.surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.accent_bright);
    visuals.widgets.active.bg_fill = theme.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, theme.accent_bright);
    style.visuals = visuals;
    context.set_style_of(egui::Theme::Dark, style);
}

pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
