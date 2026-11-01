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

#[derive(Clone, Copy)]
pub struct SemanticColors {
    pub surface: Color32,
    pub elevated: Color32,
    pub input: Color32,
    pub text: Color32,
    pub muted_text: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub focus: Color32,
    pub warning: Color32,
    pub error: Color32,
}

fn semantic_colors(dark: bool) -> SemanticColors {
    if dark {
        SemanticColors {
            surface: Color32::from_rgb(20, 27, 37),
            elevated: Color32::from_rgb(25, 34, 46),
            input: Color32::from_rgb(15, 22, 32),
            text: Color32::from_rgb(233, 239, 244),
            muted_text: Color32::from_rgb(171, 190, 205),
            border: Color32::from_rgb(62, 77, 94),
            accent: Color32::from_rgb(76, 202, 195),
            selection: Color32::from_rgb(27, 105, 111),
            focus: Color32::from_rgb(126, 226, 220),
            warning: Color32::from_rgb(241, 190, 82),
            error: Color32::from_rgb(255, 135, 135),
        }
    } else {
        SemanticColors {
            surface: Color32::from_rgb(245, 246, 248),
            elevated: Color32::WHITE,
            input: Color32::WHITE,
            text: Color32::from_rgb(32, 39, 51),
            muted_text: Color32::from_rgb(82, 96, 111),
            border: Color32::from_rgb(183, 194, 204),
            accent: Color32::from_rgb(8, 127, 134),
            selection: Color32::from_rgb(195, 235, 232),
            focus: Color32::from_rgb(8, 127, 134),
            warning: Color32::from_rgb(126, 79, 0),
            error: Color32::from_rgb(176, 36, 24),
        }
    }
}

pub fn colors(ctx: &egui::Context) -> SemanticColors {
    semantic_colors(ctx.style().visuals.dark_mode)
}

/// Pick canvas text that contrasts with the configured solid scene background.
pub fn contrasting_canvas_text(background_srgb: [f32; 3]) -> Color32 {
    let luminance = scene_luminance(background_srgb);
    if luminance > 0.42 {
        Color32::from_rgb(32, 39, 51)
    } else {
        Color32::from_rgb(241, 245, 248)
    }
}

pub fn canvas_accent(background_srgb: [f32; 3]) -> Color32 {
    if scene_luminance(background_srgb) > 0.42 {
        Color32::from_rgb(0, 104, 110)
    } else {
        Color32::from_rgb(85, 232, 222)
    }
}

/// Pick a selection color that remains distinct from solid and image backgrounds.
pub fn canvas_selection_accent(background_srgb: [f32; 3], has_image: bool) -> Color32 {
    if has_image {
        Color32::from_rgb(220, 35, 165)
    } else if scene_luminance(background_srgb) > 0.42 {
        Color32::from_rgb(0, 91, 118)
    } else {
        Color32::from_rgb(255, 171, 48)
    }
}

pub fn canvas_overlay_accent(background_srgb: [f32; 3], has_image: bool) -> Color32 {
    if has_image {
        Color32::from_rgb(220, 35, 165)
    } else {
        canvas_accent(background_srgb)
    }
}

fn scene_luminance(background_srgb: [f32; 3]) -> f32 {
    background_srgb
        .into_iter()
        .map(|channel| {
            let channel = channel.clamp(0.0, 1.0);
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        })
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(channel, weight)| channel * weight)
        .sum::<f32>()
}

pub const FIELD_LABEL_WIDTH: f32 = 104.0;
pub const CONTROL_HEIGHT: f32 = 36.0;

pub fn apply(ctx: &egui::Context, preference: crate::presentation::UiTheme) {
    ctx.options_mut(|options| options.fallback_theme = egui::Theme::Dark);
    ctx.set_theme(preference.egui());
    let dark = ctx.style().visuals.dark_mode;
    let colors = semantic_colors(dark);
    let mut style = (*ctx.style()).clone();
    style.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = colors.surface;
    style.visuals.window_fill = colors.elevated;
    style.visuals.extreme_bg_color = colors.input;
    style.visuals.faint_bg_color = colors.surface;
    style.visuals.selection.bg_fill = colors.selection;
    style.visuals.selection.stroke = Stroke::new(1.5_f32, colors.focus);
    style.visuals.widgets.noninteractive.fg_stroke.color = colors.text;
    style.visuals.widgets.inactive.bg_fill = if dark {
        Color32::from_rgb(38, 49, 64)
    } else {
        Color32::from_rgb(232, 236, 240)
    };
    style.visuals.widgets.hovered.bg_fill = if dark {
        Color32::from_rgb(49, 67, 84)
    } else {
        Color32::from_rgb(222, 232, 237)
    };
    style.visuals.widgets.active.bg_fill = if dark {
        Color32::from_rgb(21, 106, 113)
    } else {
        Color32::from_rgb(187, 225, 222)
    };
    style.visuals.widgets.inactive.fg_stroke.color = colors.text;
    style.visuals.widgets.hovered.fg_stroke.color = colors.text;
    style.visuals.widgets.active.fg_stroke.color = colors.text;
    style.visuals.widgets.open.fg_stroke.color = colors.text;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, colors.focus);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.focus);
    style.visuals.widgets.noninteractive.bg_fill = colors.surface;
    style.visuals.widgets.noninteractive.bg_stroke.color = colors.border;
    style.visuals.widgets.inactive.bg_stroke.color = colors.border;
    style.visuals.hyperlink_color = colors.accent;
    style.visuals.warn_fg_color = colors.warning;
    style.visuals.error_fg_color = colors.error;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.slider_width = 148.0;
    ctx.set_style(style);
}

pub fn section(ui: &mut Ui, title: &str) {
    let muted = colors(ui.ctx()).muted_text;
    ui.add_space(4.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(muted),
    );
}

fn label_icon_button(response: Response, accessible_name: &str) -> Response {
    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::WidgetType::Button,
            response.enabled(),
            accessible_name,
        )
    });
    response
}

/// Adds a glyph button with a contextual label for assistive technology.
pub fn icon_button(ui: &mut Ui, glyph: &str, accessible_name: &str, enabled: bool) -> Response {
    label_icon_button(
        ui.add_enabled(enabled, egui::Button::new(glyph)),
        accessible_name,
    )
}

/// Adds a compact glyph button with a contextual label for assistive technology.
pub fn small_icon_button(
    ui: &mut Ui,
    glyph: &str,
    accessible_name: &str,
    enabled: bool,
) -> Response {
    label_icon_button(
        ui.add_enabled(enabled, egui::Button::new(glyph).small()),
        accessible_name,
    )
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
            Stroke::new(1.5_f32, colors(ui.ctx()).error),
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
        ui.colored_label(colors(ui.ctx()).error, warning);
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
            Stroke::new(1.5_f32, colors(ui.ctx()).error),
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
        ui.colored_label(colors(ui.ctx()).error, warning);
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
                    Stroke::new(2.0_f32, colors(ui.ctx()).focus)
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
                        Stroke::new(1.5_f32, colors(ui.ctx()).error),
                        egui::StrokeKind::Inside,
                    );
                    ui.label(RichText::new("Invalid value").color(colors(ui.ctx()).error));
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

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let [r, g, b] = [color.r(), color.g(), color.b()].map(|channel| {
                let value = f32::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (high, low) = {
            let a = luminance(a);
            let b = luminance(b);
            (a.max(b), a.min(b))
        };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn semantic_text_and_focus_tokens_meet_contrast_targets_in_both_themes() {
        for dark in [false, true] {
            let colors = semantic_colors(dark);
            for text in [colors.text, colors.muted_text] {
                for background in [colors.surface, colors.elevated, colors.input] {
                    assert!(
                        contrast_ratio(text, background) >= 4.5,
                        "text/background contrast below 4.5:1 in dark={dark}"
                    );
                }
            }
            for marker in [colors.focus, colors.accent] {
                for background in [colors.surface, colors.elevated, colors.input] {
                    assert!(
                        contrast_ratio(marker, background) >= 3.0,
                        "focus/accent boundary contrast below 3:1 in dark={dark}"
                    );
                }
            }
            let context = egui::Context::default();
            apply(
                &context,
                if dark {
                    crate::presentation::UiTheme::Dark
                } else {
                    crate::presentation::UiTheme::Light
                },
            );
            let style = context.style();
            for (name, widget) in [
                ("inactive", &style.visuals.widgets.inactive),
                ("hovered", &style.visuals.widgets.hovered),
                ("active", &style.visuals.widgets.active),
                ("open", &style.visuals.widgets.open),
            ] {
                assert!(
                    contrast_ratio(widget.fg_stroke.color, widget.bg_fill) >= 4.5,
                    "{name} control text contrast below 4.5:1 in dark={dark}: {:?} on {:?}",
                    widget.fg_stroke.color,
                    widget.bg_fill
                );
            }
            assert!(
                contrast_ratio(
                    style.visuals.selection.stroke.color,
                    style.visuals.selection.bg_fill,
                ) >= 3.0,
                "selection focus contrast below 3:1 in dark={dark}"
            );
        }
    }

    #[test]
    fn canvas_selection_and_preview_accents_contrast_on_solid_and_image_backgrounds() {
        let to_color = |rgb: [f32; 3]| {
            Color32::from_rgb(
                (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            )
        };
        for background in [[0.035, 0.045, 0.06], [0.94, 0.95, 0.97], [0.46, 0.31, 0.68]] {
            let canvas = to_color(background);
            for accent in [
                canvas_overlay_accent(background, false),
                canvas_selection_accent(background, false),
            ] {
                assert!(
                    contrast_ratio(accent, canvas) >= 3.0,
                    "canvas accent {accent:?} lacks 3:1 contrast on {canvas:?}"
                );
            }
        }

        for background in [
            [238.0 / 255.0, 236.0 / 255.0, 224.0 / 255.0],
            [31.0 / 255.0, 42.0 / 255.0, 55.0 / 255.0],
        ] {
            let canvas = to_color(background);
            for accent in [
                canvas_overlay_accent([0.0, 0.0, 0.0], true),
                canvas_selection_accent([0.0, 0.0, 0.0], true),
            ] {
                assert!(
                    contrast_ratio(accent, canvas) >= 3.0,
                    "image-background accent {accent:?} lacks 3:1 contrast on {canvas:?}"
                );
            }
        }
    }

    #[test]
    fn compact_icon_buttons_emit_contextual_accesskit_names() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(420.0, 180.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    small_icon_button(ui, "↑", "Move bookmark up", true);
                    small_icon_button(ui, "+", "Zoom in reference board", true);
                    small_icon_button(ui, "−", "Remove revolved profile 1", false);
                });
            },
        );
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit update is enabled for this context");
        let named_nodes = update
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                node.label()
                    .map(|label| (label.to_owned(), node.is_disabled()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(named_nodes.get("Move bookmark up"), Some(&false));
        assert_eq!(named_nodes.get("Zoom in reference board"), Some(&false));
        assert_eq!(named_nodes.get("Remove revolved profile 1"), Some(&true));
    }

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
