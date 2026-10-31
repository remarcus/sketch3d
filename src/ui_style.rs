//! Shared egui styling and small primitives used by the inspector panels.
//!
//! Keep these helpers deliberately small: later panel migrations can adopt a
//! common row and section without moving application state or changing its
//! semantics.

use eframe::egui::{
    self, Align, Color32, Key, Layout, Response, RichText, Stroke, Ui, Vec2, WidgetInfo,
};
use std::{hash::Hash, ops::RangeInclusive};

const CURRENT_FIELD_LABEL: &str = "ui-style-current-field-accessible-label";

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
        let label_id = ui.make_persistent_id(CURRENT_FIELD_LABEL);
        ui.ctx()
            .data_mut(|data| data.insert_temp(label_id, label.to_owned()));
        let result = add_control(ui);
        ui.ctx().data_mut(|data| data.remove::<String>(label_id));
        result
    })
    .inner
}

fn accessible_field_label(ui: &Ui, suffix: &str) -> Option<String> {
    let label_id = ui.make_persistent_id(CURRENT_FIELD_LABEL);
    ui.ctx()
        .data(|data| data.get_temp::<String>(label_id))
        .map(|label| {
            if suffix.is_empty() {
                label
            } else {
                format!("{label}, {suffix}")
            }
        })
}

#[derive(Clone)]
struct NumericEditState {
    text: String,
    editing: bool,
    warning: Option<String>,
}

/// A precise numeric editor. Enter commits; Escape restores; clicking away
/// commits a valid value and discards an invalid one. Ranges are expressed in
/// the stored value's units; `display_scale` allows values such as 0.35 to be
/// edited as 35% without changing the model representation.
pub fn numeric_field(
    ui: &mut Ui,
    id_source: impl Hash,
    value: &mut f32,
    range: Option<RangeInclusive<f32>>,
    display_scale: f32,
    suffix: &str,
) -> Response {
    let id = ui.make_persistent_id(id_source);
    let display_scale = if display_scale.is_finite() && display_scale != 0.0 {
        display_scale
    } else {
        1.0
    };
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<NumericEditState>(id))
        .unwrap_or_else(|| NumericEditState {
            text: format_numeric(*value * display_scale),
            editing: false,
            warning: None,
        });

    if !state.editing {
        state.text = format_numeric(*value * display_scale);
    }

    let mut response = ui.add(
        egui::TextEdit::singleline(&mut state.text)
            .id(id)
            .desired_width(68.0)
            .horizontal_align(egui::Align::RIGHT),
    );
    if let Some(label) = accessible_field_label(ui, suffix) {
        response.widget_info(|| {
            let mut info = WidgetInfo::text_edit(ui.is_enabled(), &state.text, &state.text);
            info.label = Some(label.clone());
            info
        });
    }
    if response.gained_focus() {
        state.editing = true;
        state.warning = None;
        state.text = format_numeric(*value * display_scale);
    }
    if response.changed() {
        state.warning = None;
        state.editing = true;
    }

    if state.warning.is_some() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            3.0,
            Stroke::new(1.5_f32, Color32::from_rgb(238, 95, 95)),
            egui::StrokeKind::Outside,
        );
        response = response.on_hover_text(state.warning.as_deref().unwrap_or_default());
    }

    if state.editing && response.has_focus() {
        let enter = ui.input(|input| input.key_pressed(Key::Enter));
        let escape = ui.input(|input| input.key_pressed(Key::Escape));
        if escape {
            state.text = format_numeric(*value * display_scale);
            state.editing = false;
            state.warning = None;
            response.surrender_focus();
        } else if enter {
            match parse_numeric(&state.text, display_scale, range.as_ref()) {
                Ok(parsed) => {
                    *value = parsed;
                    state.text = format_numeric(*value * display_scale);
                    state.editing = false;
                    state.warning = None;
                    response.surrender_focus();
                }
                Err(error) => state.warning = Some(error),
            }
        }
    } else if state.editing && response.lost_focus() {
        match parse_numeric(&state.text, display_scale, range.as_ref()) {
            Ok(parsed) => {
                *value = parsed;
                state.text = format_numeric(*value * display_scale);
                state.warning = None;
            }
            Err(error) => {
                state.text = format_numeric(*value * display_scale);
                state.warning = Some(format!("{error} Uncommitted value was discarded."));
            }
        }
        state.editing = false;
    }

    if !suffix.is_empty() {
        ui.label(egui::RichText::new(suffix).small());
    }
    if let Some(warning) = &state.warning {
        ui.colored_label(Color32::from_rgb(255, 145, 145), warning);
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, state));
    response
}

/// Pairs the exact slider range with a keyboard-editable value in one row.
pub fn slider_row(
    ui: &mut Ui,
    label: &str,
    id_source: impl Hash + Clone,
    value: &mut f32,
    range: RangeInclusive<f32>,
    display_scale: f32,
    suffix: &str,
) -> Response {
    slider_row_inner(
        ui,
        label,
        id_source,
        value,
        range,
        SliderDisplay {
            scale: display_scale,
            suffix,
            logarithmic: false,
        },
    )
}

pub fn logarithmic_slider_row(
    ui: &mut Ui,
    label: &str,
    id_source: impl Hash + Clone,
    value: &mut f32,
    range: RangeInclusive<f32>,
    display_scale: f32,
    suffix: &str,
) -> Response {
    slider_row_inner(
        ui,
        label,
        id_source,
        value,
        range,
        SliderDisplay {
            scale: display_scale,
            suffix,
            logarithmic: true,
        },
    )
}

struct SliderDisplay<'a> {
    scale: f32,
    suffix: &'a str,
    logarithmic: bool,
}

fn slider_row_inner(
    ui: &mut Ui,
    label: &str,
    id_source: impl Hash,
    value: &mut f32,
    range: RangeInclusive<f32>,
    display: SliderDisplay<'_>,
) -> Response {
    field_row(ui, label, |ui| {
        let edit_width = 68.0 + if display.suffix.is_empty() { 0.0 } else { 18.0 };
        let slider_width = (ui.available_width() - edit_width - 6.0).max(32.0);
        let mut slider = egui::Slider::new(value, range.clone())
            .show_value(false)
            .trailing_fill(false);
        if display.logarithmic {
            slider = slider.logarithmic(true);
        }
        let slider_response = ui.add_sized(Vec2::new(slider_width, CONTROL_HEIGHT), slider);
        slider_response.widget_info(|| {
            WidgetInfo::slider(
                ui.is_enabled(),
                f64::from(*value),
                format!("{label}, {}", display.suffix),
            )
        });
        numeric_field(
            ui,
            id_source,
            value,
            Some(range),
            display.scale,
            display.suffix,
        )
    })
}

pub fn numeric_row(
    ui: &mut Ui,
    label: &str,
    id_source: impl Hash,
    value: &mut f32,
    range: Option<RangeInclusive<f32>>,
    suffix: &str,
) -> Response {
    field_row(ui, label, |ui| {
        numeric_field(ui, id_source, value, range, 1.0, suffix)
    })
}

pub fn integer_slider_row(
    ui: &mut Ui,
    label: &str,
    id_source: impl Hash + Clone,
    value: &mut usize,
    range: RangeInclusive<usize>,
) -> Response {
    field_row(ui, label, |ui| {
        let edit_width = 68.0;
        let slider_width = (ui.available_width() - edit_width - 6.0).max(32.0);
        let slider_response = ui.add_sized(
            Vec2::new(slider_width, CONTROL_HEIGHT),
            egui::Slider::new(value, range.clone()).show_value(false),
        );
        slider_response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *value as f64, label));
        integer_field(ui, id_source, value, range)
    })
}

fn integer_field(
    ui: &mut Ui,
    id_source: impl Hash,
    value: &mut usize,
    range: RangeInclusive<usize>,
) -> Response {
    let id = ui.make_persistent_id(id_source);
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<NumericEditState>(id))
        .unwrap_or_else(|| NumericEditState {
            text: value.to_string(),
            editing: false,
            warning: None,
        });
    if !state.editing {
        state.text = value.to_string();
    }
    let mut response = ui.add(
        egui::TextEdit::singleline(&mut state.text)
            .id(id)
            .desired_width(68.0)
            .horizontal_align(egui::Align::RIGHT),
    );
    if let Some(label) = accessible_field_label(ui, "") {
        response.widget_info(|| {
            let mut info = WidgetInfo::text_edit(ui.is_enabled(), &state.text, &state.text);
            info.label = Some(label.clone());
            info
        });
    }
    if response.gained_focus() {
        state.editing = true;
        state.warning = None;
        state.text = value.to_string();
    }
    if response.changed() {
        state.editing = true;
        state.warning = None;
    }
    if let Some(warning) = &state.warning {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            3.0,
            Stroke::new(1.5_f32, Color32::from_rgb(238, 95, 95)),
            egui::StrokeKind::Outside,
        );
        response = response.on_hover_text(warning);
    }
    if state.editing && response.has_focus() {
        let enter = ui.input(|input| input.key_pressed(Key::Enter));
        let escape = ui.input(|input| input.key_pressed(Key::Escape));
        if escape {
            state.text = value.to_string();
            state.editing = false;
            state.warning = None;
            response.surrender_focus();
        } else if enter {
            match state.text.trim().parse::<usize>() {
                Ok(parsed) if range.contains(&parsed) => {
                    *value = parsed;
                    state.text = parsed.to_string();
                    state.editing = false;
                    state.warning = None;
                    response.surrender_focus();
                }
                Ok(_) => {
                    state.warning = Some(format!(
                        "Enter a whole number from {} to {}.",
                        range.start(),
                        range.end()
                    ));
                }
                Err(_) => state.warning = Some("Enter a valid whole number.".into()),
            }
        }
    } else if state.editing && response.lost_focus() {
        match state.text.trim().parse::<usize>() {
            Ok(parsed) if range.contains(&parsed) => {
                *value = parsed;
                state.text = parsed.to_string();
                state.warning = None;
            }
            _ => {
                state.text = value.to_string();
                state.warning = Some(format!(
                    "Invalid value was discarded. Enter a whole number from {} to {}.",
                    range.start(),
                    range.end()
                ));
            }
        }
        state.editing = false;
    }
    if let Some(warning) = &state.warning {
        ui.colored_label(Color32::from_rgb(255, 145, 145), warning);
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, state));
    response
}

fn format_numeric(value: f32) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn parse_numeric(
    text: &str,
    display_scale: f32,
    range: Option<&RangeInclusive<f32>>,
) -> Result<f32, String> {
    let shown = text
        .trim()
        .parse::<f32>()
        .map_err(|_| "Enter a valid number.".to_owned())?;
    let value = shown / display_scale;
    if !value.is_finite() {
        return Err("Value must be finite.".into());
    }
    if let Some(range) = range {
        if !range.contains(&value) {
            return Err(format!(
                "Enter a value from {} to {}.",
                format_numeric(*range.start() * display_scale),
                format_numeric(*range.end() * display_scale)
            ));
        }
    }
    Ok(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_parser_preserves_precision_and_display_units() {
        assert!((parse_numeric("1.234567", 1.0, None).unwrap() - 1.234567).abs() < 1e-6);
        assert!((parse_numeric("35", 100.0, Some(&(0.0..=1.0))).unwrap() - 0.35).abs() < 1e-6);
    }

    #[test]
    fn numeric_parser_rejects_nonfinite_and_out_of_range_values() {
        assert!(parse_numeric("NaN", 1.0, None).is_err());
        assert!(parse_numeric("inf", 1.0, None).is_err());
        assert!(parse_numeric("101", 100.0, Some(&(0.0..=1.0))).is_err());
        assert!(parse_numeric("nope", 1.0, None).is_err());
    }
}
