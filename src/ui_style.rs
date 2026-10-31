//! Shared egui styling and small primitives used by the inspector panels.
//!
//! Keep these helpers deliberately small: later panel migrations can adopt a
//! common row and section without moving application state or changing its
//! semantics.

use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Ui, Vec2};

pub const FIELD_LABEL_WIDTH: f32 = 104.0;
pub const CONTROL_HEIGHT: f32 = 36.0;

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(20, 27, 37);
    style.visuals.window_fill = Color32::from_rgb(25, 34, 46);
    style.visuals.selection.bg_fill = Color32::from_rgb(27, 105, 111);
    style.visuals.selection.stroke = Stroke::new(1.5_f32, Color32::from_rgb(126, 226, 220));
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 49, 64);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 67, 84);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(32, 124, 132);
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.slider_width = 148.0;
    ctx.set_style(style);
}

pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(Color32::from_rgb(171, 190, 205)),
    );
}

/// Aligns a field label and its control using the shared inspector label width.
pub fn field_row<R>(ui: &mut Ui, label: &str, add_control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let label_width = FIELD_LABEL_WIDTH.min((ui.available_width() * 0.42).max(72.0));
        ui.allocate_ui_with_layout(
            Vec2::new(label_width, CONTROL_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(label).on_hover_text(label);
            },
        );
        add_control(ui)
    })
    .inner
}

/// Shows the interactive states used by shared controls. Hover/pressed and
/// focus are real egui interactions; invalid and mixed values are examples.
pub fn component_showcase(ui: &mut Ui) {
    egui::CollapsingHeader::new("UI component showcase")
        .id_salt("shared-ui-component-showcase")
        .show(ui, |ui| {
            ui.small("Hover or press the buttons; click Focus sample, then use Tab.");
            showcase_row(ui, "Normal", |ui| ui.button("Action"));
            showcase_row(ui, "Hover", |ui| ui.button("Hover me"));
            showcase_row(ui, "Pressed", |ui| {
                let response = ui.button("Hold to press");
                if response.is_pointer_button_down_on() {
                    ui.label("Active");
                }
                response
            });
            showcase_row(ui, "Selected", |ui| ui.selectable_label(true, "Selected"));
            showcase_row(ui, "Focused", |ui| {
                let response = ui.button("Focus sample");
                if response.clicked() {
                    response.request_focus();
                }
                let focused = response.has_focus();
                let stroke = if focused {
                    Stroke::new(2.0_f32, Color32::from_rgb(126, 226, 220))
                } else {
                    Stroke::NONE
                };
                ui.painter().rect_stroke(
                    response.rect.expand(1.0),
                    4.0,
                    stroke,
                    egui::StrokeKind::Outside,
                );
                response
            });
            showcase_row(ui, "Disabled", |ui| {
                ui.add_enabled(false, egui::Button::new("Unavailable"))
            });
            showcase_row(ui, "Invalid", |ui| {
                ui.horizontal(|ui| {
                    ui.painter().rect_stroke(
                        ui.max_rect(),
                        4.0,
                        Stroke::new(1.5_f32, Color32::from_rgb(238, 95, 95)),
                        egui::StrokeKind::Inside,
                    );
                    ui.label(
                        RichText::new("Invalid value").color(Color32::from_rgb(255, 145, 145)),
                    );
                })
            });
            showcase_row(ui, "Mixed", |ui| {
                ui.add_enabled(false, egui::Button::new("— Mixed values"))
            });
        });
}

fn showcase_row<R>(ui: &mut Ui, label: &str, add_sample: impl FnOnce(&mut Ui) -> R) -> R {
    field_row(ui, label, add_sample)
}
