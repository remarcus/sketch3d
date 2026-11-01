//! Shared staged color editor for brush ink, the canvas background, and ground.
//! Model colors stay in linear RGB; text and swatches are shown as sRGB.

use super::ui_style;
use crate::{palette::Palette, presentation};
use eframe::egui::ecolor::Hsva;
use eframe::egui::{self, Color32, Id, RichText, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorTarget {
    Brush,
    Background,
    Ground,
}

impl ColorTarget {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush color",
            Self::Background => "Canvas background",
            Self::Ground => "Ground color",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ColorAction {
    None,
    Apply(ColorTarget, [f32; 3]),
    Sample(ColorTarget),
}

pub(super) fn apply_target(
    target: ColorTarget,
    color: [f32; 3],
    brush: &mut [f32; 3],
    background: &mut [f32; 3],
    ground: &mut [f32; 3],
) {
    match target {
        ColorTarget::Brush => *brush = color,
        ColorTarget::Background => {
            *background = color.map(presentation::linear_to_srgb);
        }
        ColorTarget::Ground => *ground = color,
    }
}

pub(super) fn target_to_linear(target: ColorTarget, color: [f32; 3]) -> [f32; 3] {
    if target == ColorTarget::Background {
        color.map(presentation::srgb_to_linear)
    } else {
        color
    }
}

#[derive(Clone, Debug)]
pub(super) struct ColorEditor {
    target: Option<ColorTarget>,
    open: bool,
    draft: [f32; 3],
    opening: [f32; 3],
    trigger_id: Option<Id>,
    restore_focus: bool,
    initial_focus: bool,
    hex: String,
    rgb: [String; 3],
    invalid: Option<String>,
    awaiting_sample: bool,
    recent: Vec<[f32; 3]>,
}

impl Default for ColorEditor {
    fn default() -> Self {
        Self {
            target: None,
            open: false,
            draft: [0.0; 3],
            opening: [0.0; 3],
            trigger_id: None,
            restore_focus: false,
            initial_focus: false,
            hex: "#000000".into(),
            rgb: ["0".into(), "0".into(), "0".into()],
            invalid: None,
            awaiting_sample: false,
            recent: Vec::new(),
        }
    }
}

impl ColorEditor {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn begin(&mut self, target: ColorTarget, color: [f32; 3], trigger_id: Id) {
        self.target = Some(target);
        self.draft = color;
        self.opening = color;
        self.trigger_id = Some(trigger_id);
        self.restore_focus = false;
        self.initial_focus = true;
        self.open = true;
        self.awaiting_sample = false;
        self.invalid = None;
        self.sync_text();
    }

    fn cancel(&mut self) {
        self.draft = self.opening;
        self.target = None;
        self.open = false;
        self.awaiting_sample = false;
        self.invalid = None;
        self.restore_focus = true;
        self.initial_focus = false;
        self.sync_text();
    }

    fn sync_text(&mut self) {
        self.hex = format_hex(self.draft);
        self.rgb = linear_to_srgb_bytes(self.draft).map(|value| value.to_string());
    }

    pub fn sampled(&mut self, color: [f32; 3]) {
        if self.awaiting_sample {
            self.draft = color;
            self.awaiting_sample = false;
            self.open = true;
            self.invalid = None;
            self.initial_focus = true;
            self.sync_text();
        }
    }

    pub fn cancel_sampling(&mut self) {
        if self.awaiting_sample {
            self.awaiting_sample = false;
            self.open = self.target.is_some();
            self.initial_focus = self.open;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette) -> ColorAction {
        if self.restore_focus {
            self.restore_focus = false;
            if let Some(trigger) = self.trigger_id.take() {
                ctx.memory_mut(|memory| memory.request_focus(trigger));
            }
        }
        if !self.open {
            return ColorAction::None;
        }
        let Some(target) = self.target else {
            self.open = false;
            return ColorAction::None;
        };

        let mut result = ColorAction::None;
        egui::Modal::new(Id::new("shared-color-editor")).show(ctx, |ui| {
            ui.set_min_width(390.0);
            let body_height = (ui.ctx().screen_rect().height() - 132.0).max(120.0);
            egui::ScrollArea::vertical()
                .max_height(body_height)
                .show(ui, |ui| {
                    ui.heading(target.label());
                    ui.small(
                        "The preview uses the stored base color. Lighting does not change it.",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Preview");
                        let (preview_rect, _) =
                            ui.allocate_exact_size(egui::vec2(86.0, 26.0), egui::Sense::hover());
                        ui.painter().rect_filled(
                            preview_rect,
                            4.0,
                            color32_from_linear(self.draft),
                        );
                        ui.label(RichText::new(format_hex(self.draft)).monospace());
                    });
                    ui.separator();

                    let mut hsva = Hsva::from_rgba_unmultiplied(
                        self.draft[0],
                        self.draft[1],
                        self.draft[2],
                        1.0,
                    );
                    if egui::color_picker::color_picker_hsva_2d(
                        ui,
                        &mut hsva,
                        egui::color_picker::Alpha::Opaque,
                    ) {
                        let rgba = hsva.to_rgba_unmultiplied();
                        self.draft = [rgba[0], rgba[1], rgba[2]];
                        self.sync_text();
                        self.invalid = None;
                    }

                    ui.horizontal(|ui| {
                        ui.label("HEX sRGB");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.hex)
                                .hint_text("#RRGGBB")
                                .desired_width(100.0),
                        );
                        if self.initial_focus {
                            response.request_focus();
                            self.initial_focus = false;
                        }
                        if response.changed() {
                            match parse_hex(&self.hex) {
                                Ok(color) => {
                                    self.draft = color;
                                    self.hex = format_hex(color);
                                    self.rgb =
                                        linear_to_srgb_bytes(color).map(|value| value.to_string());
                                    self.invalid = None;
                                }
                                Err(error) => self.invalid = Some(error),
                            }
                        }
                        if response.lost_focus() && response.changed() {
                            self.hex = format_hex(self.draft);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("RGB sRGB");
                        let mut changed = false;
                        for (axis, value) in ["R", "G", "B"].into_iter().zip(self.rgb.iter_mut()) {
                            ui.label(axis);
                            changed |= ui
                                .add(
                                    egui::TextEdit::singleline(value)
                                        .desired_width(48.0)
                                        .horizontal_align(egui::Align::RIGHT),
                                )
                                .changed();
                        }
                        if changed {
                            match parse_rgb(&self.rgb) {
                                Ok(color) => {
                                    self.draft = color;
                                    self.hex = format_hex(color);
                                    self.invalid = None;
                                }
                                Err(error) => self.invalid = Some(error),
                            }
                        }
                    });

                    if let Some(error) = &self.invalid {
                        ui.colored_label(ui_style::colors(ui.ctx()).error, error);
                    }

                    ui.separator();
                    ui.label("Saved swatches");
                    if palette.swatches.is_empty() {
                        ui.small("No saved swatches. Save colors from the palette section.");
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            for swatch in &palette.swatches {
                                if ui.add_swatch(swatch.name.as_str(), swatch.color).clicked() {
                                    self.draft = swatch.color;
                                    self.invalid = None;
                                    self.sync_text();
                                }
                            }
                        });
                    }

                    ui.label("Recent colors · kept for this session");
                    ui.horizontal_wrapped(|ui| {
                        for color in self.recent.clone() {
                            if ui.add_swatch(&format_hex(color), color).clicked() {
                                self.draft = color;
                                self.invalid = None;
                                self.sync_text();
                            }
                        }
                    });
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Sample artwork…").clicked() {
                    self.open = false;
                    self.awaiting_sample = true;
                    result = ColorAction::Sample(target);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        self.cancel();
                    }
                    if ui
                        .add_enabled(self.invalid.is_none(), egui::Button::new("Apply color"))
                        .clicked()
                    {
                        self.open = false;
                        self.awaiting_sample = false;
                        self.target = None;
                        self.opening = self.draft;
                        self.restore_focus = true;
                        self.recent.retain(|color| *color != self.draft);
                        self.recent.insert(0, self.draft);
                        self.recent.truncate(8);
                        result = ColorAction::Apply(target, self.draft);
                    }
                });
            });

            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.cancel();
            }
        });
        result
    }
}

trait SwatchButton {
    fn add_swatch(&mut self, name: &str, color: [f32; 3]) -> egui::Response;
}

impl SwatchButton for Ui {
    fn add_swatch(&mut self, name: &str, color: [f32; 3]) -> egui::Response {
        let fill = color32_from_linear(color);
        ui_style::accessible_button(
            self.add(
                egui::Button::new("  ")
                    .fill(fill)
                    .min_size(egui::vec2(34.0, 30.0)),
            ),
            &format!("Select color {name}"),
            None,
        )
        .on_hover_text(name)
    }
}

pub(super) fn color32_from_linear(color: [f32; 3]) -> Color32 {
    Color32::from(egui::Rgba::from_rgb(
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ))
}

pub(super) fn format_hex(linear: [f32; 3]) -> String {
    let [red, green, blue] = linear_to_srgb_bytes(linear);
    format!("#{red:02X}{green:02X}{blue:02X}")
}

fn linear_to_srgb_bytes(linear: [f32; 3]) -> [u8; 3] {
    linear.map(|value| {
        (presentation::linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

pub(super) fn parse_hex(text: &str) -> Result<[f32; 3], String> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Enter a six-digit sRGB hex color, such as #2AC8B8.".into());
    }
    let red = u8::from_str_radix(&hex[0..2], 16).map_err(|_| "Invalid hex color")?;
    let green = u8::from_str_radix(&hex[2..4], 16).map_err(|_| "Invalid hex color")?;
    let blue = u8::from_str_radix(&hex[4..6], 16).map_err(|_| "Invalid hex color")?;
    Ok([red, green, blue].map(|value| presentation::srgb_to_linear(value as f32 / 255.0)))
}

pub(super) fn parse_rgb(values: &[String; 3]) -> Result<[f32; 3], String> {
    let mut srgb = [0_u8; 3];
    for (index, text) in values.iter().enumerate() {
        let value = text
            .trim()
            .parse::<u8>()
            .map_err(|_| "RGB values must be whole sRGB numbers from 0 to 255.".to_owned())?;
        srgb[index] = value;
    }
    Ok(srgb.map(|value| presentation::srgb_to_linear(value as f32 / 255.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_swatch_buttons_emit_contextual_accesskit_labels() {
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
                    ui.add_swatch("Coral", [0.9, 0.2, 0.3]);
                });
            },
        );
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit update is enabled for this context");
        let swatch = update
            .nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.label() == Some("Select color Coral"))
            .expect("production color swatch has a contextual accessible name");
        assert_eq!(format!("{:?}", swatch.role()), "Button");
        assert!(!swatch.is_disabled());
    }

    #[test]
    fn hex_and_rgb_use_srgb_text_with_linear_model_values() {
        let teal = parse_hex("#2AC8B8").unwrap();
        assert_eq!(format_hex(teal), "#2AC8B8");
        assert_eq!(
            parse_rgb(&["42".into(), "200".into(), "184".into()]).unwrap(),
            teal
        );
        for component in teal {
            assert!(component.is_finite() && (0.0..=1.0).contains(&component));
        }
    }

    #[test]
    fn invalid_rgb_and_hex_are_rejected() {
        assert!(parse_hex("#12345").is_err());
        assert!(parse_hex("GG0000").is_err());
        assert!(parse_rgb(&["256".into(), "0".into(), "0".into()]).is_err());
        assert!(parse_rgb(&["NaN".into(), "0".into(), "0".into()]).is_err());
    }

    #[test]
    fn cancel_discards_staged_edit_and_keeps_other_targets_untouched() {
        let brush = [0.08, 0.35, 0.82];
        let background = [0.12, 0.2, 0.3];
        let ground = [0.4, 0.4, 0.4];
        let mut editor = ColorEditor::default();
        editor.begin(ColorTarget::Background, background, Id::new("test-trigger"));
        editor.draft = parse_hex("#2AC8B8").unwrap();
        editor.cancel();
        assert_eq!(editor.target, None);
        assert_eq!(editor.draft, background);
        assert_eq!(brush, [0.08, 0.35, 0.82]);
        assert_eq!(ground, [0.4, 0.4, 0.4]);
    }

    #[test]
    fn applying_color_changes_only_the_labeled_target() {
        let mut brush = [0.08, 0.35, 0.82];
        let mut background = [0.12, 0.2, 0.3];
        let mut ground = [0.4, 0.4, 0.4];
        let color = parse_hex("#2AC8B8").unwrap();
        apply_target(
            ColorTarget::Ground,
            color,
            &mut brush,
            &mut background,
            &mut ground,
        );
        assert_eq!(ground, color);
        assert_eq!(brush, [0.08, 0.35, 0.82]);
        assert_eq!(background, [0.12, 0.2, 0.3]);
    }

    #[test]
    fn background_target_converts_between_stored_srgb_and_editor_linear_rgb() {
        let stored_srgb = [0.16, 0.44, 0.74];
        let editor_rgb = target_to_linear(ColorTarget::Background, stored_srgb);
        assert_eq!(editor_rgb.map(presentation::linear_to_srgb), stored_srgb);
        let mut brush = [0.08, 0.35, 0.82];
        let mut background = stored_srgb;
        let mut ground = [0.4, 0.4, 0.4];
        let replacement = parse_hex("#2AC8B8").unwrap();
        apply_target(
            ColorTarget::Background,
            replacement,
            &mut brush,
            &mut background,
            &mut ground,
        );
        assert_eq!(
            format_hex(target_to_linear(ColorTarget::Background, background)),
            "#2AC8B8"
        );
        assert_eq!(brush, [0.08, 0.35, 0.82]);
        assert_eq!(ground, [0.4, 0.4, 0.4]);
    }

    #[test]
    fn hsva_picker_roundtrip_keeps_the_known_srgb_color() {
        let linear = parse_hex("#2AC8B8").unwrap();
        let hsva = Hsva::from_rgba_unmultiplied(linear[0], linear[1], linear[2], 1.0);
        let rgba = hsva.to_rgba_unmultiplied();
        assert_eq!(format_hex([rgba[0], rgba[1], rgba[2]]), "#2AC8B8");
        assert!((hsva.h - 0.473).abs() < 0.01);
    }

    #[test]
    fn sampled_artwork_is_staged_and_cancel_restores_the_opening_color() {
        let opening = [0.08, 0.35, 0.82];
        let sampled = parse_hex("#2AC8B8").unwrap();
        let mut brush = opening;
        let mut background = [0.12, 0.2, 0.3];
        let mut ground = [0.4, 0.4, 0.4];
        let mut editor = ColorEditor::default();
        editor.begin(ColorTarget::Brush, opening, Id::new("sample-trigger"));
        editor.awaiting_sample = true;
        editor.open = false;
        editor.sampled(sampled);
        assert!(editor.is_open());
        assert_eq!(editor.draft, sampled);
        assert_eq!(brush, opening);
        editor.cancel();
        assert_eq!(editor.draft, opening);
        assert_eq!(brush, opening);
        assert_eq!(background, [0.12, 0.2, 0.3]);
        assert_eq!(ground, [0.4, 0.4, 0.4]);

        // Applying the explicitly chosen brush draft mutates only the brush.
        apply_target(
            ColorTarget::Brush,
            sampled,
            &mut brush,
            &mut background,
            &mut ground,
        );
        assert_eq!(brush, sampled);
        assert_eq!(background, [0.12, 0.2, 0.3]);
        assert_eq!(ground, [0.4, 0.4, 0.4]);
    }
}
