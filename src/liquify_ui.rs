use super::*;
use crate::{
    editing::Snapshot,
    liquify::{Direction, Falloff, LiquifySettings},
};

pub(super) struct LiquifyDraft {
    before: Snapshot,
    after: Snapshot,
    settings: LiquifySettings,
    has_preview: bool,
}

#[derive(Clone, Copy)]
pub(super) struct LiquifyGesture {
    pub(super) start_screen: Pos2,
    pub(super) last_screen: Pos2,
    pub(super) start_world: Vec3,
    pub(super) plane_origin: Vec3,
    pub(super) plane_normal: Vec3,
    pub(super) base_strength: f32,
}

pub(super) struct StrokeSculptDraft {
    before: Snapshot,
    after: Snapshot,
    settings: crate::stroke_sculpt::Settings,
    has_preview: bool,
    brush_mode: bool,
    gesture: Option<LiquifyGesture>,
}

#[derive(Default)]
pub(super) struct LiquifyUi {
    draft: Option<LiquifyDraft>,
    sculpt: Option<StrokeSculptDraft>,
    brush_mode: bool,
    gesture: Option<LiquifyGesture>,
}

impl SketchApp {
    pub(super) fn liquify_active(&self) -> bool {
        self.liquify_ui.draft.is_some() || self.liquify_ui.sculpt.is_some()
    }

    pub(super) fn liquify_brush_active(&self) -> bool {
        (self.liquify_ui.draft.is_some() && self.liquify_ui.brush_mode)
            || self
                .liquify_ui
                .sculpt
                .as_ref()
                .is_some_and(|draft| draft.brush_mode)
    }

    pub(super) fn activate_liquify_brush(&mut self) {
        if self.liquify_ui.sculpt.is_some() {
            self.cancel_sculpt_gesture();
            if let Some(sculpt) = self.liquify_ui.sculpt.as_mut() {
                sculpt.brush_mode = true;
            }
        } else if self.liquify_ui.draft.is_some() {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = true;
        }
    }

    pub(super) fn begin_liquify(&mut self) {
        if self.selected.is_empty() {
            self.status = "Select ordinary strokes before starting Liquify.".into();
            return;
        }
        let before = crate::editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() || before.len() != self.selected.len() {
            self.status = "Liquify requires a complete visible, unlocked selection.".into();
            return;
        }
        if before.iter().any(|(_, stroke)| stroke.fill.is_some()) {
            self.status =
                "Liquify currently supports ordinary strokes; deselect filled surfaces first."
                    .into();
            return;
        }
        let mut center = Vec3::ZERO;
        let mut count = 0_u64;
        for (_, stroke) in &before {
            for point in &stroke.points {
                center += point.position;
                count += 1;
            }
        }
        if count > 0 {
            center /= count as f32;
        }
        let settings = LiquifySettings {
            center,
            displacement: Vec3::ZERO,
            radius: 0.5,
            strength: 0.75,
            direction: Direction::Push,
            falloff: Falloff::Smooth,
        };
        self.liquify_ui.draft = Some(LiquifyDraft {
            after: before.clone(),
            before,
            settings,
            has_preview: false,
        });
        self.status =
            "Liquify preview. Choose Brush on canvas or adjust numeric controls, then Apply or Cancel."
                .into();
    }

    fn refresh_liquify(&mut self) {
        let Some(draft) = self.liquify_ui.draft.as_mut() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
        }
        match crate::liquify::preview(&draft.before, draft.settings) {
            Ok(after) if after != draft.before => {
                crate::editing::replace(&mut self.doc, &draft.before, &after, true);
                draft.after = after;
                draft.has_preview = true;
                self.status =
                    "Liquify preview only. Apply to keep the deformation, or Cancel to restore."
                        .into();
            }
            Ok(_) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
            }
            Err(error) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
                self.status = error;
            }
        }
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
    }

    pub(super) fn finish_liquify(&mut self, apply: bool) {
        if self.liquify_ui.sculpt.is_some() {
            self.finish_stroke_sculpt(apply);
            return;
        }
        let Some(draft) = self.liquify_ui.draft.take() else {
            return;
        };
        self.liquify_ui.brush_mode = false;
        self.liquify_ui.gesture = None;
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview && apply {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
            match crate::liquify::edit(&self.doc, &ids, draft.settings) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.status =
                        "Liquify deformation applied. One undo restores the original points."
                            .into();
                }
                Err(error) => self.status = error,
            }
        } else if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
            self.status = if apply {
                "Liquify did not change the selected strokes."
            } else {
                "Liquify cancelled; original points restored."
            }
            .into();
        }
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
    }

    pub(super) fn begin_stroke_sculpt(&mut self) {
        if self.selected.is_empty() {
            self.status = "Select ordinary strokes before sculpting.".into();
            return;
        }
        if self.selected.iter().any(|id| {
            self.doc.has_linked_source(*id) || self.doc.linked_instance_for_stroke(*id).is_some()
        }) {
            self.status = "Make linked strokes unique before sculpting.".into();
            return;
        }
        let before = crate::editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() || before.len() != self.selected.len() {
            self.status = "Sculpting requires a complete visible, unlocked selection.".into();
            return;
        }
        let center = snapshot_center(&before);
        let settings = crate::stroke_sculpt::Settings {
            operation: crate::stroke_sculpt::Operation::Smooth,
            center,
            travel: 0.0,
            radius: 0.5,
            strength: 0.5,
            falloff: Falloff::Smooth,
            thickness_direction: crate::stroke_sculpt::ThicknessDirection::Increase,
        };
        if let Err(error) = crate::stroke_sculpt::preview(&before, settings) {
            self.status = error;
            return;
        }
        self.liquify_ui.sculpt = Some(StrokeSculptDraft {
            after: before.clone(),
            before,
            settings,
            has_preview: false,
            brush_mode: false,
            gesture: None,
        });
        self.status = "Sculpt preview. Choose a brush operation, then Apply or Cancel.".into();
    }

    fn refresh_stroke_sculpt(&mut self) {
        let Some(draft) = self.liquify_ui.sculpt.as_mut() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
        }
        match crate::stroke_sculpt::preview(&draft.before, draft.settings) {
            Ok(after) if after != draft.before => {
                crate::editing::replace(&mut self.doc, &draft.before, &after, true);
                draft.after = after;
                draft.has_preview = true;
                self.status = match draft.settings.operation {
                    crate::stroke_sculpt::Operation::Smooth => "Smooth preview only.",
                    crate::stroke_sculpt::Operation::Relax => "Relax preview only.",
                    crate::stroke_sculpt::Operation::Thickness => {
                        "Thickness preview only. A dedicated per-sample multiplier changes width while pressure, opacity, tilt, and timing stay intact."
                    }
                }
                .to_owned();
            }
            Ok(_) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
            }
            Err(error) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
                self.status = error;
            }
        }
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
    }

    fn finish_stroke_sculpt(&mut self, apply: bool) {
        let Some(draft) = self.liquify_ui.sculpt.take() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
        }
        if draft.has_preview && apply {
            match crate::stroke_sculpt::edit(&self.doc, &ids, draft.settings) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.status =
                        "Sculpt operation applied. One undo restores the source strokes.".into();
                }
                Err(error) => self.status = error,
            }
        } else if draft.has_preview {
            self.status = "Sculpt cancelled; original samples restored.".into();
        } else if apply {
            self.status = "Sculpt did not change the selected strokes.".into();
        }
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
    }

    fn stroke_sculpt_controls(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut apply = false;
        let mut cancel = false;
        let mut enter_brush = false;
        let mut leave_brush = false;
        ui.group(|ui| {
            ui.strong("STROKE SCULPT PREVIEW");
            ui.small("Sampled strokes only. Fills and semantic curves use their dedicated editors; make linked strokes unique first.");
            if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
                if draft.brush_mode {
                    ui.label("Drag within the brush radius; travel controls a deterministic bounded strength.");
                    if ui.button("Use settings only").clicked() {
                        leave_brush = true;
                    }
                } else if ui.button("Brush on canvas…").clicked() {
                    enter_brush = true;
                }
                egui::ComboBox::from_label("Operation")
                    .selected_text(match draft.settings.operation {
                        crate::stroke_sculpt::Operation::Smooth => "Smooth centerline",
                        crate::stroke_sculpt::Operation::Relax => "Relax centerline",
                        crate::stroke_sculpt::Operation::Thickness => "Thickness profile",
                    })
                    .show_ui(ui, |ui| {
                        changed |= ui.selectable_value(
                            &mut draft.settings.operation,
                            crate::stroke_sculpt::Operation::Smooth,
                            "Smooth centerline",
                        ).changed();
                        changed |= ui.selectable_value(
                            &mut draft.settings.operation,
                            crate::stroke_sculpt::Operation::Relax,
                            "Relax centerline",
                        ).changed();
                        changed |= ui.selectable_value(
                            &mut draft.settings.operation,
                            crate::stroke_sculpt::Operation::Thickness,
                            "Thickness profile",
                        ).changed();
                    });
                changed |= ui.add(
                    egui::Slider::new(&mut draft.settings.radius, 0.01..=10.0)
                        .text("Radius (wu)"),
                ).changed();
                changed |= ui.add(
                    egui::Slider::new(&mut draft.settings.strength, 0.0..=1.0)
                        .text("Strength"),
                ).changed();
                egui::ComboBox::from_label("Falloff")
                    .selected_text(format!("{:?}", draft.settings.falloff))
                    .show_ui(ui, |ui| {
                        for falloff in [Falloff::Linear, Falloff::Smooth, Falloff::Sharp] {
                            changed |= ui.selectable_value(
                                &mut draft.settings.falloff,
                                falloff,
                                format!("{falloff:?}"),
                            ).changed();
                        }
                    });
                if draft.settings.operation == crate::stroke_sculpt::Operation::Thickness {
                    egui::ComboBox::from_label("Thickness direction")
                        .selected_text(format!("{:?}", draft.settings.thickness_direction))
                        .show_ui(ui, |ui| {
                            for direction in [
                                crate::stroke_sculpt::ThicknessDirection::Increase,
                                crate::stroke_sculpt::ThicknessDirection::Decrease,
                            ] {
                                changed |= ui.selectable_value(
                                    &mut draft.settings.thickness_direction,
                                    direction,
                                    format!("{direction:?}"),
                                ).changed();
                            }
                        });
                    ui.small("Thickness uses a dedicated per-sample width multiplier. Pressure and pressure-driven opacity are preserved.");
                }
            }
            ui.horizontal(|ui| {
                apply = ui.button("Apply sculpt").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if enter_brush {
            if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
                draft.brush_mode = true;
            }
            self.status =
                "Sculpt brush ready. Drag selected ink on the camera-facing plane.".into();
        }
        if leave_brush {
            self.cancel_sculpt_gesture();
            if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
                draft.brush_mode = false;
            }
        }
        if changed {
            self.refresh_stroke_sculpt();
        }
        if apply {
            self.finish_stroke_sculpt(true);
        }
        if cancel {
            self.finish_stroke_sculpt(false);
        }
    }

    fn stroke_sculpt_brush_input(
        &mut self,
        pos: Pos2,
        pressure: f32,
        rect: Rect,
        phase: crate::input::Phase,
    ) {
        if !self
            .liquify_ui
            .sculpt
            .as_ref()
            .is_some_and(|draft| draft.brush_mode)
        {
            return;
        }
        match phase {
            crate::input::Phase::Down => {
                if !rect.contains(pos) {
                    return;
                }
                let Some(draft) = self.liquify_ui.sculpt.as_ref() else {
                    return;
                };
                let plane_origin = snapshot_center(&draft.before);
                let plane_normal = self.camera.basis().2;
                let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let denominator = ray_direction.dot(plane_normal);
                if denominator.abs() < 1.0e-5 {
                    self.status =
                        "Cannot start sculpt: camera ray is parallel to the working plane.".into();
                    return;
                }
                let distance = (plane_origin - ray_origin).dot(plane_normal) / denominator;
                if !distance.is_finite() || distance <= 0.0 {
                    self.status = "Cannot start sculpt behind the camera.".into();
                    return;
                }
                let start_world = ray_origin + ray_direction * distance;
                let gesture = LiquifyGesture {
                    start_screen: pos,
                    last_screen: pos,
                    start_world,
                    plane_origin,
                    plane_normal,
                    base_strength: self
                        .liquify_ui
                        .sculpt
                        .as_ref()
                        .map_or(0.5, |d| d.settings.strength),
                };
                if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
                    draft.gesture = Some(gesture);
                    draft.settings.center = start_world;
                    draft.settings.travel = 0.0;
                }
            }
            crate::input::Phase::Move | crate::input::Phase::Up => {
                let Some(gesture) = self
                    .liquify_ui
                    .sculpt
                    .as_ref()
                    .and_then(|draft| draft.gesture)
                else {
                    return;
                };
                if !rect.contains(pos) {
                    self.cancel_sculpt_gesture();
                    self.status = "Sculpt drag cancelled outside the canvas.".into();
                    return;
                }
                let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let denominator = ray_direction.dot(gesture.plane_normal);
                if denominator.abs() < 1.0e-5 {
                    return;
                }
                let distance =
                    (gesture.plane_origin - ray_origin).dot(gesture.plane_normal) / denominator;
                if !distance.is_finite() || distance <= 0.0 {
                    return;
                }
                let current_world = ray_origin + ray_direction * distance;
                let travel = gesture.start_world.distance(current_world);
                if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
                    draft.settings.travel = travel;
                    draft.settings.strength =
                        (gesture.base_strength * pressure.clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    if phase == crate::input::Phase::Up {
                        draft.gesture = None;
                    }
                }
                self.refresh_stroke_sculpt();
                if phase == crate::input::Phase::Up {
                    self.status =
                        "Sculpt preview ready. Apply to keep the change, or Cancel to restore."
                            .into();
                }
            }
            crate::input::Phase::Cancel => self.finish_stroke_sculpt(false),
        }
    }

    fn cancel_sculpt_gesture(&mut self) {
        let restore = self.liquify_ui.sculpt.as_mut().map(|draft| {
            let should_restore = draft.has_preview;
            let after = draft.after.clone();
            let before = draft.before.clone();
            let ids: HashSet<_> = before.iter().map(|(_, stroke)| stroke.id).collect();
            draft.gesture = None;
            draft.settings.travel = 0.0;
            draft.settings.center = snapshot_center(&draft.before);
            draft.has_preview = false;
            draft.after = before.clone();
            (should_restore, after, before, ids)
        });
        if let Some((true, after, before, ids)) = restore {
            crate::editing::replace(&mut self.doc, &before, &after, false);
            self.renderer
                .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
        }
    }

    pub(super) fn verify_stroke_sculpt_workflow(&mut self, rect: Rect) {
        let saved_document = self.doc.clone();
        let saved_selection = self.selected.clone();
        let saved_undo = self.undo.clone();
        let saved_redo = self.redo.clone();
        let saved_dirty = self.dirty;
        let saved_camera = self.camera;
        let saved_sculpt = self.liquify_ui.sculpt.take();
        let saved_liquify = self.liquify_ui.draft.take();
        let saved_brush_mode = self.liquify_ui.brush_mode;
        let points = [
            Vec3::new(-0.4, 0.0, 0.0),
            Vec3::new(-0.2, 0.35, 0.0),
            Vec3::new(0.0, 0.6, 0.0),
            Vec3::new(0.2, 0.35, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
        ];
        let fixture = Stroke {
            model_surface: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: saved_document.active_layer,
            id: 700_001,
            color: [0.25, 0.5, 0.8],
            gradient: None,
            radius: 0.06,
            brush: crate::document::Brush {
                texture: crate::texture::TextureSettings {
                    kind: crate::texture::TextureKind::DryBrush,
                    strength: 0.5,
                    ..Default::default()
                },
                ..Default::default()
            },
            points: points
                .into_iter()
                .enumerate()
                .map(|(index, position)| crate::document::Point {
                    position,
                    pressure: 0.3 + index as f32 * 0.1,
                    tilt: Some(Vec3::Z),
                    time_ms: index as u32 * 8,
                })
                .collect(),
            curve: None,
        };
        let id = fixture.id;
        let center = fixture
            .points
            .iter()
            .map(|point| point.position)
            .sum::<Vec3>()
            / fixture.points.len() as f32;
        self.doc = Document {
            layers: saved_document.layers.clone(),
            active_layer: saved_document.active_layer,
            version: saved_document.version,
            strokes: vec![fixture],
            ..Document::default()
        };
        self.selected = HashSet::from([id]);
        self.undo.clear();
        self.redo.clear();
        self.camera.align(crate::geometry::Plane::Front);
        self.camera.target = center;
        self.camera.distance = 5.0;
        self.liquify_ui.sculpt = None;
        self.liquify_ui.draft = None;
        self.liquify_ui.brush_mode = false;
        self.renderer.invalidate();
        let source = self.doc.strokes.clone();
        let pressure = source[0]
            .points
            .iter()
            .map(|point| point.pressure)
            .collect::<Vec<_>>();
        let (right, _, _) = self.camera.basis();
        let start = crate::gizmo::project(&self.camera, rect, center - right * 0.1)
            .expect("sculpt start projects")
            .0;
        let end = crate::gizmo::project(&self.camera, rect, center + right * 0.4)
            .expect("sculpt end projects")
            .0;
        assert!(rect.contains(start) && rect.contains(end));

        // Exercise the actual panel draft, canvas pointer route, cancellation,
        // Apply, history, picking, and persistence using synthetic mouse input.
        self.begin_stroke_sculpt();
        assert!(self.liquify_ui.sculpt.is_some());
        if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
            draft.settings.radius = 1.0;
            draft.settings.strength = 1.0;
            draft.settings.operation = crate::stroke_sculpt::Operation::Smooth;
        }
        self.activate_liquify_brush();
        self.liquify_brush_input(start, 1.0, rect, crate::input::Phase::Down);
        self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Move);
        self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Up);
        let smooth_preview = self.doc.strokes.clone();
        assert_ne!(
            smooth_preview, source,
            "Synthetic mouse drag creates a smooth preview"
        );
        assert!(self.undo.is_empty(), "Preview does not enter history");
        assert_eq!(smooth_preview[0].points[0], source[0].points[0]);
        assert_eq!(smooth_preview[0].points[4], source[0].points[4]);
        self.finish_liquify(false);
        assert_eq!(
            self.doc.strokes, source,
            "Cancel restores exact source strokes"
        );

        self.begin_stroke_sculpt();
        if let Some(draft) = self.liquify_ui.sculpt.as_mut() {
            draft.settings.radius = 1.0;
            draft.settings.strength = 0.8;
            draft.settings.operation = crate::stroke_sculpt::Operation::Thickness;
            draft.settings.thickness_direction = crate::stroke_sculpt::ThicknessDirection::Increase;
        }
        self.activate_liquify_brush();
        self.liquify_brush_input(start, 1.0, rect, crate::input::Phase::Down);
        self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Move);
        self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Up);
        let thickness_preview = self.doc.strokes.clone();
        assert_ne!(thickness_preview[0].point_thickness, None);
        assert_eq!(thickness_preview[0].points, source[0].points);
        assert_eq!(
            thickness_preview[0]
                .points
                .iter()
                .map(|point| point.pressure)
                .collect::<Vec<_>>(),
            pressure
        );
        assert!(
            self.undo.is_empty(),
            "Thickness preview remains detached from history"
        );
        self.finish_liquify(true);
        assert_eq!(self.undo.len(), 1, "Apply creates one atomic history edit");
        assert_eq!(self.doc.strokes, thickness_preview);
        let thickness_mesh = crate::geometry::stroke_mesh(&self.doc.strokes[0]);
        assert!(thickness_mesh
            .iter()
            .zip(crate::geometry::stroke_mesh(&source[0]).iter())
            .any(|(after, before)| Vec3::from(after.position)
                .distance(Vec3::from(before.position))
                > 1.0e-5));
        let pick_position =
            crate::gizmo::project(&self.camera, rect, self.doc.strokes[0].points[2].position)
                .unwrap()
                .0;
        assert_eq!(
            crate::gizmo::pick(&self.doc, &self.camera, rect, pick_position),
            Some(id)
        );
        self.undo();
        assert_eq!(self.doc.strokes, source);
        self.redo();
        assert_eq!(self.doc.strokes, thickness_preview);
        let native = Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
        assert_eq!(native.strokes, thickness_preview);
        let recovery_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "sculpt-profile-verification".into(),
            None,
            native.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
        let scan = crate::recovery_store::scan(recovery_dir.path(), "fresh-sculpt-reader").unwrap();
        assert_eq!(
            scan.candidates[0].snapshot.document.strokes,
            thickness_preview
        );
        let glb = crate::export::encode(&native, None).unwrap();
        assert!(glb.summary.triangles > 0);
        let obj_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_dir.path().join("sculpt-profile.obj");
        assert!(crate::obj_export::write_obj_bundle(&native, None, &obj_path, false).unwrap() > 0);

        self.doc = saved_document;
        self.selected = saved_selection;
        self.undo = saved_undo;
        self.redo = saved_redo;
        self.dirty = saved_dirty;
        self.camera = saved_camera;
        self.liquify_ui.sculpt = saved_sculpt;
        self.liquify_ui.draft = saved_liquify;
        self.liquify_ui.brush_mode = saved_brush_mode;
        self.renderer.invalidate();
    }

    fn paint_sculpt_cursor(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        pointer: Option<Pos2>,
    ) -> Result<(), &'static str> {
        let draft = self
            .liquify_ui
            .sculpt
            .as_ref()
            .ok_or("Sculpt draft is missing")?;
        let pointer = pointer
            .filter(|position| rect.contains(*position))
            .ok_or("synthetic pointer is outside the canvas")?;
        let center = draft
            .gesture
            .as_ref()
            .map_or(pointer, |gesture| gesture.start_screen);
        let plane_origin = draft
            .gesture
            .as_ref()
            .map_or(draft.settings.center, |gesture| gesture.plane_origin);
        let world = draft.settings.center;
        let radius = draft.settings.radius / self.camera.world_per_pixel(world, rect.height());
        painter.with_clip_rect(rect).circle_stroke(
            center,
            radius,
            egui::Stroke::new(1.5_f32, Color32::WHITE),
        );
        painter.circle_stroke(
            center,
            radius + 1.0,
            egui::Stroke::new(0.7_f32, Color32::BLACK),
        );
        let depth = (self.camera.eye() - plane_origin).dot(self.camera.basis().2);
        painter.text(
            center + Vec2::new(radius + 8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("r {:.2} wu · depth {depth:.2} wu", draft.settings.radius),
            egui::FontId::proportional(12.0),
            super::ui_style::canvas_overlay_accent(
                self.presentation.preferences.background,
                self.presentation.preferences.background_image.is_some(),
            ),
        );
        Ok(())
    }

    pub(super) fn liquify_controls(&mut self, ui: &mut egui::Ui) {
        if self.liquify_ui.sculpt.is_some() {
            self.stroke_sculpt_controls(ui);
            return;
        }
        if self.liquify_ui.draft.is_none() {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.selected.is_empty(),
                        egui::Button::new("Liquify selected strokes…"),
                    )
                    .clicked()
                {
                    self.begin_liquify();
                }
                if ui
                    .add_enabled(
                        !self.selected.is_empty(),
                        egui::Button::new("Smooth / relax / thickness…"),
                    )
                    .clicked()
                {
                    self.begin_stroke_sculpt();
                }
            });
            return;
        }
        let mut changed = false;
        let mut apply = false;
        let mut cancel = false;
        let mut enter_brush = false;
        let mut leave_brush = false;
        ui.group(|ui| {
            ui.strong("LIQUIFY PREVIEW");
            if self.liquify_ui.brush_mode {
                ui.label("Drag across the canvas to preview selected-ink deformation.");
                ui.small("Working plane: camera-facing plane through the selected ink centroid; depth stays fixed.");
                if ui.button("Use numeric controls").clicked() {
                    leave_brush = true;
                }
            } else if ui.button("Brush on canvas…").clicked() {
                enter_brush = true;
            }
            if let Some(draft) = self.liquify_ui.draft.as_mut() {
                if !self.liquify_ui.brush_mode {
                    ui.label("Brush center (world units)");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.x)
                                    .speed(0.01)
                                    .prefix("X "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.y)
                                    .speed(0.01)
                                    .prefix("Y "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.z)
                                    .speed(0.01)
                                    .prefix("Z "),
                            )
                            .changed();
                    });
                    ui.label("Brush displacement");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.x)
                                    .speed(0.01)
                                    .prefix("X "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.y)
                                    .speed(0.01)
                                    .prefix("Y "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.z)
                                    .speed(0.01)
                                    .prefix("Z "),
                            )
                            .changed();
                    });
                }
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.settings.radius, 0.001..=10.0)
                            .text("Radius (wu)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.settings.strength, 0.0..=1.0).text("Strength"),
                    )
                    .changed();
                egui::ComboBox::from_label("Direction")
                    .selected_text(format!("{:?}", draft.settings.direction))
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.direction,
                                Direction::Push,
                                "Push",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.direction,
                                Direction::Pull,
                                "Pull",
                            )
                            .changed();
                    });
                egui::ComboBox::from_label("Falloff")
                    .selected_text(format!("{:?}", draft.settings.falloff))
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.falloff,
                                Falloff::Linear,
                                "Linear",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.falloff,
                                Falloff::Smooth,
                                "Smooth",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(&mut draft.settings.falloff, Falloff::Sharp, "Sharp")
                            .changed();
                    });
            }
            ui.horizontal(|ui| {
                apply = ui.button("Apply deformation").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if enter_brush {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = true;
            self.status =
                "Liquify brush ready. Drag selected ink on its camera-facing working plane.".into();
        }
        if leave_brush {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = false;
        }
        if changed {
            self.refresh_liquify();
        }
        if apply {
            self.finish_liquify(true);
        }
        if cancel {
            self.finish_liquify(false);
        }
    }

    pub(super) fn liquify_brush_input(
        &mut self,
        pos: Pos2,
        pressure: f32,
        rect: Rect,
        phase: crate::input::Phase,
    ) {
        if self.liquify_ui.sculpt.is_some() {
            self.stroke_sculpt_brush_input(pos, pressure, rect, phase);
            return;
        }
        if !self.liquify_brush_active() {
            return;
        }
        match phase {
            crate::input::Phase::Down => {
                if !rect.contains(pos) {
                    return;
                }
                let Some(draft) = self.liquify_ui.draft.as_ref() else {
                    return;
                };
                let mut plane_origin = Vec3::ZERO;
                let mut point_count = 0usize;
                for (_, stroke) in &draft.before {
                    for point in &stroke.points {
                        plane_origin += point.position;
                        point_count += 1;
                    }
                }
                if point_count == 0 {
                    return;
                }
                plane_origin /= point_count as f32;
                let plane_normal = self.camera.basis().2;
                let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let denominator = ray_direction.dot(plane_normal);
                if denominator.abs() < 1.0e-5 {
                    self.status =
                        "Cannot start Liquify: camera ray is parallel to the working plane.".into();
                    return;
                }
                let distance = (plane_origin - ray_origin).dot(plane_normal) / denominator;
                if !distance.is_finite() || distance <= 0.0 {
                    self.status = "Cannot start Liquify behind the camera.".into();
                    return;
                }
                let start_world = ray_origin + ray_direction * distance;
                let base_strength = self
                    .liquify_ui
                    .draft
                    .as_ref()
                    .map_or(0.75, |draft| draft.settings.strength);
                self.liquify_ui.gesture = Some(LiquifyGesture {
                    start_screen: pos,
                    last_screen: pos,
                    start_world,
                    plane_origin,
                    plane_normal,
                    base_strength,
                });
                self.update_liquify_gesture(pos, pressure, rect);
            }
            crate::input::Phase::Move => {
                if self.liquify_ui.gesture.is_some() {
                    if rect.contains(pos) {
                        self.update_liquify_gesture(pos, pressure, rect);
                    } else {
                        self.cancel_liquify_gesture();
                        self.status = "Liquify stroke cancelled outside the canvas.".into();
                    }
                }
            }
            crate::input::Phase::Up => {
                if self.liquify_ui.gesture.is_some() {
                    if rect.contains(pos) {
                        self.update_liquify_gesture(pos, pressure, rect);
                        self.liquify_ui.gesture = None;
                        self.status =
                            "Liquify preview ready. Apply to keep the drag, or Cancel to restore."
                                .into();
                    } else {
                        self.cancel_liquify_gesture();
                    }
                }
            }
            crate::input::Phase::Cancel => self.finish_liquify(false),
        }
    }

    fn update_liquify_gesture(&mut self, pos: Pos2, pressure: f32, rect: Rect) {
        let Some(gesture) = self.liquify_ui.gesture.as_mut() else {
            return;
        };
        let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
        let denominator = ray_direction.dot(gesture.plane_normal);
        if denominator.abs() < 1.0e-5 {
            return;
        }
        let distance = (gesture.plane_origin - ray_origin).dot(gesture.plane_normal) / denominator;
        if !distance.is_finite() || distance <= 0.0 {
            return;
        }
        let current_world = ray_origin + ray_direction * distance;
        gesture.last_screen = pos;
        if let Some(draft) = self.liquify_ui.draft.as_mut() {
            draft.settings.center = gesture.start_world;
            draft.settings.displacement = current_world - gesture.start_world;
            draft.settings.strength = pressure_strength(gesture.base_strength, pressure);
        }
        self.refresh_liquify();
    }

    pub(super) fn cancel_liquify_gesture(&mut self) {
        if self.liquify_ui.sculpt.is_some() {
            self.cancel_sculpt_gesture();
            return;
        }
        self.liquify_ui.gesture = None;
        let restore = self.liquify_ui.draft.as_mut().map(|draft| {
            let should_restore = draft.has_preview;
            let after = draft.after.clone();
            let before = draft.before.clone();
            let ids: HashSet<_> = before.iter().map(|(_, stroke)| stroke.id).collect();
            draft.settings.displacement = Vec3::ZERO;
            draft.settings.center = snapshot_center(&draft.before);
            draft.has_preview = false;
            draft.after = before.clone();
            (should_restore, after, before, ids)
        });
        if let Some((true, after, before, ids)) = restore {
            crate::editing::replace(&mut self.doc, &before, &after, false);
            self.renderer
                .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
        }
    }

    pub(super) fn cancel_liquify_for_navigation(&mut self) {
        if self.liquify_brush_active() && self.mouse_down {
            self.cancel_liquify_gesture();
        }
    }

    pub(super) fn paint_liquify_cursor(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        pointer: Option<Pos2>,
    ) -> Result<(), &'static str> {
        if self.liquify_ui.sculpt.is_some() {
            return self.paint_sculpt_cursor(painter, rect, pointer);
        }
        if !self.liquify_brush_active() {
            return Err("Liquify brush mode or draft is inactive");
        }
        let Some(draft) = self.liquify_ui.draft.as_ref() else {
            return Err("Liquify draft is missing");
        };
        let Some(pointer) = pointer.filter(|pos| rect.contains(*pos)) else {
            return Err("synthetic pointer is outside the canvas");
        };
        let (cursor_center, plane_origin) = self
            .liquify_ui
            .gesture
            .as_ref()
            .map_or((pointer, draft.settings.center), |gesture| {
                (gesture.start_screen, gesture.plane_origin)
            });
        let normal = self.camera.basis().2;
        let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pointer);
        let denominator = ray_direction.dot(normal);
        if denominator.abs() < 1.0e-5 {
            return Err("cursor ray is parallel to its working plane");
        }
        let distance = (plane_origin - ray_origin).dot(normal) / denominator;
        if distance <= 0.0 {
            return Err("working plane is behind the cursor ray");
        }
        let world = ray_origin + ray_direction * distance;
        let radius_pixels =
            draft.settings.radius / self.camera.world_per_pixel(world, rect.height());
        let painter = painter.with_clip_rect(rect);
        let label_color = super::ui_style::canvas_overlay_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        painter.circle_stroke(
            cursor_center,
            radius_pixels,
            egui::Stroke::new(1.5_f32, Color32::WHITE),
        );
        painter.circle_stroke(
            cursor_center,
            radius_pixels + 1.0,
            egui::Stroke::new(0.7_f32, Color32::BLACK),
        );
        let depth = (self.camera.eye() - plane_origin).dot(self.camera.basis().2);
        painter.text(
            cursor_center + Vec2::new(radius_pixels + 8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("r {:.2} wu · depth {depth:.2} wu", draft.settings.radius),
            egui::FontId::proportional(12.0),
            label_color,
        );
        if let Some(gesture) = &self.liquify_ui.gesture {
            painter.arrow(
                gesture.start_screen,
                gesture.last_screen - gesture.start_screen,
                egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 205, 50)),
            );
        }
        Ok(())
    }
}

fn snapshot_center(snapshot: &Snapshot) -> Vec3 {
    let (sum, count) = snapshot
        .iter()
        .flat_map(|(_, stroke)| stroke.points.iter())
        .fold((Vec3::ZERO, 0usize), |(sum, count), point| {
            (sum + point.position, count + 1)
        });
    if count == 0 {
        Vec3::ZERO
    } else {
        sum / count as f32
    }
}

fn pressure_strength(base: f32, pressure: f32) -> f32 {
    (base * pressure.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::pressure_strength;

    #[test]
    fn pen_pressure_scales_the_configured_liquify_strength() {
        assert_eq!(pressure_strength(0.8, 0.5), 0.4);
        assert_eq!(pressure_strength(0.8, 2.0), 0.8);
        assert_eq!(pressure_strength(0.8, -1.0), 0.0);
    }
}
