use egui::{Color32, Visuals};

pub const OFFICE_ACCENT_RGB: (u8, u8, u8) = (68, 114, 196);
pub const OFFICE_RIBBON_RGB: (u8, u8, u8) = (255, 255, 255);
pub const OFFICE_CANVAS_RGB: (u8, u8, u8) = (244, 246, 248);
pub const OFFICE_TEXT_RGB: (u8, u8, u8) = (36, 36, 36);
pub const OFFICE_BORDER_RGB: (u8, u8, u8) = (214, 218, 223);

pub fn apply_office_chrome(context: &egui::Context) {
    let mut visuals = Visuals::light();
    visuals.panel_fill = Color32::from_rgb(
        OFFICE_RIBBON_RGB.0,
        OFFICE_RIBBON_RGB.1,
        OFFICE_RIBBON_RGB.2,
    );
    visuals.window_fill = visuals.panel_fill;
    visuals.override_text_color = Some(Color32::from_rgb(
        OFFICE_TEXT_RGB.0,
        OFFICE_TEXT_RGB.1,
        OFFICE_TEXT_RGB.2,
    ));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(
        OFFICE_CANVAS_RGB.0,
        OFFICE_CANVAS_RGB.1,
        OFFICE_CANVAS_RGB.2,
    );
    visuals.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(
        OFFICE_BORDER_RGB.0,
        OFFICE_BORDER_RGB.1,
        OFFICE_BORDER_RGB.2,
    );
    visuals.selection.bg_fill = Color32::from_rgb(
        OFFICE_ACCENT_RGB.0,
        OFFICE_ACCENT_RGB.1,
        OFFICE_ACCENT_RGB.2,
    );
    context.set_visuals(visuals);
}
