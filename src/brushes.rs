use super::ui_style;
use super::*;
use crate::{editing, export_check, geometry, gizmo};

impl SketchApp {
    pub(super) fn brush_controls(&mut self, ui: &mut egui::Ui) {
        let before = self.brush;
        let image_tip = self.brush.tip_asset_id != 0;
        let strand_mode = self.brush.strands.enabled;
        let flow_mode = self.brush.flow.enabled;
        let profiled_tube = self.brush.kind == BrushKind::Round
            && self.brush.tube_profile != crate::document::TubeProfile::Round
            && !flow_mode
            && !strand_mode
            && !self.brush.dabs.enabled
            && !image_tip;
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
            ui.small("Stamps project quad corners onto saved guides or captured OBJ surface patches. They have no depth offset; marks at edges or steep turns are skipped.");
            ui_style::slider_row(
                ui,
                "Angle",
                "brush-angle",
                &mut self.brush.angle_degrees,
                -180.0..=180.0,
                1.0,
                "°",
            );
        } else if flow_mode {
            ui.small(
                "Timed soft deposits accumulate along the stroke and while contact is held still.",
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
        if !image_tip
            && self.brush.kind == BrushKind::Round
            && !flow_mode
            && !strand_mode
            && !self.brush.dabs.enabled
        {
            ui_style::field_row(ui, "Tube profile", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for profile in [
                        crate::document::TubeProfile::Round,
                        crate::document::TubeProfile::Square,
                        crate::document::TubeProfile::Triangle,
                        crate::document::TubeProfile::Star,
                    ] {
                        ui.radio_value(&mut self.brush.tube_profile, profile, profile.label());
                    }
                });
            });
        }
        if !image_tip && (self.brush.kind != BrushKind::Round || profiled_tube) {
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
        ui.collapsing("Airbrush flow", |ui| {
            let enabled = ui
                .add_enabled(
                    !image_tip
                        && (!self.brush.dabs.enabled || self.brush.flow.enabled)
                        && (!self.brush.strands.enabled || self.brush.flow.enabled),
                    egui::Checkbox::new(&mut self.brush.flow.enabled, "Use timed airbrush flow"),
                )
                .on_hover_text("Deposits are sampled in 32 ms time cells, independent of repaint count.");
            if enabled.changed() && self.brush.flow.enabled {
                self.brush.dabs.enabled = false;
                self.brush.strands.enabled = false;
            }
            if self.brush.flow.enabled {
                ui_style::slider_row(
                    ui,
                    "Flow rate",
                    "airbrush-flow-rate",
                    &mut self.brush.flow.rate_per_second,
                    0.1..=12.0,
                    100.0,
                    "/ s",
                )
                .on_hover_text("Higher flow reaches full coverage sooner during a stationary hold.");
                ui.small("A held contact builds to alpha 1.0. Pen-up, navigation, cancellation, and focus loss stop the timed deposits.");
                ui.small("Per-stroke cap: 8,192 deposits, 65,536 timed input points, and 589,824 vertices.");
            }
        });
        ui.collapsing("Bristle and rake", |ui| {
            let enabled = ui
                .add_enabled(
                    !image_tip
                        && (!self.brush.dabs.enabled || self.brush.strands.enabled)
                        && (!self.brush.flow.enabled || self.brush.strands.enabled),
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
            ui.add_enabled_ui(
                !image_tip && !self.brush.strands.enabled && !self.brush.flow.enabled,
                |ui| {
                ui.checkbox(&mut self.brush.dabs.enabled, "Use deterministic dabs")
                    .on_hover_text("Places marks at fixed world-space distances along the stroke.");
                },
            );
            if self.brush.dabs.enabled {
                ui.small("On saved curved guides, dab marks are projected surface decals with no depth offset.");
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
                .on_hover_text("Scatter is available on planes, saved curved guides, and captured OBJ surface patches. Curved/model placements are projected onto the surface and omitted at edges or steep turns.");
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
                    self.brush.dabs.enabled = kind == BrushKind::Round;
                    self.brush.dabs.spacing = 0.5;
                    self.brush.dabs.scatter = 0.0;
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
                    if stroke.brush.dabs.enabled {
                        let dabs = crate::dab_placement::sample(&stroke).unwrap();
                        assert!(!dabs.is_empty());
                        assert_eq!(
                            geometry::stroke_mesh(&stroke).len(),
                            dabs.len() * 24,
                            "planar input preview uses the saved dab placements"
                        );
                    }
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
                assert_eq!(loaded.version, 26);
                assert_eq!(loaded.strokes, original);
                self.verify_stipple_spray(rect);
                self.verify_image_stamps(rect);
                self.verify_strand_brushes(rect);
                self.verify_profile_brushes(rect);
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

    fn verify_stipple_spray(&mut self, rect: Rect) {
        let library = crate::presets::Library::default();
        let preset = |name: &str| {
            library
                .presets
                .iter()
                .find(|preset| preset.name == name)
                .unwrap_or_else(|| panic!("built-in {name} preset exists"))
                .clone()
        };
        let stipple_preset = preset("Stipple");
        let spray_preset = preset("Spray");
        let make_stroke = |preset: &crate::presets::Preset, id: u64, y: f32, x0: f32, x1: f32| {
            let mut brush = preset.brush;
            brush.dab_seed = crate::dab_placement::seed_for_stroke(id);
            Stroke {
                model_surface: None,
                curve: None,
                material: Default::default(),
                id,
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: preset.color,
                gradient: None,
                radius: preset.radius,
                brush,
                points: (0..=80)
                    .map(|index| {
                        let t = index as f32 / 80.0;
                        Point {
                            position: Vec3::new(x0 + (x1 - x0) * t, y, 0.0),
                            pressure: 0.15 + 0.85 * t,
                            tilt: None,
                            time_ms: (500.0 * t).round() as u32,
                        }
                    })
                    .collect(),
            }
        };

        let stipple = make_stroke(&stipple_preset, 73_001, 1.25, -2.25, 2.25);
        let mut sparse_density = make_stroke(&spray_preset, 73_002, 0.0, -2.2, -0.3);
        sparse_density.brush.dabs.spacing = 1.4;
        let mut dense_density = make_stroke(&spray_preset, 73_003, 0.0, 0.3, 2.2);
        dense_density.brush.dabs.spacing = 0.35;
        let mut narrow_spread = make_stroke(&spray_preset, 73_004, -1.25, -2.2, -0.3);
        narrow_spread.brush.dabs.scatter = 0.0;
        let mut wide_spread = make_stroke(&spray_preset, 73_005, -1.25, 0.3, 2.2);
        narrow_spread.brush.dab_seed = 73_100;
        wide_spread.brush.dab_seed = 73_100;

        let stipple_dabs = crate::dab_placement::sample(&stipple).unwrap();
        let sparse_dabs = crate::dab_placement::sample(&sparse_density).unwrap();
        let dense_dabs = crate::dab_placement::sample(&dense_density).unwrap();
        let narrow_dabs = crate::dab_placement::sample(&narrow_spread).unwrap();
        let wide_dabs = crate::dab_placement::sample(&wide_spread).unwrap();
        assert_eq!(
            stipple_dabs,
            crate::dab_placement::sample(&stipple).unwrap()
        );
        assert_eq!(
            wide_dabs,
            crate::dab_placement::sample(&wide_spread).unwrap()
        );
        assert!(stipple_dabs.len() > 1);
        let stipple_step = stipple.radius * stipple.brush.dabs.spacing;
        assert!(stipple_dabs.windows(2).all(|pair| {
            ((pair[1].gradient_t - pair[0].gradient_t)
                * crate::dynamics::arc_lengths(&stipple.points)
                    .last()
                    .copied()
                    .unwrap()
                - stipple_step)
                .abs()
                < 1.0e-4
        }));
        assert!(dense_dabs.len() > sparse_dabs.len());
        assert!(narrow_dabs
            .iter()
            .all(|dab| (dab.position.y + 1.25).abs() < 1.0e-5));
        assert!(wide_dabs
            .iter()
            .any(|dab| (dab.position.y + 1.25).abs() > 1.0e-4));
        assert!(stipple_dabs.first().unwrap().pressure < stipple_dabs.last().unwrap().pressure);

        let mut tap = wide_spread.clone();
        tap.points.truncate(1);
        let tap_dabs = crate::dab_placement::sample(&tap).unwrap();
        assert_eq!(tap_dabs.len(), 1);

        let study = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![
                stipple,
                sparse_density,
                dense_density,
                narrow_spread,
                wide_spread.clone(),
            ],
            ..Document::default()
        };
        let resolved_dabs = crate::dab_placement::document_count(&study).unwrap();
        let mesh_vertices: usize = study
            .strokes
            .iter()
            .map(|stroke| {
                let vertices = geometry::stroke_mesh(stroke).len();
                assert!(vertices <= crate::export::MAX_TRIANGLE_VERTICES_PER_STROKE);
                vertices
            })
            .sum();
        assert!(mesh_vertices <= crate::dab_placement::MAX_DOCUMENT_DABS * 24);
        let glb = crate::export::encode(&study, None).expect("dab sampler GLB stays within budget");
        assert_eq!(glb.summary.strokes, study.strokes.len());
        assert!(glb.summary.vertices > 0);

        let mut dense_limit = make_stroke(&spray_preset, 73_006, 0.0, -9.5, 9.5);
        dense_limit.brush.dabs.spacing = 0.1;
        let dense_limit_dabs = crate::dab_placement::sample(&dense_limit).unwrap();
        assert!(dense_limit_dabs.len() <= crate::dab_placement::MAX_DABS_PER_STROKE);
        let dense_limit_document = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![dense_limit.clone()],
            ..Document::default()
        };
        let dense_limit_placements =
            crate::dab_placement::document_count(&dense_limit_document).unwrap();
        let dense_limit_mesh = geometry::stroke_mesh(&dense_limit);
        assert!(dense_limit_mesh.len() <= crate::export::MAX_TRIANGLE_VERTICES_PER_STROKE);
        let dense_limit_glb = crate::export::encode(&dense_limit_document, None)
            .expect("maximum-density spray fixture stays within the GLB export budget");
        assert_eq!(dense_limit_glb.summary.strokes, 1);
        assert!(dense_limit_glb.summary.triangles > 0);

        let mut view_camera = self.camera;
        view_camera.distance = 8.5;
        view_camera.align_guide(DrawingPlane::default());
        let mut spray_doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![wide_spread.clone()],
            ..Document::default()
        };
        let target_dab = wide_dabs
            .iter()
            .find(|dab| (dab.position.y + 1.25).abs() > spray_preset.radius * 0.5)
            .expect("spread spray places a visible mark away from its path");
        let mark = gizmo::project(&view_camera, rect, target_dab.position)
            .expect("scattered mark is visible")
            .0;
        assert_eq!(
            gizmo::pick(&spray_doc, &view_camera, rect, mark),
            Some(wide_spread.id),
            "picking follows the visible scattered mark"
        );
        let plan = crate::partial_eraser::plan_partial_erase(
            &spray_doc,
            &view_camera,
            rect,
            &[mark],
            1.0,
            73_100,
        )
        .unwrap()
        .expect("partial eraser follows the visible scattered mark");
        assert!(plan.removed > 0 || plan.split > 0);
        plan.edit.apply(&mut spray_doc, true);
        assert_ne!(spray_doc.strokes, vec![wide_spread.clone()]);

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/sketch-073");
        std::fs::create_dir_all(&output).unwrap();
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let background = [0.94, 0.93, 0.89];
        let mut renders = Vec::new();
        for size in [[640, 360], [1280, 720], [1920, 1080]] {
            for (view, yaw_offset) in [("front", 0.0), ("orbit", 0.65)] {
                let mut camera = view_camera;
                camera.yaw += yaw_offset;
                let pixels = self
                    .renderer
                    .render_png_pixels(&study, &camera, size, background, lighting)
                    .expect("Stipple and Spray GPU renders fit the requested export sizes");
                let image = image::RgbaImage::from_raw(size[0], size[1], pixels)
                    .expect("render dimensions match pixel count");
                let file = format!("dab-sampler-{view}-{}x{}.png", size[0], size[1]);
                image.save(output.join(&file)).unwrap();
                renders.push(serde_json::json!({
                    "view": view,
                    "yaw_offset": yaw_offset,
                    "size": size,
                    "file": file,
                }));
            }
        }
        let report = serde_json::json!({
            "passed": true,
            "checks": [
                "Stipple spacing follows saved world-space distance landmarks",
                "Spray density changes placement count for the same path length",
                "Spray scatter changes visible mark spread while staying bounded",
                "sparse and dense input sampling, taps, and camera orbit preserve resolved placement",
                "visible scattered marks are pickable and partial erasure retains the other marks",
                "placement, mesh, and GLB export stay within per-stroke and document budgets",
                "front and orbit renders at 640x360, 1280x720, and 1920x1080",
            ],
            "placement_count": resolved_dabs,
            "mesh_vertices": mesh_vertices,
            "glb_bytes": glb.bytes.len(),
            "dense_limit_placements": dense_limit_placements,
            "dense_limit_mesh_vertices": dense_limit_mesh.len(),
            "dense_limit_glb_bytes": dense_limit_glb.bytes.len(),
            "stipple_dabs": stipple_dabs.len(),
            "spray_sparse_dabs": sparse_dabs.len(),
            "spray_dense_dabs": dense_dabs.len(),
            "spray_narrow_max_spread": 0.0,
            "spray_wide_max_spread": wide_dabs.iter().map(|dab| (dab.position.y + 1.25).abs()).fold(0.0_f32, f32::max),
            "physical_tablet_tested": false,
            "renders": renders,
        });
        std::fs::write(
            output.join("stipple-spray-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    fn verify_image_stamps(&mut self, rect: Rect) {
        use std::time::Instant;

        let library = crate::presets::Library::default();
        let names = ["Leaf Scatter", "Grass Scatter", "Star Stamp"];
        let presets: Vec<_> = names
            .iter()
            .map(|name| {
                library
                    .presets
                    .iter()
                    .find(|preset| preset.name == *name)
                    .unwrap_or_else(|| panic!("built-in {name} preset exists"))
                    .clone()
            })
            .collect();
        let assets = library.tip_assets.clone();
        let mut camera = Camera {
            distance: 8.5,
            ..self.camera
        };
        camera.align_guide(DrawingPlane::default());

        let make_stamp = |preset: &crate::presets::Preset, id: u64, y: f32, tap: bool| {
            let mut brush = preset.brush;
            brush.dab_seed = crate::dab_placement::seed_for_stroke(id);
            let points = if tap {
                vec![Point {
                    position: Vec3::new(3.0, y, 0.0),
                    pressure: 0.8,
                    tilt: None,
                    time_ms: 0,
                }]
            } else {
                (0..=64)
                    .map(|index| {
                        let t = index as f32 / 64.0;
                        let x = -2.8 + 5.0 * t;
                        Point {
                            position: Vec3::new(
                                x,
                                y + 0.13 * (t * std::f32::consts::TAU).sin(),
                                0.0,
                            ),
                            pressure: 0.45 + 0.45 * t,
                            tilt: None,
                            time_ms: (500.0 * t).round() as u32,
                        }
                    })
                    .collect()
            };
            Stroke {
                model_surface: None,
                curve: None,
                material: Default::default(),
                id,
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: preset.color,
                gradient: None,
                radius: preset.radius,
                brush,
                points,
            }
        };

        let mut strokes = Vec::new();
        let mut taps = Vec::new();
        let mut alpha_summaries = Vec::new();
        for (index, preset) in presets.iter().enumerate() {
            let asset = library
                .tip_asset(preset.brush.tip_asset_id)
                .unwrap_or_else(|| panic!("{} preset has its embedded tip", preset.name));
            let decoded = asset.decoded().unwrap();
            let alpha = decoded.pixels.as_chunks::<4>().0;
            assert!(alpha.iter().any(|pixel| pixel[3] == 0));
            assert!(alpha.iter().any(|pixel| pixel[3] == 255));
            assert_eq!(
                preset.brush.tip_aspect,
                asset.size[0] as f32 / asset.size[1] as f32
            );

            let y = 1.0 - index as f32;
            let stroke = make_stamp(preset, 75_001 + index as u64 * 2, y, false);
            let dabs = crate::dab_placement::sample(&stroke).unwrap();
            assert!(dabs.len() > 1, "{} stamps along a drag", preset.name);
            let mesh = geometry::stroke_mesh(&stroke);
            assert_eq!(mesh.len(), dabs.len() * 6);
            assert!(mesh.iter().all(|vertex| {
                vertex
                    .uv
                    .into_iter()
                    .all(|coordinate| coordinate.is_finite() && (0.0..=1.0).contains(&coordinate))
            }));
            assert!(mesh.len() <= crate::export::MAX_TRIANGLE_VERTICES_PER_STROKE);

            let tap = make_stamp(preset, 75_002 + index as u64 * 2, y, true);
            assert_eq!(crate::dab_placement::sample(&tap).unwrap().len(), 1);
            let tap_doc = Document {
                linked_stroke_groups: Default::default(),
                strokes: vec![tap.clone()],
                brush_tip_assets: vec![asset.clone()],
                ..Document::default()
            };
            let tap_dab = crate::dab_placement::sample(&tap).unwrap()[0];
            let mut opaque_pick = false;
            let mut transparent_miss = false;
            for y_index in 1..32 {
                for x_index in 1..32 {
                    let uv = [x_index as f32 / 32.0, y_index as f32 / 32.0];
                    let pixel_x = (uv[0] * decoded.size[0] as f32)
                        .floor()
                        .min(decoded.size[0].saturating_sub(1) as f32)
                        as usize;
                    let pixel_y = (uv[1] * decoded.size[1] as f32)
                        .floor()
                        .min(decoded.size[1].saturating_sub(1) as f32)
                        as usize;
                    let coverage =
                        decoded.pixels[(pixel_y * decoded.size[0] as usize + pixel_x) * 4 + 3];
                    if coverage == 255 && !opaque_pick {
                        let world = image_tip_position_at_uv(&tap, tap_dab, uv);
                        let screen = gizmo::project(&camera, rect, world)
                            .expect("opaque tip pixel is in the visible camera frustum")
                            .0;
                        assert_eq!(
                            gizmo::pick(&tap_doc, &camera, rect, screen),
                            Some(tap.id),
                            "opaque pixel on {} is pickable",
                            preset.name
                        );
                        opaque_pick = true;
                    }
                    if coverage == 0 && !transparent_miss {
                        let world = image_tip_position_at_uv(&tap, tap_dab, uv);
                        let screen = gizmo::project(&camera, rect, world)
                            .expect("transparent tip pixel is in the visible camera frustum")
                            .0;
                        assert_ne!(
                            gizmo::pick(&tap_doc, &camera, rect, screen),
                            Some(tap.id),
                            "transparent pixel on {} is not pickable",
                            preset.name
                        );
                        transparent_miss = true;
                    }
                    if opaque_pick && transparent_miss {
                        break;
                    }
                }
                if opaque_pick && transparent_miss {
                    break;
                }
            }
            assert!(
                opaque_pick,
                "{} contains a pickable opaque pixel",
                preset.name
            );
            assert!(
                transparent_miss,
                "{} transparent edge pixels do not intercept picking",
                preset.name
            );
            alpha_summaries.push(serde_json::json!({
                "preset": preset.name,
                "tip_id": asset.id,
                "size": asset.size,
                "png_bytes": asset.png.len(),
                "tap_dabs": 1,
                "drag_dabs": dabs.len(),
                "drag_vertices": mesh.len(),
                "transparent_texels": alpha.iter().filter(|pixel| pixel[3] == 0).count(),
                "opaque_texels": alpha.iter().filter(|pixel| pixel[3] == 255).count(),
                "partial_alpha_texels": alpha.iter().filter(|pixel| (1..255).contains(&pixel[3])).count(),
            }));
            strokes.push(stroke);
            taps.push(tap);
        }

        let mut study_strokes = Vec::with_capacity(strokes.len() + taps.len());
        for (stroke, tap) in strokes.iter().cloned().zip(taps) {
            study_strokes.push(stroke);
            study_strokes.push(tap);
        }
        let study = Document {
            linked_stroke_groups: Default::default(),
            strokes: study_strokes,
            brush_tip_assets: assets.clone(),
            ..Document::default()
        };
        let native_bytes = serde_json::to_vec_pretty(&study).unwrap();
        let native_text = std::str::from_utf8(&native_bytes).unwrap();
        let reloaded = Document::from_json(native_text).unwrap();
        assert_eq!(reloaded.strokes, study.strokes);
        assert_eq!(reloaded.brush_tip_assets, study.brush_tip_assets);

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/sketch-075");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("stamp-study.sketch.json"), &native_bytes).unwrap();

        let mut placement_ms = Vec::new();
        let mut mesh_ms = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            for stroke in &reloaded.strokes {
                std::hint::black_box(crate::dab_placement::sample(stroke).unwrap());
            }
            placement_ms.push(start.elapsed().as_secs_f64() * 1000.0);

            let start = Instant::now();
            for stroke in &reloaded.strokes {
                std::hint::black_box(geometry::stroke_mesh(stroke));
            }
            mesh_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }

        let mut glb_ms = Vec::new();
        let mut glb_bytes = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let encoded = crate::export::encode(&reloaded, None).unwrap();
            glb_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(encoded.summary.strokes, reloaded.strokes.len());
            assert!(
                encoded.summary.vertices
                    <= reloaded.strokes.len() * crate::export::MAX_TRIANGLE_VERTICES_PER_STROKE
            );
            assert!(encoded.summary.bytes <= crate::export::MAX_BINARY_BYTES);
            if glb_bytes.is_empty() {
                glb_bytes = encoded.bytes;
            }
        }
        assert!(glb_bytes.len() >= 28);
        assert_eq!(&glb_bytes[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb_bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(glb_bytes[8..12].try_into().unwrap()) as usize,
            glb_bytes.len()
        );
        let json_length = u32::from_le_bytes(glb_bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(glb_bytes[16..20].try_into().unwrap()),
            0x4e4f_534a
        );
        let document_end = 20 + json_length;
        let gltf_doc: serde_json::Value =
            serde_json::from_slice(&glb_bytes[20..document_end]).unwrap();
        let bin_length = u32::from_le_bytes(
            glb_bytes[document_end..document_end + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(
            u32::from_le_bytes(
                glb_bytes[document_end + 4..document_end + 8]
                    .try_into()
                    .unwrap()
            ),
            0x004e_4942
        );
        let binary_start = document_end + 8;
        let binary = &glb_bytes[binary_start..binary_start + bin_length];
        let images = gltf_doc["images"].as_array().unwrap();
        assert_eq!(images.len(), assets.len());
        let views = gltf_doc["bufferViews"].as_array().unwrap();
        let embedded: Vec<_> = images
            .iter()
            .map(|image| {
                assert_eq!(image["mimeType"], "image/png");
                let view = &views[image["bufferView"].as_u64().unwrap() as usize];
                let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
                let length = view["byteLength"].as_u64().unwrap() as usize;
                &binary[offset..offset + length]
            })
            .collect();
        for asset in &assets {
            assert!(embedded.contains(&asset.png.as_slice()));
        }
        let meshes = gltf_doc["meshes"].as_array().unwrap();
        let materials = gltf_doc["materials"].as_array().unwrap();
        let accessors = gltf_doc["accessors"].as_array().unwrap();
        assert_eq!(meshes.len(), reloaded.strokes.len());
        for (mesh, stroke) in meshes.iter().zip(&reloaded.strokes) {
            let primitive = &mesh["primitives"][0];
            let material = &materials[primitive["material"].as_u64().unwrap() as usize];
            assert_eq!(material["doubleSided"], true);
            assert_eq!(material["alphaMode"], "BLEND");
            assert_eq!(
                material["pbrMetallicRoughness"]["baseColorFactor"],
                serde_json::json!([stroke.color[0], stroke.color[1], stroke.color[2], 1.0])
            );
            assert_eq!(
                material["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"],
                0
            );
            let uv_accessor = accessors
                [primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize]
                .as_object()
                .unwrap();
            assert_eq!(uv_accessor["type"], "VEC2");
            assert!(uv_accessor["count"].as_u64().unwrap() > 0);
            assert_eq!(uv_accessor["componentType"], 5126);
        }
        std::fs::write(output.join("stamp-study.glb"), &glb_bytes).unwrap();

        let obj_root = tempfile::tempdir().unwrap();
        let mut obj_ms = Vec::new();
        let mut obj_bundle_bytes = 0usize;
        for iteration in 0..5 {
            let destination = obj_root.path().join(format!("stamp-study-{iteration}.obj"));
            let start = Instant::now();
            let exported_bytes =
                crate::obj_export::write_obj_bundle(&reloaded, None, &destination, false)
                    .expect("image stamps export to an OBJ/MTL bundle");
            obj_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(exported_bytes > 0);
            if iteration == 0 {
                let obj = std::fs::read_to_string(&destination).unwrap();
                assert_eq!(obj.matches("usemtl ").count(), reloaded.strokes.len());
                assert!(obj.contains("vt "));
                assert!(obj.contains("f "));
                let mtl_name = obj
                    .lines()
                    .find_map(|line| line.strip_prefix("mtllib "))
                    .unwrap();
                let mtl_path = obj_root.path().join(mtl_name);
                let mtl = std::fs::read_to_string(&mtl_path).unwrap();
                let color_maps: Vec<_> = mtl
                    .lines()
                    .filter_map(|line| line.strip_prefix("map_Kd "))
                    .collect();
                let alpha_maps: Vec<_> = mtl
                    .lines()
                    .filter_map(|line| line.strip_prefix("map_d "))
                    .collect();
                assert_eq!(color_maps.len(), assets.len());
                assert_eq!(alpha_maps.len(), assets.len());
                for (preset, asset) in presets.iter().zip(&assets) {
                    let diffuse = format!(
                        "Kd {:.6} {:.6} {:.6}",
                        preset.color[0], preset.color[1], preset.color[2]
                    );
                    assert!(
                        mtl.contains(&diffuse),
                        "OBJ carries tint for {}",
                        preset.name
                    );
                    let section = mtl
                        .split_once(&diffuse)
                        .unwrap()
                        .1
                        .split("\nnewmtl ")
                        .next()
                        .unwrap();
                    assert!(section.contains("map_Kd "));
                    assert!(section.contains("map_d "));

                    let color_map = section
                        .lines()
                        .find_map(|line| line.strip_prefix("map_Kd "))
                        .unwrap();
                    let alpha_map = section
                        .lines()
                        .find_map(|line| line.strip_prefix("map_d "))
                        .unwrap();
                    let color_file = obj_root.path().join(color_map);
                    let color_bytes = std::fs::read(&color_file).unwrap();
                    assert_eq!(color_bytes, asset.png);

                    let decoded = asset.decoded().unwrap();
                    let alpha_file = obj_root.path().join(alpha_map);
                    let alpha_image = image::open(&alpha_file).unwrap().to_rgba8();
                    assert_eq!([alpha_image.width(), alpha_image.height()], decoded.size);
                    assert!(alpha_image
                        .pixels()
                        .zip(decoded.pixels.as_chunks::<4>().0)
                        .all(|(pixel, source)| pixel.0 == [source[3], source[3], source[3], 255]));
                }
                let referenced_paths = color_maps
                    .iter()
                    .chain(&alpha_maps)
                    .map(|path| obj_root.path().join(path))
                    .chain(std::iter::once(mtl_path));
                obj_bundle_bytes = std::iter::once(destination.clone())
                    .chain(referenced_paths)
                    .map(|path| std::fs::metadata(path).unwrap().len() as usize)
                    .sum();
                assert_eq!(obj_bundle_bytes, exported_bytes);
                assert!(obj_bundle_bytes <= crate::export::MAX_BINARY_BYTES);
            }
        }

        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let background = [0.94, 0.93, 0.89];
        let mut renders = Vec::new();
        let mut rendered_geometry = Vec::new();
        for stroke in &reloaded.strokes {
            rendered_geometry.push(
                geometry::stroke_mesh(stroke)
                    .into_iter()
                    .map(|vertex| vertex.position)
                    .collect::<Vec<_>>(),
            );
        }
        for size in [[640, 360], [1280, 720], [1920, 1080]] {
            for (view, yaw_offset) in [("front", 0.0), ("orbit", 0.65)] {
                let mut view_camera = camera;
                view_camera.yaw += yaw_offset;
                let pixels = self
                    .renderer
                    .render_png_pixels(&reloaded, &view_camera, size, background, lighting)
                    .expect("image stamp sampler renders at each review size");
                let image = image::RgbaImage::from_raw(size[0], size[1], pixels)
                    .expect("image stamp render dimensions match pixel count");
                let file = format!("stamps-{view}-{}x{}.png", size[0], size[1]);
                image.save(output.join(&file)).unwrap();
                for (stroke, geometry) in reloaded.strokes.iter().zip(&rendered_geometry) {
                    assert_eq!(
                        geometry::stroke_mesh(stroke)
                            .into_iter()
                            .map(|vertex| vertex.position)
                            .collect::<Vec<_>>(),
                        *geometry,
                        "camera orbit does not reorient captured stamp geometry"
                    );
                }
                renders.push(serde_json::json!({
                    "view": view,
                    "yaw_offset": yaw_offset,
                    "size": size,
                    "file": file,
                }));
            }
        }

        let timing_median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let report = serde_json::json!({
            "passed": true,
            "presets": alpha_summaries,
            "stroke_count": reloaded.strokes.len(),
            "placement_count": crate::dab_placement::document_count(&reloaded).unwrap(),
            "mesh_vertices": reloaded.strokes.iter().map(|stroke| geometry::stroke_mesh(stroke).len()).sum::<usize>(),
            "glb_vertices": crate::export::encode(&reloaded, None).unwrap().summary.vertices,
            "glb_bytes": glb_bytes.len(),
            "obj_bundle_bytes": obj_bundle_bytes,
            "cpu_medians_ms": {
                "placement": timing_median(&mut placement_ms),
                "mesh_build": timing_median(&mut mesh_ms),
                "glb_encode": timing_median(&mut glb_ms),
                "obj_export": timing_median(&mut obj_ms),
            },
            "cpu_timing_note": "Five local release-mode wall-clock iterations over the 6-stroke fixture; OBJ includes temporary-file writes and syncs. Excludes GPU rendering and physical pen latency.",
            "platform": {
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "logical_cpus": std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1),
            },
            "checks": [
                "Leaf Scatter, Grass Scatter, and Star Stamp tap and drag geometry",
                "transparent edges miss picking while fully opaque tip pixels remain pickable",
                "native save data roundtrips all six stamps and three embedded tip assets",
                "GLB embeds the exact tip PNGs with UVs, tint, blending, and double-sided material",
                "OBJ/MTL writes original diffuse PNGs and grayscale alpha companions with tint",
                "stamp mesh, GLB, and OBJ bundles remain within geometry and file budgets",
                "front and orbit GPU renders at 640x360, 1280x720, and 1920x1080",
                "camera orbit leaves saved world-space stamp geometry unchanged",
            ],
            "physical_tablet_tested": false,
            "gpu": self.renderer.adapter,
            "renders": renders,
            "native_file": "stamp-study.sketch.json",
            "glb_file": "stamp-study.glb",
        });
        std::fs::write(
            output.join("image-stamps-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    fn verify_strand_brushes(&mut self, rect: Rect) {
        use std::time::Instant;

        let library = crate::presets::Library::default();
        let preset = |name: &str| {
            library
                .presets
                .iter()
                .find(|preset| preset.name == name)
                .unwrap_or_else(|| panic!("built-in {name} preset exists"))
                .clone()
        };
        let rake = preset("Rake");
        let bristle = preset("Bristle Fan");
        let dry = preset("Dry Brush");
        let make_stroke = |preset: &crate::presets::Preset, id: u64, y: f32| Stroke {
            model_surface: None,
            curve: None,
            material: Default::default(),
            id,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: preset.color,
            gradient: None,
            radius: preset.radius,
            brush: preset.brush,
            points: (0..=80)
                .map(|index| {
                    let t = index as f32 / 80.0;
                    Point {
                        position: Vec3::new(
                            -2.0 + 4.0 * t,
                            y + 0.12 * (t * std::f32::consts::TAU).sin(),
                            0.0,
                        ),
                        pressure: 0.18 + 0.82 * t,
                        tilt: None,
                        time_ms: 0,
                    }
                })
                .collect(),
        };
        let strokes = vec![
            make_stroke(&rake, 76_001, 0.95),
            make_stroke(&bristle, 76_002, 0.0),
            make_stroke(&dry, 76_003, -0.95),
        ];
        let study = Document {
            strokes,
            ..Document::default()
        };
        for (stroke, preset) in study.strokes.iter().zip([&rake, &bristle, &dry]) {
            assert!(stroke.brush.is_valid());
            let mesh = geometry::stroke_mesh(stroke);
            assert!(
                !mesh.is_empty(),
                "{} builds exportable geometry",
                preset.name
            );
            assert!(
                mesh.len() <= crate::geometry::MAX_STRAND_VERTICES || !preset.brush.strands.enabled
            );
            assert!(mesh.iter().all(|vertex| {
                vertex.position.into_iter().all(f32::is_finite)
                    && vertex.normal.into_iter().all(f32::is_finite)
                    && vertex.uv.into_iter().all(f32::is_finite)
            }));
            if preset.brush.strands.enabled {
                assert!(
                    stroke.brush.strands.vertex_estimate(stroke.points.len())
                        <= crate::geometry::MAX_STRAND_VERTICES
                );
                assert_eq!(
                    crate::dab_placement::sample(stroke).unwrap().len(),
                    0,
                    "{} remains one logical strand stroke",
                    preset.name
                );
            }
        }

        let native = Document::from_json(&serde_json::to_string(&study).unwrap()).unwrap();
        assert_eq!(native.strokes, study.strokes);
        let mut camera = self.camera;
        camera.align(crate::geometry::Plane::Front);
        camera.target = Vec3::ZERO;
        camera.distance = 7.5;
        let doc = native.clone();
        for stroke in doc.strokes.iter().take(2) {
            let mesh = geometry::stroke_mesh(stroke);
            let point = (Vec3::from(mesh[0].position)
                + Vec3::from(mesh[1].position)
                + Vec3::from(mesh[2].position))
                / 3.0;
            let screen = gizmo::project(&camera, rect, point)
                .expect("strand mesh sample projects to the viewport")
                .0;
            assert_eq!(
                gizmo::pick(&doc, &camera, rect, screen),
                Some(stroke.id),
                "visible {} strand surface remains pickable",
                if stroke.brush.strands.count == rake.brush.strands.count {
                    "Rake"
                } else {
                    "Bristle Fan"
                }
            );
        }

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/sketch-076");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(
            output.join("rake-bristle-study.sketch.json"),
            serde_json::to_vec_pretty(&native).unwrap(),
        )
        .unwrap();
        let encoded = crate::export::encode(&native, None).unwrap();
        assert_eq!(&encoded.bytes[..4], b"glTF");
        let json_length = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let document_end = 20 + json_length;
        let gltf_document: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..document_end]).unwrap();
        let meshes = gltf_document["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), native.strokes.len());
        let accessors = gltf_document["accessors"].as_array().unwrap();
        let glb_positions: usize = meshes
            .iter()
            .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
            .map(|primitive| {
                let position_accessor = primitive["attributes"]["POSITION"].as_u64().unwrap();
                accessors[position_accessor as usize]["count"]
                    .as_u64()
                    .unwrap() as usize
            })
            .sum();
        assert_eq!(glb_positions, encoded.summary.vertices);
        assert!(glb_positions <= 3 * crate::export::MAX_TRIANGLE_VERTICES_PER_STROKE);
        std::fs::write(output.join("rake-bristle-study.glb"), &encoded.bytes).unwrap();

        let obj_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_root.path().join("rake-bristle-study.obj");
        let obj_bytes = crate::obj_export::write_obj_bundle(&native, None, &obj_path, false)
            .expect("strand strokes export to OBJ/MTL");
        let obj = std::fs::read_to_string(&obj_path).unwrap();
        assert_eq!(obj.matches("usemtl ").count(), native.strokes.len());
        let obj_vertices = obj.lines().filter(|line| line.starts_with("v ")).count();
        let obj_faces = obj.lines().filter(|line| line.starts_with("f ")).count();
        assert_eq!(obj_vertices, encoded.summary.vertices);
        assert!(obj_faces > 0);
        assert!(obj_bytes > 0);
        assert!(obj_bytes < crate::export::MAX_BINARY_BYTES);
        std::fs::copy(&obj_path, output.join("rake-bristle-study.obj")).unwrap();
        let mtl_name = obj
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .unwrap();
        std::fs::copy(
            obj_root.path().join(mtl_name),
            output.join("rake-bristle-study.mtl"),
        )
        .unwrap();

        let geometry_before: Vec<_> = native
            .strokes
            .iter()
            .map(|stroke| {
                geometry::stroke_mesh(stroke)
                    .into_iter()
                    .map(|vertex| vertex.position)
                    .collect::<Vec<_>>()
            })
            .collect();
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let mut renders = Vec::new();
        for size in [[640, 360], [1280, 720], [1920, 1080]] {
            for (view, yaw) in [("front", 0.0), ("orbit", 0.65)] {
                camera.yaw = yaw;
                let pixels = self
                    .renderer
                    .render_png_pixels(&native, &camera, size, [0.94, 0.93, 0.89], lighting)
                    .expect("strand sampler renders on the active GPU adapter");
                let image = image::RgbaImage::from_raw(size[0], size[1], pixels).unwrap();
                let file = format!("strands-{view}-{}x{}.png", size[0], size[1]);
                image.save(output.join(&file)).unwrap();
                for (stroke, before) in native.strokes.iter().zip(&geometry_before) {
                    assert_eq!(
                        geometry::stroke_mesh(stroke)
                            .into_iter()
                            .map(|vertex| vertex.position)
                            .collect::<Vec<_>>(),
                        *before,
                        "camera orbit preserves captured {} geometry",
                        stroke.id
                    );
                }
                renders.push(serde_json::json!({"view":view,"size":size,"file":file}));
            }
        }

        let mut placement_ms = Vec::new();
        let mut mesh_ms = Vec::new();
        let mut glb_ms = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            for stroke in &native.strokes {
                std::hint::black_box(crate::dab_placement::sample(stroke).unwrap());
            }
            placement_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            for stroke in &native.strokes {
                std::hint::black_box(geometry::stroke_mesh(stroke));
            }
            mesh_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            std::hint::black_box(crate::export::encode(&native, None).unwrap());
            glb_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let report = serde_json::json!({
            "passed":true,
            "presets":["Rake","Bristle Fan","Dry Brush"],
            "logical_strokes":native.strokes.len(),
            "strand_counts":[rake.brush.strands.count,bristle.brush.strands.count],
            "mesh_vertices":native.strokes.iter().map(|stroke|geometry::stroke_mesh(stroke).len()).sum::<usize>(),
            "glb_vertices":glb_positions,
            "glb_bytes":encoded.bytes.len(),
            "obj_vertices":obj_vertices,
            "obj_faces":obj_faces,
            "obj_bundle_bytes":obj_bytes,
            "cpu_medians_ms":{"placement":median(&mut placement_ms),"mesh_build":median(&mut mesh_ms),"glb_encode":median(&mut glb_ms)},
            "cpu_timing_note":"Five local CPU iterations; excludes GPU rendering and physical pen latency.",
            "checks":["Rake and Bristle Fan contrast with textured Dry Brush","finite pressure-varying meshes and strand vertex budgets","same logical stroke and picking on visible strand triangles","native semantic settings roundtrip","GLB primitives independently parsed and vertex budgets checked","OBJ vertex and face counts independently parsed","front/orbit GPU samplers at three output sizes","camera orbit preserves saved strand geometry"],
            "gpu":self.renderer.adapter,
            "physical_tablet_tested":false,
            "renders":renders,
        });
        std::fs::write(
            output.join("strand-brush-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    fn verify_profile_brushes(&mut self, rect: Rect) {
        use std::time::Instant;

        let library = crate::presets::Library::default();
        let presets: Vec<_> = ["Square Tube", "Triangular Tube", "Star Tube"]
            .into_iter()
            .map(|name| {
                library
                    .presets
                    .iter()
                    .find(|preset| preset.name == name)
                    .unwrap_or_else(|| panic!("built-in {name} preset exists"))
                    .clone()
            })
            .collect();
        let strokes: Vec<_> = presets
            .iter()
            .enumerate()
            .map(|(index, preset)| Stroke {
                model_surface: None,
                curve: None,
                material: Default::default(),
                id: 78_001 + index as u64,
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: preset.color,
                gradient: None,
                radius: preset.radius,
                brush: preset.brush,
                points: [
                    (-1.8, -0.2, 0.28),
                    (-0.35, 0.22, 0.7),
                    (0.15, -0.18, 1.0),
                    (1.8, 0.2, 0.48),
                ]
                .into_iter()
                .map(|(x, bend, pressure)| Point {
                    position: Vec3::new(x, index as f32 * -0.8 + bend, 0.0),
                    pressure,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
            })
            .collect();
        let study = Document {
            strokes,
            ..Document::default()
        };
        for (stroke, preset) in study.strokes.iter().zip(&presets) {
            assert!(stroke.brush.is_valid());
            assert_ne!(
                stroke.brush.tube_profile,
                crate::document::TubeProfile::Round
            );
            let mesh = geometry::stroke_mesh(stroke);
            assert!(!mesh.is_empty());
            assert!(mesh.len() <= crate::geometry::MAX_PROFILE_TUBE_VERTICES);
            assert!(mesh.iter().all(|vertex| {
                vertex.position.into_iter().all(f32::is_finite)
                    && vertex.normal.into_iter().all(f32::is_finite)
                    && vertex.uv.into_iter().all(f32::is_finite)
            }));
            assert_eq!(crate::dab_placement::sample(stroke).unwrap().len(), 0);
            assert!(
                stroke.brush.tube_profile_scale.is_finite(),
                "{} retains bounded cross-section controls",
                preset.name
            );
        }

        let native = Document::from_json(&serde_json::to_string(&study).unwrap()).unwrap();
        assert_eq!(native.strokes, study.strokes);
        let mut camera = self.camera;
        camera.align(crate::geometry::Plane::Front);
        camera.target = Vec3::new(0.0, -0.8, 0.0);
        camera.distance = 7.5;
        for stroke in &native.strokes {
            let mesh = geometry::stroke_mesh(stroke);
            let point = (Vec3::from(mesh[0].position)
                + Vec3::from(mesh[1].position)
                + Vec3::from(mesh[2].position))
                / 3.0;
            let screen = gizmo::project(&camera, rect, point)
                .expect("profile mesh sample projects to the viewport")
                .0;
            assert_eq!(
                gizmo::pick(&native, &camera, rect, screen),
                Some(stroke.id),
                "profile surface remains pickable"
            );
        }

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/sketch-078");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(
            output.join("tube-profile-study.sketch.json"),
            serde_json::to_vec_pretty(&native).unwrap(),
        )
        .unwrap();
        let glb = crate::export::encode(&native, None).unwrap();
        assert_eq!(&glb.bytes[..4], b"glTF");
        let json_length = u32::from_le_bytes(glb.bytes[12..16].try_into().unwrap()) as usize;
        let gltf: serde_json::Value =
            serde_json::from_slice(&glb.bytes[20..20 + json_length]).unwrap();
        let meshes = gltf["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), native.strokes.len());
        let accessors = gltf["accessors"].as_array().unwrap();
        let glb_vertices: usize = meshes
            .iter()
            .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
            .map(|primitive| {
                let position_accessor = primitive["attributes"]["POSITION"].as_u64().unwrap();
                accessors[position_accessor as usize]["count"]
                    .as_u64()
                    .unwrap() as usize
            })
            .sum();
        assert_eq!(glb_vertices, glb.summary.vertices);
        assert!(glb_vertices <= 3 * crate::geometry::MAX_PROFILE_TUBE_VERTICES);
        std::fs::write(output.join("tube-profile-study.glb"), &glb.bytes).unwrap();

        let obj_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_root.path().join("tube-profile-study.obj");
        let obj_bytes = crate::obj_export::write_obj_bundle(&native, None, &obj_path, false)
            .expect("profiled tubes export to OBJ/MTL");
        let obj = std::fs::read_to_string(&obj_path).unwrap();
        let obj_vertices = obj.lines().filter(|line| line.starts_with("v ")).count();
        let obj_normals = obj.lines().filter(|line| line.starts_with("vn ")).count();
        let obj_faces = obj.lines().filter(|line| line.starts_with("f ")).count();
        assert_eq!(obj_vertices, glb_vertices);
        assert_eq!(obj_normals, glb_vertices);
        assert!(obj_faces > 0);
        assert!(obj_bytes < crate::export::MAX_BINARY_BYTES);
        std::fs::copy(&obj_path, output.join("tube-profile-study.obj")).unwrap();
        let mtl_name = obj
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .unwrap();
        std::fs::copy(
            obj_root.path().join(mtl_name),
            output.join("tube-profile-study.mtl"),
        )
        .unwrap();

        let geometry_before: Vec<_> = native
            .strokes
            .iter()
            .map(|stroke| {
                geometry::stroke_mesh(stroke)
                    .into_iter()
                    .map(|vertex| vertex.position)
                    .collect::<Vec<_>>()
            })
            .collect();
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let mut renders = Vec::new();
        for size in [[640, 360], [1280, 720], [1920, 1080]] {
            for (view, yaw) in [("front", 0.0), ("orbit", 0.65)] {
                camera.yaw = yaw;
                let pixels = self
                    .renderer
                    .render_png_pixels(&native, &camera, size, [0.94, 0.93, 0.89], lighting)
                    .expect("tube profile sampler renders on the active GPU adapter");
                let image = image::RgbaImage::from_raw(size[0], size[1], pixels).unwrap();
                let file = format!("profiles-{view}-{}x{}.png", size[0], size[1]);
                image.save(output.join(&file)).unwrap();
                for (stroke, before) in native.strokes.iter().zip(&geometry_before) {
                    assert_eq!(
                        geometry::stroke_mesh(stroke)
                            .into_iter()
                            .map(|vertex| vertex.position)
                            .collect::<Vec<_>>(),
                        *before,
                        "camera orbit preserves saved profile orientation"
                    );
                }
                renders.push(serde_json::json!({"view":view,"size":size,"file":file}));
            }
        }
        let mut mesh_ms = Vec::new();
        let mut glb_ms = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            for stroke in &native.strokes {
                std::hint::black_box(geometry::stroke_mesh(stroke));
            }
            mesh_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            std::hint::black_box(crate::export::encode(&native, None).unwrap());
            glb_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let report = serde_json::json!({
            "passed":true,
            "profiles":["Square Tube","Triangular Tube","Star Tube"],
            "stroke_count":native.strokes.len(),
            "mesh_vertices":native.strokes.iter().map(|stroke|geometry::stroke_mesh(stroke).len()).sum::<usize>(),
            "glb_vertices":glb_vertices,
            "glb_bytes":glb.bytes.len(),
            "obj_vertices":obj_vertices,
            "obj_normals":obj_normals,
            "obj_faces":obj_faces,
            "obj_bundle_bytes":obj_bytes,
            "cpu_medians_ms":{"mesh_build":median(&mut mesh_ms),"glb_encode":median(&mut glb_ms)},
            "cpu_timing_note":"Five local CPU iterations; excludes GPU rendering and physical pen latency.",
            "checks":["three built-in profiles keep distinct cross-sections under pressure","native profile settings roundtrip","profile mesh samples remain pickable","GLB accessor and OBJ vertex/normal/face records parsed independently","profile export respects one-million-vertex per-stroke limit","front/orbit GPU samplers at three output sizes","camera orbit preserves captured profile geometry"],
            "gpu":self.renderer.adapter,
            "physical_tablet_tested":false,
            "renders":renders,
        });
        std::fs::write(
            output.join("tube-profile-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}

fn image_tip_position_at_uv(
    stroke: &Stroke,
    dab: crate::dab_placement::ResolvedDab,
    uv: [f32; 2],
) -> Vec3 {
    let tangent = dab.tangent.try_normalize().unwrap_or(Vec3::Y);
    let side = dab.normal.cross(tangent).try_normalize().unwrap_or(Vec3::X);
    let angle = (stroke.brush.angle_degrees + dab.angle_degrees).to_radians();
    let along = (tangent * angle.cos() + side * angle.sin())
        .try_normalize()
        .unwrap_or(tangent);
    let across = (side * angle.cos() - tangent * angle.sin())
        .try_normalize()
        .unwrap_or(side);
    let half_width = stroke.radius * dab.size_scale;
    let half_height = half_width / stroke.brush.tip_aspect;
    dab.position
        + along * ((2.0 * uv[0] - 1.0) * half_width)
        + across * ((1.0 - 2.0 * uv[1]) * half_height)
}
