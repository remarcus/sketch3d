use super::ui_style;
use super::*;
use crate::{editing, export_check, geometry, gizmo};

impl SketchApp {
    pub(super) fn brush_controls(&mut self, ui: &mut egui::Ui) {
        let before = self.brush;
        let image_tip = self.brush.tip_asset_id != 0;
        let strand_mode = self.brush.strands.enabled;
        ui_style::section(ui, "Brush shape");
        if image_tip {
            let aspect = self
                .doc
                .brush_tip_assets
                .iter()
                .find(|asset| asset.id == self.brush.tip_asset_id)
                .map_or(self.brush.tip_aspect, |asset| {
                    asset.size[0] as f32 / asset.size[1] as f32
                });
            ui.small(format!(
                "PNG image stamp · aspect {:.2} · source alpha kept · brush color tints RGB",
                aspect
            ));
            ui.small("Image stamps currently draw on planar targets; curved and model surfaces are rejected.");
            ui_style::slider_row(
                ui,
                "Angle",
                "brush-angle",
                &mut self.brush.angle_degrees,
                -180.0..=180.0,
                1.0,
                "°",
            );
        } else if strand_mode {
            ui.small("Parallel round strands follow the captured guide and stroke frame.");
            ui_style::slider_row(
                ui,
                "Angle",
                "brush-angle",
                &mut self.brush.angle_degrees,
                -180.0..=180.0,
                1.0,
                "°",
            );
        } else {
            ui_style::field_row(ui, "Shape", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (kind, label) in [
                        (BrushKind::Round, "Round"),
                        (BrushKind::Ribbon, "Ribbon"),
                        (BrushKind::Chisel, "Flat / chisel"),
                    ] {
                        ui.radio_value(&mut self.brush.kind, kind, label);
                    }
                });
            });
        }
        if !image_tip && self.brush.kind == BrushKind::Chisel && before.kind != BrushKind::Chisel {
            self.brush.softness = 0.0;
        }
        if !image_tip && self.brush.kind != BrushKind::Round {
            ui_style::slider_row(
                ui,
                "Angle",
                "brush-angle",
                &mut self.brush.angle_degrees,
                -180.0..=180.0,
                1.0,
                "°",
            );
            if self.brush.kind == BrushKind::Chisel {
                ui_style::slider_row(
                    ui,
                    "Thickness",
                    "brush-thickness",
                    &mut self.brush.thickness,
                    0.02..=1.0,
                    100.0,
                    "%",
                )
                .on_hover_text("Thickness as a percentage of brush width.");
            }
            ui.small("Orientation starts from the guide, then follows the stroke. Orbit to view it from either side.");
        }
        if !image_tip {
            self.texture_controls(ui);
        }
        self.dynamics_controls(ui);
        ui.collapsing("Bristle and rake", |ui| {
            let enabled = ui
                .add_enabled(
                    !image_tip && (!self.brush.dabs.enabled || self.brush.strands.enabled),
                    egui::Checkbox::new(&mut self.brush.strands.enabled, "Use parallel strands"),
                )
                .on_hover_text("Builds one editable stroke from separated, capped ribbons.");
            if enabled.changed() && self.brush.strands.enabled {
                self.brush.dabs.enabled = false;
            }
            if self.brush.strands.enabled {
                ui.add(
                    egui::Slider::new(&mut self.brush.strands.count, 2..=16)
                        .text("Strands"),
                );
                ui_style::slider_row(
                    ui,
                    "Spread",
                    "strand-spread",
                    &mut self.brush.strands.spread,
                    0.25..=2.5,
                    100.0,
                    "× radius",
                );
                ui_style::slider_row(
                    ui,
                    "Thickness",
                    "strand-thickness",
                    &mut self.brush.strands.thickness,
                    0.05..=1.0,
                    100.0,
                    "% lane pitch",
                );
                ui_style::slider_row(
                    ui,
                    "Tilt spread",
                    "strand-tilt-spread",
                    &mut self.brush.strands.tilt_spread,
                    0.0..=1.0,
                    100.0,
                    "%",
                )
                .on_hover_text("Missing tilt keeps the saved spread; tilt can widen the strand fan by up to this amount.");
                ui.small("Strands use pressure for width and opacity. Geometry is limited to 400,000 vertices per stroke.");
            }
        });
        ui.collapsing("Dab placement", |ui| {
            ui.add_enabled_ui(!image_tip && !self.brush.strands.enabled, |ui| {
                ui.checkbox(&mut self.brush.dabs.enabled, "Use deterministic dabs")
                    .on_hover_text("Places marks at fixed world-space distances along the stroke.");
            });
            if self.brush.dabs.enabled {
                ui_style::slider_row(
                    ui,
                    "Spacing",
                    "dab-spacing",
                    &mut self.brush.dabs.spacing,
                    0.1..=4.0,
                    100.0,
                    "× radius",
                );
                ui.small("Lower spacing places more dabs along the same path length.");
                ui_style::slider_row(
                    ui,
                    "Scatter",
                    "dab-scatter",
                    &mut self.brush.dabs.scatter,
                    0.0..=4.0,
                    100.0,
                    "× radius",
                )
                .on_hover_text("Scatter is available on planar targets. Curved and model surfaces keep dab centers on the surface path.");
                ui_style::slider_row(
                    ui,
                    "Size variation",
                    "dab-size-variation",
                    &mut self.brush.dabs.size_variation,
                    0.0..=0.95,
                    100.0,
                    "%",
                );
                ui_style::slider_row(
                    ui,
                    "Angle variation",
                    "dab-angle-variation",
                    &mut self.brush.dabs.angle_variation_degrees,
                    0.0..=180.0,
                    1.0,
                    "°",
                );
                ui.checkbox(
                    &mut self.brush.dabs.pressure_size.enabled,
                    "Pressure controls dab size",
                );
                if self.brush.dabs.pressure_size.enabled {
                    ui_style::slider_row(
                        ui,
                        "Minimum size",
                        "dab-pressure-minimum",
                        &mut self.brush.dabs.pressure_size.minimum,
                        0.0..=1.0,
                        100.0,
                        "%",
                    );
                    ui_style::slider_row(
                        ui,
                        "Pressure response",
                        "dab-pressure-curve",
                        &mut self.brush.dabs.pressure_size.curve,
                        0.3..=3.0,
                        100.0,
                        "",
                    );
                }
                ui.small("Spacing is measured in world units from the stroke radius. Taps place one dab; pen-up does not add an extra endpoint dab.");
            }
        });
        ui_style::component_showcase(ui);
        if self.brush != before {
            self.finish();
        }
    }

    pub(super) fn load_brushes(&mut self) {
        self.reset_recovery_document();
        self.cancel();
        self.doc = export_check::brush_fixture();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.camera = Camera::default();
        self.camera.distance = 8.5;
        self.guide = DrawingPlane::default();
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = false;
        self.starter_scene = false;
        self.cylinder_example = false;
        self.file = None;
        self.dirty = false;
        self.select_mode = false;
        self.eraser = false;
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.renderer.invalidate();
        self.status = "Brush sampler / top: Round, middle: Ribbon, bottom: Flat/chisel. Editable pressure curves and dots.".into();
    }

    pub(super) fn verify_brushes(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.brush_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.dirty = false;
                self.request(Pending::New);
                self.stabilization = 0.0;
                self.camera.align_guide(self.guide);
                for (i, kind) in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel]
                    .into_iter()
                    .enumerate()
                {
                    self.brush.kind = kind;
                    self.brush.angle_degrees = 25.0;
                    self.radius = 0.1;
                    let start = rect.center() + egui::vec2(-120.0, (i as f32 - 1.0) * 80.0);
                    self.add_point(start, 0.15, rect, true);
                    self.add_point(start + egui::vec2(120.0, -30.0), 1.0, rect, false);
                    self.add_point(start + egui::vec2(240.0, 0.0), 0.4, rect, false);
                    self.finish();
                    let stroke = self.doc.strokes.last().unwrap().clone();
                    assert_eq!(stroke.brush.kind, kind);
                    assert!(stroke.brush.normal.abs_diff_eq(self.guide.basis().2, 1e-6));
                    let midpoint = stroke.points[1].position;
                    let screen = gizmo::project(&self.camera, rect, midpoint).unwrap().0;
                    assert_eq!(
                        gizmo::pick(&self.doc, &self.camera, rect, screen),
                        Some(stroke.id)
                    );
                    self.erase(screen, rect);
                    assert!(!self.doc.strokes.iter().any(|s| s.id == stroke.id));
                    self.undo();
                    assert_eq!(self.doc.strokes.last().unwrap(), &stroke);
                    self.selected = HashSet::from([stroke.id]);
                    self.duplicate_selected();
                    assert_eq!(self.doc.strokes.last().unwrap().brush, stroke.brush);
                    self.undo();
                    assert_eq!(self.doc.strokes.last().unwrap(), &stroke);
                    self.redo();
                    self.undo();
                    self.selected = HashSet::from([stroke.id]);
                    let before = editing::snapshot(&self.doc, &self.selected);
                    let after = editing::transformed(
                        &before,
                        Vec3::ZERO,
                        Vec3::ZERO,
                        glam::Quat::from_rotation_x(0.5),
                        1.4,
                    )
                    .unwrap();
                    let edit = Edit::Change {
                        before: before.clone(),
                        after,
                    };
                    edit.apply(&mut self.doc, true);
                    assert!(
                        (self.doc.strokes.last().unwrap().radius - stroke.radius * 1.4).abs()
                            < 1e-6
                    );
                    assert_eq!(
                        self.doc.strokes.last().unwrap().brush.thickness,
                        stroke.brush.thickness
                    );
                    edit.apply(&mut self.doc, false);
                    assert_eq!(editing::snapshot(&self.doc, &self.selected), before);
                }
                let mesh = geometry::stroke_mesh(&self.doc.strokes[1]);
                self.camera.yaw += 1.0;
                assert_eq!(
                    mesh.iter().map(|v| v.position).collect::<Vec<_>>(),
                    geometry::stroke_mesh(&self.doc.strokes[1])
                        .iter()
                        .map(|v| v.position)
                        .collect::<Vec<_>>()
                );
                // Exercise captured cylinder frames and the stabilized release tail.
                self.use_cylinder = true;
                self.cylinder = Cylinder {
                    center: Vec3::ZERO,
                    ..Cylinder::default()
                };
                self.camera = Camera {
                    yaw: 0.0,
                    pitch: 0.0,
                    ..Camera::default()
                };
                self.stabilization = 0.8;
                for kind in [BrushKind::Ribbon, BrushKind::Chisel] {
                    self.brush.kind = kind;
                    for (i, angle) in [-0.4, 0.0, 0.4].into_iter().enumerate() {
                        let world = self.cylinder.point(angle, 0.3 * i as f32);
                        let screen = gizmo::project(&self.camera, rect, world).unwrap().0;
                        self.add_point(screen, 0.3 + i as f32 * 0.3, rect, i == 0);
                    }
                    self.finish();
                    let stroke = self.doc.strokes.last().unwrap();
                    assert!(stroke.brush.is_valid());
                    assert!(stroke.points.iter().all(|p| {
                        let local = self.cylinder.to_local(p.position);
                        ((local.x * local.x + local.z * local.z).sqrt() - self.cylinder.radius)
                            .abs()
                            < 1e-4
                    }));
                    assert!(!geometry::stroke_mesh(stroke).is_empty());
                }
                // Changing the current guide cannot rotate existing brush frames.
                let original = self.doc.strokes.clone();
                self.guide = DrawingPlane::from_preset(Plane::Side);
                assert_eq!(self.doc.strokes, original);
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("brushes.sketch.json"));
                assert!(self.save(false));
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.version, 17);
                assert_eq!(loaded.strokes, original);
                self.load_brushes();
                self.brush = Brush {
                    kind: BrushKind::Chisel,
                    angle_degrees: 35.0,
                    ..Brush::default()
                };
                self.radius = 0.1;
                self.stabilization = 0.35;
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Brush screenshot timed out"),
            _ => {}
        }
        self.brush_verification_frame = Some(frame + 1);
    }
}
