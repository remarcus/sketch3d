use super::*;

pub(super) struct PointDrag {
    origin: Vec3,
    start: Vec3,
    normal: Vec3,
    camera: Camera,
    rect: Rect,
}

impl PointDrag {
    fn position(&self, pos: Pos2) -> Option<Vec3> {
        crate::gizmo::plane_hit(&self.camera, self.rect, pos, self.origin, self.normal)
            .map(|hit| self.origin + hit - self.start)
    }
}

impl SketchApp {
    pub(super) fn preview_selected_cleanup(&mut self, spacing: f32, max_error: f32) {
        if self.selected.len() != 1 {
            self.status = "Select one ordinary stroke to preview cleanup.".into();
            return;
        }
        let id = *self.selected.iter().next().unwrap();
        match crate::point_edit::Draft::cleanup(&self.doc, id, spacing, max_error) {
            Ok(draft) => {
                self.point_drag = None;
                self.point_draft = Some(draft);
                self.presentation.preferences.layout.inspector_open = true;
                self.presentation.preferences.layout.inspector_tab =
                    crate::presentation::InspectorTab::Properties;
                self.brush_select_enabled = false;
                self.renderer.invalidate();
                self.status = "Stroke cleanup preview is detached from the saved drawing. Apply commits one edit; Cancel restores the source.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn canvas_point_shortcuts(&mut self, ctx: &egui::Context) {
        if self.point_draft.is_some() && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.finish_canvas_point_edit(false);
            ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, Key::Escape);
            });
        }
    }
    pub(super) fn verify_canvas_point_workflow(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.point_verification_frame else {
            return;
        };
        self.point_verification_frame = Some(frame + 1);
        self.tutorial.offer = false;
        self.recovery_prompt = false;
        self.show_help = false;
        let directory = std::env::current_dir()
            .unwrap()
            .join("verification/point-edit");
        std::fs::create_dir_all(&directory).unwrap();
        if frame > 3 {
            for event in ctx.input(|input| input.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let pixels: Vec<u8> = image
                        .pixels
                        .iter()
                        .flat_map(|pixel| pixel.to_array())
                        .collect();
                    image::save_buffer(
                        directory.join("canvas-point-preview.png"),
                        &pixels,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    std::fs::write(directory.join("report.json"), serde_json::to_vec_pretty(&serde_json::json!({
                        "passed": true, "physical_tablet_tested": false,
                        "checks": ["mouse picking and detached view-plane dragging in perspective and orthographic views", "cancel preserves document and history", "apply is one edit and undo/redo are exact", "native save/reload and GLB survive point edits", "split and ordered join each create one exact reversible edit", "bounded cleanup preview preserves source until Apply; Cancel, undo, and redo are exact", "cleanup survives native reload, fresh recovery scan, gradient/dab checks, and GLB export", "stale source preview is discarded", "preview handles and centerline screenshot"]
                    })).unwrap()).unwrap();
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
            assert!(frame < 90, "point preview screenshot timed out");
            ctx.request_repaint();
            return;
        }
        if frame < 3 {
            ctx.request_repaint();
            return;
        }
        fn drag(app: &mut SketchApp, ctx: &egui::Context, rect: Rect, start: Pos2, end: Pos2) {
            ctx.input_mut(|input| {
                input.events = vec![
                    egui::Event::PointerButton {
                        pos: start,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            });
            app.canvas_input(ctx, rect, Vec::new());
            ctx.input_mut(|input| input.events.clear());
        }
        self.cancel();
        self.tutorial.offer = false;
        self.recovery_prompt = false;
        self.select_mode = true;
        self.starter_scene = false;
        self.cylinder_example = false;
        self.eraser = false;
        self.last_pen_at = None;
        self.doc = Document::default();
        let mut source = crate::scenes::japanese_house().strokes.remove(0);
        source.id = 1;
        source.layer_id = 1;
        source.group_id = None;
        source.fill = None;
        source.curve = None;
        source.gradient = None;
        source.brush = crate::document::Brush::default();
        source.points.resize(3, source.points[0]);
        source.points.truncate(3);
        for (index, point) in source.points.iter_mut().enumerate() {
            point.position = Vec3::new((index as f32 - 1.0) * 0.5, 0.0, 0.0);
            point.time_ms = 0;
        }
        self.doc.strokes = vec![source];
        self.next_id = 2;
        self.selected = HashSet::from([1]);
        self.undo.clear();
        self.redo.clear();
        let original = self.doc.strokes.clone();
        for projection in [
            crate::geometry::Projection::Perspective,
            crate::geometry::Projection::Orthographic,
        ] {
            self.camera = Camera {
                target: Vec3::ZERO,
                projection,
                ..Camera::default()
            };
            self.begin_canvas_point_edit();
            let start = crate::gizmo::project(&self.camera, rect, Vec3::ZERO)
                .unwrap()
                .0;
            let end = start + egui::vec2(40.0, -25.0);
            assert!(
                canvas_pointer_allowed(ctx, rect, start),
                "point verifier canvas is covered by another UI layer"
            );
            drag(self, ctx, rect, start, end);
            assert!(self.point_drag.is_none());
            assert_eq!(self.doc.strokes, original);
            assert!(self.undo.is_empty());
            assert_ne!(
                self.point_draft.as_ref().unwrap().preview,
                self.point_draft.as_ref().unwrap().original
            );
            ctx.input_mut(|input| {
                input.events = vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: Some(Key::Escape),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            });
            self.canvas_point_shortcuts(ctx);
            ctx.input_mut(|input| input.events.clear());
            assert!(self.point_draft.is_none(), "Escape closes the preview");
            assert_eq!(self.doc.strokes, original);
            self.begin_canvas_point_edit();
            drag(self, ctx, rect, start, end);
            self.finish_canvas_point_edit(true);
            assert_eq!(self.undo.len(), 1);
            let edited = self.doc.strokes.clone();
            assert_ne!(edited, original);
            self.undo();
            assert_eq!(self.doc.strokes, original);
            self.redo();
            assert_eq!(self.doc.strokes, edited);
            let glb = crate::export::encode(&self.doc, None).unwrap().bytes;
            self.file = Some(directory.join("stroke.sketch.json"));
            assert!(self.save(false));
            let loaded =
                Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                    .unwrap();
            assert_eq!(loaded.strokes, edited);
            assert_eq!(crate::export::encode(&loaded, None).unwrap().bytes, glb);
            self.undo();
            assert_eq!(self.doc.strokes, original);
            self.redo.clear();
        }
        let mut split_source = original[0].clone();
        let template = original[0].points[0];
        split_source.points = (0..4)
            .map(|index| {
                let mut point = template;
                point.position = Vec3::new((index as f32 - 1.5) * 0.5, 0.0, 0.0);
                point.time_ms = index as u32 * 10;
                point
            })
            .collect();
        self.doc.strokes = vec![split_source];
        self.selected = HashSet::from([1]);
        self.undo.clear();
        self.redo.clear();
        let before_split = self.doc.strokes.clone();
        self.split_ordinary_stroke_after(1, 1);
        let split = self.doc.strokes.clone();
        assert_eq!(split.len(), 2, "split creates two editable strokes");
        assert_eq!(self.undo.len(), 1, "split is one history edit");
        self.undo();
        assert_eq!(self.doc.strokes, before_split, "split undo is exact");
        self.redo();
        assert_eq!(self.doc.strokes, split, "split redo is exact");
        self.join_ordinary_strokes(1, 2, 0.0);
        assert_eq!(
            self.doc.strokes, before_split,
            "joining the split restores source samples"
        );
        self.undo();
        assert_eq!(self.doc.strokes, split, "join undo restores both strokes");
        self.redo();
        assert_eq!(self.doc.strokes, before_split, "join redo is exact");
        let mut cleanup_source = original[0].clone();
        cleanup_source.gradient = Some(crate::document::StrokeGradient {
            start: [0.1, 0.2, 0.3],
            end: [0.8, 0.7, 0.6],
        });
        cleanup_source.brush.dabs.enabled = true;
        cleanup_source.brush.dabs.spacing = 0.35;
        cleanup_source.points = (0..101)
            .map(|index| {
                let t = index as f32 / 100.0;
                let mut point = template;
                point.position = Vec3::new(t, (t * std::f32::consts::TAU).sin() * 0.002, 0.0);
                point.pressure = 0.35 + 0.3 * t;
                point.tilt = Some(Vec3::Y);
                point.time_ms = index as u32 * 8;
                point
            })
            .collect();
        self.doc.strokes = vec![cleanup_source];
        self.selected = HashSet::from([1]);
        self.undo.clear();
        self.redo.clear();
        let before_cleanup = self.doc.strokes.clone();
        self.preview_selected_cleanup(0.2, 0.02);
        let first_cleanup_preview = self
            .point_draft
            .as_ref()
            .unwrap_or_else(|| panic!("cleanup preview opens: {}", self.status));
        assert!(first_cleanup_preview.cleanup_stats.is_some());
        assert!(first_cleanup_preview.preview.points.len() < before_cleanup[0].points.len());
        assert_eq!(
            self.doc.strokes, before_cleanup,
            "preview leaves source untouched"
        );
        self.finish_canvas_point_edit(false);
        assert_eq!(
            self.doc.strokes, before_cleanup,
            "cancel restores exact source"
        );
        assert!(self.undo.is_empty());
        self.preview_selected_cleanup(0.2, 0.02);
        let cleanup_preview = self.point_draft.as_ref().unwrap().preview.clone();
        self.finish_canvas_point_edit(true);
        assert_eq!(self.undo.len(), 1, "Apply creates one cleanup edit");
        assert_eq!(self.doc.strokes[0], cleanup_preview);
        assert_eq!(self.doc.strokes[0].gradient, before_cleanup[0].gradient);
        assert_eq!(
            self.doc.strokes[0].brush.dab_seed,
            before_cleanup[0].brush.dab_seed
        );
        let cleaned = self.doc.strokes.clone();
        self.undo();
        assert_eq!(self.doc.strokes, before_cleanup, "cleanup undo is exact");
        self.redo();
        assert_eq!(self.doc.strokes, cleaned, "cleanup redo is exact");
        let cleanup_glb = crate::export::encode(&self.doc, None).unwrap().bytes;
        self.file = Some(directory.join("cleanup.sketch.json"));
        assert!(self.save(false));
        let loaded_cleanup =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded_cleanup.strokes, cleaned);
        assert_eq!(
            crate::export::encode(&loaded_cleanup, None).unwrap().bytes,
            cleanup_glb
        );
        let cleanup_recovery_root = directory.join(format!(
            "cleanup-recovery-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cleanup_snapshot = crate::recovery_store::RecoverySnapshot::new(
            "stroke-cleanup-verification".into(),
            None,
            self.doc.clone(),
        );
        crate::recovery_store::write(&cleanup_recovery_root, &cleanup_snapshot).unwrap();
        let cleanup_recovered = crate::recovery_store::scan(
            &cleanup_recovery_root,
            "stroke-cleanup-verification-reader",
        )
        .unwrap();
        assert_eq!(
            cleanup_recovered.candidates[0].snapshot.document.strokes,
            cleaned
        );

        self.doc.strokes = original.clone();
        self.selected = HashSet::from([1]);
        self.undo.clear();
        self.redo.clear();
        self.begin_canvas_point_edit();
        self.doc.strokes[0].color = [0.4, 0.5, 0.6];
        self.sync_canvas_point_edit();
        assert!(self.point_draft.is_none());
        self.begin_canvas_point_edit();
        let start = crate::gizmo::project(&self.camera, rect, Vec3::ZERO)
            .unwrap()
            .0;
        drag(self, ctx, rect, start, start + egui::vec2(50.0, -50.0));
        self.renderer.invalidate();
        self.status =
            "Point editing verified; detached preview shown. Apply commits; Escape cancels.".into();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        ctx.request_repaint();
    }

    pub(super) fn begin_canvas_point_edit(&mut self) {
        if self.selected.len() != 1 {
            return;
        }
        self.finish();
        let id = *self.selected.iter().next().unwrap();
        match crate::point_edit::Draft::new(&self.doc, id) {
            Ok(draft) => {
                self.point_draft = Some(draft);
                self.presentation.preferences.layout.inspector_open = true;
                self.presentation.preferences.layout.inspector_tab =
                    crate::presentation::InspectorTab::Properties;
                self.brush_select_enabled = false;
                self.status = "Drag a point in the view plane. Centerline preview only; Apply commits, Escape cancels.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn finish_canvas_point_edit(&mut self, apply: bool) {
        self.point_drag = None;
        let Some(draft) = self.point_draft.take() else {
            return;
        };
        let was_cleanup = draft.cleanup_stats.is_some();
        if !apply {
            self.status = if was_cleanup {
                "Cleanup preview cancelled; the drawing is unchanged.".into()
            } else {
                "Point editing cancelled; the drawing is unchanged.".into()
            };
            return;
        }
        match draft.edit(&self.doc) {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.renderer.invalidate_ids(&self.selected);
                self.dirty = true;
                self.record_recovery_edit();
                self.status = if was_cleanup {
                    "Stroke cleanup applied as one undo step.".into()
                } else {
                    "Point changes applied as one undo step.".into()
                };
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn sync_canvas_point_edit(&mut self) {
        if self.point_draft.as_ref().is_some_and(|draft| {
            !self.select_mode
                || self.selected.len() != 1
                || !self.selected.contains(&draft.original.id)
                || !draft.valid_for(&self.doc)
                || self.liquify_active()
                || self.mixed_resource_preview_active()
                || self.appearance_draft.is_some()
                || self.repeat.is_some()
                || self.curve_draft_active()
        }) {
            self.finish_canvas_point_edit(false);
        }
    }

    pub(super) fn canvas_point_pointer(&mut self, pos: Pos2, rect: Rect, begin: bool) {
        let Some(draft) = self.point_draft.as_mut() else {
            return;
        };
        if draft.cleanup_stats.is_some() {
            return;
        }
        if begin {
            self.point_drag = None;
            let hit = nearest_point(&draft.preview, &self.camera, rect, pos);
            if let Some(index) = hit {
                draft.selected = index;
                let origin = draft.preview.points[index].position;
                let normal = self.camera.basis().2;
                if let Some(start) =
                    crate::gizmo::plane_hit(&self.camera, rect, pos, origin, normal)
                {
                    self.point_drag = Some(PointDrag {
                        origin,
                        start,
                        normal,
                        camera: self.camera,
                        rect,
                    });
                }
            }
        } else if let Some(drag) = &self.point_drag {
            if let Some(position) = drag.position(pos) {
                if let Err(error) = draft.move_selected(position) {
                    self.status = error;
                }
            }
        }
    }

    pub(super) fn canvas_point_controls(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.point_draft.as_mut() else {
            return;
        };
        let is_cleanup = draft.cleanup_stats.is_some();
        ui.heading(if is_cleanup {
            "Preview stroke cleanup"
        } else {
            "Edit stroke points"
        });
        if let Some(stats) = draft.cleanup_stats {
            ui.small(format!(
                "{} → {} samples · max path deviation {:.4} wu · path length {:.3} → {:.3} wu · {} μs local CPU. Timed Airbrush strokes are preserved unchanged.",
                stats.input_points,
                stats.output_points,
                stats.maximum_deviation,
                stats.source_length,
                stats.result_length,
                stats.elapsed_micros
            ));
            ui.small(
                "The highlighted centerline is a detached preview; Apply creates one undo step.",
            );
        } else {
            ui.small("Click a point, then drag in the view plane. Handles include points behind artwork. Orbit to inspect depth. The line is a centerline preview; the rendered stroke changes on Apply.");
            ui.horizontal(|ui| {
                ui.label("Point (0-based)");
                ui.add(
                    egui::DragValue::new(&mut draft.selected)
                        .range(0..=draft.preview.points.len() - 1),
                );
            });
            let mut coordinates = draft.preview.points[draft.selected].position.to_array();
            let mut changed = false;
            for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                changed |= super::ui_style::numeric_row(
                    ui,
                    label,
                    ("canvas-point", axis),
                    &mut coordinates[axis],
                    Some(-10000.0..=10000.0),
                    "wu",
                )
                .changed();
            }
            if changed {
                if let Err(error) = draft.move_selected(Vec3::from_array(coordinates)) {
                    self.status = error;
                }
            }
            if ui
                .add_enabled(
                    draft.preview.points.len() > 2,
                    egui::Button::new("Delete selected point"),
                )
                .clicked()
            {
                self.point_drag = None;
                if let Err(error) = draft.delete_selected() {
                    self.status = error;
                }
            }
        }
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui
                .add_enabled(
                    draft.preview != draft.original,
                    egui::Button::new(if is_cleanup {
                        "Apply cleanup"
                    } else {
                        "Apply point edits"
                    }),
                )
                .clicked();
            cancel = ui
                .button(if is_cleanup {
                    "Cancel cleanup"
                } else {
                    "Cancel point edits"
                })
                .clicked();
        });
        if apply || cancel {
            self.finish_canvas_point_edit(apply);
        }
    }

    pub(super) fn canvas_point_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(draft) = &self.point_draft else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let accent = super::ui_style::canvas_selection_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        let projected: Vec<_> = draft
            .preview
            .points
            .iter()
            .map(|point| crate::gizmo::project(&self.camera, rect, point.position))
            .collect();
        for pair in projected.windows(2) {
            if let [Some((a, _)), Some((b, _))] = pair {
                painter.line_segment([*a, *b], egui::Stroke::new(2.0_f32, accent));
            }
        }
        for (index, point) in projected.iter().enumerate() {
            if let Some((pos, _)) = point {
                if rect.contains(*pos) {
                    painter.circle_filled(
                        *pos,
                        if index == draft.selected { 5.0 } else { 2.5 },
                        accent,
                    );
                    if index == draft.selected {
                        painter.circle_stroke(*pos, 8.0, egui::Stroke::new(1.5_f32, accent));
                    }
                }
            }
        }
    }
}

fn nearest_point(stroke: &Stroke, camera: &Camera, rect: Rect, pos: Pos2) -> Option<usize> {
    stroke
        .points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let (screen, depth) = crate::gizmo::project(camera, rect, point.position)?;
            let distance = screen.distance_sq(pos);
            (rect.contains(screen) && distance <= 100.0).then_some((index, distance, depth))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.0.cmp(&b.0))
        })
        .map(|hit| hit.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_picking_and_view_plane_drag_work_in_both_projections() {
        let mut source = crate::scenes::japanese_house().strokes.remove(0);
        source.points.truncate(3);
        source.points.resize(3, source.points[0]);
        assert_eq!(source.points.len(), 3);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0));
        for projection in [
            crate::geometry::Projection::Perspective,
            crate::geometry::Projection::Orthographic,
        ] {
            let camera = Camera {
                projection,
                ..Camera::default()
            };
            let (right, up, normal) = camera.basis();
            for (index, point) in source.points.iter_mut().enumerate() {
                point.position = camera.target + right * (index as f32 - 1.0) * 0.4;
            }
            let origin = source.points[1].position;
            let screen = crate::gizmo::project(&camera, rect, origin).unwrap().0;
            assert_eq!(nearest_point(&source, &camera, rect, screen), Some(1));
            assert_eq!(
                nearest_point(&source, &camera, rect, screen + egui::vec2(0.0, 30.0)),
                None
            );
            let start_pos = screen + egui::vec2(3.0, 0.0);
            let start = crate::gizmo::plane_hit(&camera, rect, start_pos, origin, normal).unwrap();
            let drag = PointDrag {
                origin,
                start,
                normal,
                camera,
                rect,
            };
            assert!(drag.position(start_pos).unwrap().distance(origin) < 0.0001);
            let end = drag.position(start_pos + egui::vec2(30.0, -20.0)).unwrap();
            assert!((end - origin).dot(normal).abs() < 0.0001);
            assert!((end - origin).dot(right) > 0.0 && (end - origin).dot(up) > 0.0);
            source.points[0].position = origin - normal * 0.2;
            source.points[2].position = origin;
            assert_eq!(nearest_point(&source, &camera, rect, screen), Some(1));
        }
    }

    #[test]
    fn split_and_join_update_canvas_point_picking() {
        let mut doc = crate::scenes::japanese_house();
        let mut source = doc.strokes.remove(0);
        source.points.truncate(4);
        source.points.resize(4, source.points[0]);
        for (index, point) in source.points.iter_mut().enumerate() {
            point.position = Vec3::new(index as f32, 0.0, 0.0);
        }
        source.id = 1;
        source.layer_id = 1;
        source.group_id = None;
        source.fill = None;
        source.curve = None;
        source.brush = Default::default();
        doc.strokes = vec![source.clone()];
        let camera = Camera {
            projection: crate::geometry::Projection::Orthographic,
            target: Vec3::new(1.5, 0.0, 0.0),
            ..Camera::default()
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0));
        let split = crate::point_edit::split_stroke(&doc, 1, 1, 2).unwrap();
        split.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 2);
        let (second_screen, _) =
            crate::gizmo::project(&camera, rect, Vec3::new(3.0, 0.0, 0.0)).unwrap();
        assert_eq!(
            nearest_point(&doc.strokes[1], &camera, rect, second_screen),
            Some(2)
        );
        let join = crate::point_edit::join_strokes(&doc, 1, 2, 0.0).unwrap();
        join.apply(&mut doc, true);
        let (screen, _) = crate::gizmo::project(&camera, rect, Vec3::new(2.0, 0.0, 0.0)).unwrap();
        assert_eq!(
            nearest_point(&doc.strokes[0], &camera, rect, screen),
            Some(2)
        );
        assert_eq!(doc.strokes[0].points, source.points);
    }
}
