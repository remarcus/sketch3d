use super::color_editor::{ColorAction, ColorEditor, ColorTarget};
use super::*;
use crate::palette::Palette;

pub(super) struct PaletteUi {
    pub palette: Palette,
    pub sampling: bool,
    name: String,
    rename_name: String,
    selected: Option<usize>,
    path: Result<PathBuf, String>,
    notice: Option<String>,
    verification_frame: Option<u32>,
    color_editor: ColorEditor,
    pub sampling_target: ColorTarget,
    pub staged_sampling: bool,
}

impl PaletteUi {
    pub fn new() -> Self {
        let verify = std::env::args().any(|arg| arg.starts_with("--verify"));
        let own_verify = std::env::args().any(|arg| arg == "--verify-palette");
        let path = if verify {
            std::env::current_dir()
                .map(|root| root.join("verification/palette/palette.json"))
                .map_err(|error| error.to_string())
        } else {
            crate::presets::path().map(|path| path.with_file_name("palette.json"))
        };
        let loaded = if verify && !own_verify {
            Ok(Palette::default())
        } else {
            path.as_ref()
                .map_err(Clone::clone)
                .and_then(|path| Palette::load(path))
        };
        let notice = loaded
            .as_ref()
            .err()
            .map(|error| format!("Palette: {error}"));
        Self {
            palette: loaded.unwrap_or_default(),
            sampling: false,
            name: String::new(),
            rename_name: String::new(),
            selected: None,
            path,
            notice,
            verification_frame: own_verify.then_some(0),
            color_editor: ColorEditor::default(),
            sampling_target: ColorTarget::Brush,
            staged_sampling: false,
        }
    }

    fn save_candidate(&mut self, candidate: Palette) -> Result<(), String> {
        let path = self.path.as_ref().map_err(Clone::clone)?;
        candidate.save(path)?;
        self.palette = candidate;
        self.notice = Some("Palette saved".into());
        Ok(())
    }

    fn add(&mut self, color: [f32; 3]) {
        let mut candidate = self.palette.clone();
        match candidate.add(&self.name, color) {
            Ok(index) => match self.save_candidate(candidate) {
                Ok(()) => {
                    self.selected = Some(index);
                    self.rename_name = self.palette.swatches[index].name.clone();
                    self.name.clear();
                }
                Err(error) => self.notice = Some(format!("Cannot save palette: {error}")),
            },
            Err(error) => self.notice = Some(error),
        }
    }

    fn rename(&mut self) {
        let Some(index) = self.selected else {
            self.notice = Some("Choose a swatch to rename".into());
            return;
        };
        let mut candidate = self.palette.clone();
        match candidate.rename(index, &self.rename_name) {
            Ok(()) => match self.save_candidate(candidate) {
                Ok(()) => self.rename_name = self.palette.swatches[index].name.clone(),
                Err(error) => self.notice = Some(format!("Cannot save palette: {error}")),
            },
            Err(error) => self.notice = Some(error),
        }
    }

    fn remove(&mut self) {
        let Some(index) = self.selected else {
            self.notice = Some("Choose a swatch to remove".into());
            return;
        };
        let mut candidate = self.palette.clone();
        match candidate.remove(index) {
            Ok(()) => match self.save_candidate(candidate) {
                Ok(()) => {
                    self.selected = None;
                    self.rename_name.clear();
                }
                Err(error) => self.notice = Some(format!("Cannot save palette: {error}")),
            },
            Err(error) => self.notice = Some(error),
        }
    }

    pub fn choose_swatch(&self, index: usize, color: &mut [f32; 3]) -> bool {
        let Some(swatch) = self.palette.swatches.get(index) else {
            return false;
        };
        *color = swatch.color;
        true
    }

    pub fn color_button(&mut self, ui: &mut egui::Ui, target: ColorTarget, color: &[f32; 3]) {
        let editor_color = super::color_editor::target_to_linear(target, *color);
        let response = ui
            .add(
                egui::Button::new(target.label())
                    .fill(super::color_editor::color32_from_linear(editor_color))
                    .min_size(egui::vec2(112.0, 34.0)),
            )
            .on_hover_text(format!("Edit {}", target.label().to_lowercase()));
        if response.clicked() {
            self.color_editor.begin(target, editor_color, response.id);
        }
    }

    pub fn show_color_editor(&mut self, ctx: &egui::Context) -> ColorAction {
        self.color_editor.show(ctx, &self.palette)
    }

    pub fn start_sampling(&mut self, target: ColorTarget) {
        self.sampling_target = target;
        self.sampling = true;
        self.staged_sampling = true;
    }

    pub fn color_editor_active(&self) -> bool {
        self.color_editor.is_open()
    }

    pub(super) fn open_editor_for_capture(&mut self, target: ColorTarget, color: [f32; 3]) {
        self.color_editor.begin(
            target,
            color,
            egui::Id::new("appearance-layout-color-trigger"),
        );
    }

    pub fn set_color_from_sample(&mut self, color: [f32; 3]) {
        self.color_editor.sampled(color);
        self.sampling = false;
        self.staged_sampling = false;
    }

    pub fn cancel_editor_sampling(&mut self) {
        self.color_editor.cancel_sampling();
        self.sampling = false;
        self.staged_sampling = false;
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, color: &mut [f32; 3]) {
        ui.collapsing("Scene palette", |ui| {
            ui.small("Saved swatches are available in each color editor. Recent colors last for this session only.");
            ui.horizontal_wrapped(|ui| {
                for (index, swatch) in self.palette.swatches.iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new("  ")
                                .fill(super::color_editor::color32_from_linear(swatch.color))
                                .selected(self.selected == Some(index)),
                        )
                        .on_hover_text(format!("Select {} for palette management", swatch.name))
                        .clicked()
                    {
                        self.selected = Some(index);
                        self.rename_name = swatch.name.clone();
                    }
                    if ui
                        .selectable_label(self.selected == Some(index), &swatch.name)
                        .clicked()
                    {
                        self.selected = Some(index);
                        self.rename_name = swatch.name.clone();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("Swatch name"));
                if ui.button("Add current color").clicked() {
                    self.add(*color);
                }
            });
            if self.selected.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.rename_name).hint_text("Rename"));
                    if ui.button("Rename").clicked() {
                        self.rename();
                    }
                    if ui.button("Remove").clicked() {
                        self.remove();
                    }
                });
            }
            if let Some(notice) = &self.notice {
                ui.small(notice);
            }
        });
    }
}

impl SketchApp {
    pub(super) fn verify_palette(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.palette_ui.verification_frame else {
            return;
        };
        if frame == 1 {
            let target_fill = self
                .doc
                .strokes
                .iter()
                .find(|stroke| stroke.fill.is_some())
                .unwrap()
                .clone();
            let outline = self
                .doc
                .strokes
                .iter()
                .find(|stroke| stroke.fill.is_none())
                .unwrap()
                .clone();
            let mut sample_stroke = outline.clone();
            sample_stroke.id = self.next_id;
            sample_stroke.group_id = None;
            sample_stroke.color = [0.08, 0.78, 0.24];
            for point in &mut sample_stroke.points {
                point.position += glam::Vec3::new(3.4, 1.2, 0.3);
            }
            self.doc.strokes.push(sample_stroke.clone());
            let original = self.doc.strokes.clone();
            let fill_center = target_fill
                .points
                .iter()
                .map(|point| point.position)
                .sum::<glam::Vec3>()
                / target_fill.points.len() as f32;
            let (fill_pos, _) = crate::gizmo::project(&self.camera, rect, fill_center).unwrap();
            assert_eq!(
                crate::gizmo::pick_visible(&self.doc, &self.camera, rect, fill_pos),
                Some(target_fill.id)
            );

            let mut palette = Palette::default();
            let index = palette.add("  Awning Blue ", [0.08, 0.35, 0.82]).unwrap();
            palette.rename(index, "Shop Blue").unwrap();
            self.palette_ui.save_candidate(palette.clone()).unwrap();
            assert_eq!(
                Palette::load(self.palette_ui.path.as_ref().unwrap()).unwrap(),
                palette
            );
            let prior_color = self.color;
            self.palette_ui.choose_swatch(index, &mut self.color);
            assert_eq!(self.color, [0.08, 0.35, 0.82]);
            assert_eq!(self.doc.strokes, original);

            self.color = [0.97, 0.04, 0.12];
            self.renderer
                .set_selection(&HashSet::from([target_fill.id]));
            self.renderer.lighting.shadows = true;
            self.start_color_sampling();
            self.pointer_action(fill_pos, 1.0, rect, true, false);
            assert_eq!(self.color, target_fill.color);
            self.pointer_action(fill_pos + egui::vec2(20.0, 10.0), 1.0, rect, false, false);
            assert_eq!(self.doc.strokes, original);
            self.sampling_contact = false;
            assert!(!self.palette_ui.sampling);

            let line_center = sample_stroke.points[0]
                .position
                .lerp(sample_stroke.points[1].position, 0.5);
            let (line_pos, _) = crate::gizmo::project(&self.camera, rect, line_center).unwrap();
            let line_hit = crate::gizmo::pick_visible(&self.doc, &self.camera, rect, line_pos)
                .expect("outline point should be sampleable");
            assert_eq!(line_hit, sample_stroke.id);
            let sampled_stroke = self
                .doc
                .strokes
                .iter()
                .find(|stroke| stroke.id == line_hit)
                .unwrap()
                .clone();
            assert!(sampled_stroke.fill.is_none());
            self.start_color_sampling();
            self.pointer_action_with_tilt(line_pos, 0.7, rect, true, false, Some(Vec3::Y));
            assert_eq!(self.color, sampled_stroke.color);
            assert_eq!(self.doc.strokes, original);
            self.sampling_contact = false;

            self.color = [0.81, 0.12, 0.66];
            self.start_color_sampling();
            self.cancel_color_sampling();
            assert_eq!(self.color, [0.81, 0.12, 0.66]);
            assert_eq!(self.doc.strokes, original);

            let layer_id = target_fill.layer_id;
            let layer_index = self
                .doc
                .layers
                .iter()
                .position(|layer| layer.id == layer_id)
                .unwrap();
            self.doc.layers[layer_index].visible = false;
            self.color = [0.81, 0.12, 0.66];
            self.start_color_sampling();
            self.pointer_action(fill_pos, 1.0, rect, true, false);
            assert_eq!(self.color, [0.81, 0.12, 0.66]);
            assert_eq!(self.doc.strokes, original);
            self.sampling_contact = false;
            self.cancel_color_sampling();
            self.doc.layers[layer_index].visible = true;
            self.doc.layers[layer_index].locked = true;
            self.start_color_sampling();
            self.pointer_action_with_tilt(fill_pos, 1.0, rect, true, false, None);
            assert_eq!(self.color, target_fill.color);
            assert_eq!(self.doc.strokes, original);
            self.sampling_contact = false;
            self.doc.layers[layer_index].locked = false;

            self.color = prior_color;
            palette.remove(index).unwrap();
            self.palette_ui.save_candidate(palette.clone()).unwrap();
            assert_eq!(
                Palette::load(self.palette_ui.path.as_ref().unwrap()).unwrap(),
                palette
            );
            assert_eq!(self.doc.strokes, original);
            self.doc
                .strokes
                .retain(|stroke| stroke.id != sample_stroke.id);
            self.renderer.invalidate();
            self.status = "Palette and artwork color sampling verified".into();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let root = std::env::current_dir().unwrap().join("verification");
            let bytes: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            image::save_buffer(
                root.join("palette-preview.png"),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            std::fs::write(
                root.join("palette-report.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed": true,
                    "checks": [
                        "named palette add rename remove and restart persistence",
                        "swatch selection changes future color only",
                        "fill base color sampling ignores lighting and selection tint",
                        "outline base color sampled through pen and mouse paths",
                        "visible locked ink can be sampled; hidden ink is ignored",
                        "sampling cancellation preserves color and creates no strokes",
                        "sample contact latch prevents drag from creating ink"
                    ],
                    "gpu": self.renderer.adapter
                }))
                .unwrap(),
            )
            .unwrap();
            self.palette_ui.verification_frame = None;
            self.recovery.discard();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        assert!(frame < 150, "Palette verification screenshot timed out");
        self.palette_ui.verification_frame = Some(frame + 1);
    }
}
