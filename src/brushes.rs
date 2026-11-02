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
                assert_eq!(loaded.version, 24);
                assert_eq!(loaded.strokes, original);
                self.verify_stipple_spray(rect);
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
                curve: None,
                material: Default::default(),
                id,
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
}
