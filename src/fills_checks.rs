use super::*;
impl SketchApp {
    pub(super) fn verify_fills(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.fills.verification_frame else {
            return;
        };
        if frame == 1 {
            self.request(Pending::New);
            self.fills.enabled = true;
            for delta in [
                (-130.0, 80.0),
                (80.0, 80.0),
                (80.0, -20.0),
                (0.0, -20.0),
                (0.0, -110.0),
                (-130.0, -110.0),
            ] {
                self.pointer_action(
                    rect.center() + egui::vec2(delta.0, delta.1),
                    1.0,
                    rect,
                    true,
                    false,
                );
            }
            assert!(self.doc.strokes.is_empty());
            assert_eq!(self.fills.draft.as_ref().unwrap().mark.points.len(), 6);
            self.commit_fill();
            assert_eq!(self.doc.strokes.len(), 1);
            assert!(self.doc.strokes[0].fill.is_some());
            assert_eq!(
                crate::gizmo::pick(
                    &self.doc,
                    &self.camera,
                    rect,
                    rect.center() + egui::vec2(-50.0, 30.0)
                ),
                Some(1)
            );
            assert!(crate::gizmo::pick(
                &self.doc,
                &self.camera,
                rect,
                rect.center() + egui::vec2(40.0, -70.0)
            )
            .is_none());
            self.undo();
            assert!(self.doc.strokes.is_empty());
            self.redo();
            self.selected = HashSet::from([1]);
            self.duplicate_selected();
            assert_eq!(self.doc.strokes.len(), 2);
            self.selected = self.doc.strokes.iter().map(|s| s.id).collect();
            let group = crate::editing::group_selected(&self.doc, &self.selected).unwrap();
            group.apply(&mut self.doc, true);
            self.undo.push(group);
            self.redo.clear();
            self.transform_mode = Mode::Move;
            self.transform_values = [0.2, 0.3, 0.1];
            self.numeric_transform();
            let moved = self.doc.strokes.clone();
            self.undo();
            self.redo();
            assert_eq!(self.doc.strokes, moved);
            let path = std::env::current_dir()
                .unwrap()
                .join("verification/fills.sketch.json");
            self.file = Some(path.clone());
            assert!(self.save(false));
            let loaded = Document::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(loaded.version, 30);
            assert_eq!(loaded.strokes, self.doc.strokes);
            let mesh = crate::geometry::stroke_mesh(&self.doc.strokes[0]);
            let center = mesh
                .iter()
                .take(3)
                .map(|v| Vec3::from(v.position))
                .sum::<Vec3>()
                / 3.0;
            let (hit, _) = crate::gizmo::project(&self.camera, rect, center).unwrap();
            self.apply_layer_action(crate::editing::LayerAction::Lock(1, true));
            self.erase(hit, rect);
            assert_eq!(self.doc.strokes.len(), 2);
            self.undo();
            self.erase(hit, rect);
            assert_eq!(self.doc.strokes.len(), 1);
            self.undo();
            self.fills.enabled = true;
            for delta in [(-90.0, -90.0), (90.0, 90.0), (-90.0, 90.0), (90.0, -90.0)] {
                self.fill_corner(rect.center() + egui::vec2(delta.0, delta.1), rect);
            }
            let before = self.doc.strokes.clone();
            self.commit_fill();
            assert_eq!(self.doc.strokes, before);
            assert!(self.fills.draft.is_some());
            assert!(!self.save(false));
            self.request(Pending::New);
            assert_eq!(self.doc.strokes, before);
            self.remove_fill_corner();
            assert_eq!(self.fills.draft.as_ref().unwrap().mark.points.len(), 3);
            self.fills.draft = None;
            self.dirty = false;
            self.request(Pending::FilledStudy);
            self.check_fill_pixels();
            self.verify_fill_refresh_transaction();
            self.status="Filled shapes passed: concavity, draft safety, picking, groups, transforms, history, native save, and depth-tested outlines.".into();
        }
        if frame == 3 {
            if std::env::args().any(|argument| argument == "--verify-fills-no-capture") {
                let path = std::env::current_dir()
                    .unwrap()
                    .join("verification/fills-no-capture-report.json");
                let report = serde_json::json!({
                    "passed": true,
                    "checks": [
                        "planar fill refresh preview leaves source artwork and history unchanged",
                        "Cancel restores the exact drawing without an undo entry",
                        "Apply commits the moved-boundary contour as one undoable edit",
                        "one undo and redo restore the source and refreshed fill",
                        "the fill renderer pixel checks passed at three camera angles",
                        "named layer masks retain inside coverage and remove outside coverage in GPU pixels"
                    ],
                    "screen_capture": false,
                    "gpu": self.renderer.adapter
                });
                std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                self.fills.verification_frame = None;
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 100, "Fill screenshot timed out");
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                path.join("fills-preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["concave click-to-close polygons","invalid draft and document boundary guards","interior and notch picking","mixed groups duplication transforms history","native v7 persistence","layer locking and whole-shape erasing","GPU outlines above paint with ordinary occlusion","editable painted-house study"],"gpu":self.renderer.adapter,"benchmarks_deferred":true});
            std::fs::write(
                path.join("fills-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.fills.verification_frame = Some(frame + 1);
    }
    fn check_fill_pixels(&mut self) {
        use crate::document::PlanarFill;
        let mut doc = Document::default();
        for (id, min, max, z, color) in [
            (1, [-1.0, -0.7], [1.0, 0.7], 0.0, [0.8, 0.65, 0.35]),
            (3, [0.2, -0.4], [0.8, 0.4], 0.3, [0.1, 0.8, 0.2]),
        ] {
            doc.strokes.push(Stroke {
                guide_attachment: None,
                model_surface: None,
                curve: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: Some(PlanarFill {
                    offset: 0.003,
                    gradient: None,
                    texture: None,
                    holes: Vec::new(),
                    cylinder: None,
                    region_source: None,
                }),
                group_id: None,
                layer_id: 1,
                id,
                color,
                gradient: None,
                radius: 0.025,
                brush: Brush::default(),
                points: [
                    [min[0], min[1]],
                    [max[0], min[1]],
                    [max[0], max[1]],
                    [min[0], max[1]],
                ]
                .into_iter()
                .map(|p| Point {
                    position: Vec3::new(p[0], p[1], z),
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
            });
        }
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 2,
            color: [0.01; 3],
            gradient: None,
            radius: 0.012,
            brush: Brush::default(),
            points: [Vec3::new(-0.9, 0.0, 0.0), Vec3::new(0.9, 0.0, 0.0)]
                .into_iter()
                .map(|position| Point {
                    position,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
        });
        let settings = self.renderer.lighting;
        self.renderer.lighting = Default::default();
        for (yaw, pitch) in [(0.0, 0.0), (0.3, 0.2), (-0.3, -0.2)] {
            let camera = Camera {
                yaw,
                pitch,
                distance: 5.0,
                ..Default::default()
            };
            self.renderer.invalidate();
            self.renderer
                .render(&doc, &camera, GuidePreview::Hidden, [600, 450]);
            let pixels = self.renderer.verification_pixels();
            let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 450.0));
            let sample = |world| {
                let (p, _) = crate::gizmo::project(&camera, rect, world).unwrap();
                let i = (p.y.round() as usize * 600 + p.x.round() as usize) * 4;
                &pixels[i..i + 3]
            };
            assert!(
                sample(Vec3::new(-0.5, 0.0, 0.0))[0] < 70,
                "Outline disappeared into its fill"
            );
            assert!(sample(Vec3::new(-0.5, 0.3, 0.0))[0] > 120);
            let foreground = sample(Vec3::new(0.5, 0.0, 0.3));
            assert!(
                foreground[1] > foreground[0] * 2,
                "Foreground failed to occlude the outline"
            );
        }
        self.renderer.lighting = settings;
        self.renderer.invalidate();
        self.renderer
            .render(&self.doc, &self.camera, GuidePreview::Hidden, [900, 700]);
        let pixels = self.renderer.verification_pixels();
        image::save_buffer(
            std::env::current_dir()
                .unwrap()
                .join("verification/filled-house.png"),
            &pixels,
            900,
            700,
            image::ColorType::Rgba8,
        )
        .unwrap();
        self.check_named_layer_mask_pixels();
    }

    fn check_named_layer_mask_pixels(&mut self) {
        use crate::document::{Layer, LayerBlendMode, PlanarFill};

        let mut doc = Document::default();
        doc.layers[0].name = "Named mask source".into();
        doc.layers.push(Layer {
            id: 2,
            name: "Masked artwork".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            clip_to_below: false,
            mask_source: Some(1),
            mask_inverted: false,
            hide_onion_skin: false,
        });

        let fill = |min: [f32; 2], max: [f32; 2], z: f32, id, layer_id, color| Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: Some(PlanarFill {
                offset: 0.003,
                gradient: None,
                texture: None,
                holes: Vec::new(),
                cylinder: None,
                region_source: None,
            }),
            group_id: None,
            layer_id,
            id,
            color,
            gradient: None,
            radius: 0.025,
            brush: Brush::default(),
            points: [
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ]
            .into_iter()
            .map(|point| Point {
                position: Vec3::new(point[0], point[1], z),
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect(),
        };
        doc.strokes
            .push(fill([-0.3, -0.5], [0.3, 0.5], 0.0, 1, 1, [0.08, 0.2, 0.85]));
        doc.strokes
            .push(fill([-0.8, -0.7], [0.8, 0.7], 0.1, 2, 2, [0.9, 0.1, 0.06]));

        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 5.0,
            ..Default::default()
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 450.0));
        let sample_red = |pixels: &[u8], world: Vec3| {
            let (position, _) = crate::gizmo::project(&camera, rect, world).unwrap();
            let index = (position.y.round() as usize * 600 + position.x.round() as usize) * 4;
            pixels[index]
        };

        let background = [0.12, 0.14, 0.18];
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let masked = self
            .renderer
            .render_png_pixels(&doc, &camera, [600, 450], background, lighting)
            .expect("named layer mask renders into an isolated GPU target");
        image::save_buffer(
            "verification/named-layer-mask-600x450.png",
            &masked,
            600,
            450,
            image::ColorType::Rgba8,
        )
        .expect("named layer mask GPU render is saved");
        let inside = sample_red(&masked, Vec3::new(0.0, 0.0, 0.1));
        let outside = sample_red(&masked, Vec3::new(0.55, 0.0, 0.1));

        doc.layers[1].mask_source = None;
        let unmasked = self
            .renderer
            .render_png_pixels(&doc, &camera, [600, 450], background, lighting)
            .expect("unmasked comparison renders into an isolated GPU target");
        let outside_without_mask = sample_red(&unmasked, Vec3::new(0.55, 0.0, 0.1));

        assert!(
            inside > 150,
            "named mask did not preserve inside coverage: {inside}"
        );
        assert!(
            outside_without_mask > outside.saturating_add(60),
            "named mask did not remove outside coverage: masked={outside}, unmasked={outside_without_mask}"
        );
    }

    fn verify_fill_refresh_transaction(&mut self) {
        let saved_doc = self.doc.clone();
        let saved_selected = self.selected.clone();
        let saved_undo = self.undo.clone();
        let saved_redo = self.redo.clone();
        let saved_fills = std::mem::take(&mut self.fills);
        let saved_dirty = self.dirty;
        let saved_select_mode = self.select_mode;

        let boundary = |right: f32| {
            let corners = [
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(right, -1.0, 0.0),
                Vec3::new(right, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
            ];
            (0..corners.len())
                .map(|index| Stroke {
                    guide_attachment: None,
                    model_surface: None,
                    curve: None,
                    material: Default::default(),
                    point_thickness: None,
                    point_colors: None,
                    modifiers: None,
                    fill: None,
                    group_id: None,
                    layer_id: 1,
                    id: index as u64 + 1,
                    color: [0.12, 0.17, 0.20],
                    gradient: None,
                    radius: 0.012,
                    brush: Default::default(),
                    points: [corners[index], corners[(index + 1) % corners.len()]]
                        .into_iter()
                        .map(|position| Point {
                            position,
                            pressure: 0.7,
                            tilt: None,
                            time_ms: 0,
                        })
                        .collect(),
                })
                .collect::<Vec<_>>()
        };

        let mut source = crate::polygon::study().strokes.remove(0);
        source.id = 50;
        source.layer_id = 1;
        source.color = [0.31, 0.52, 0.77];
        source.brush.opacity = 0.63;
        source.fill.as_mut().unwrap().region_source = Some(
            crate::document::FillRegionSource::Planar(crate::document::PlanarFillRegionSource {
                seed: Vec3::ZERO,
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                tangent: Vec3::X,
                half_extent: Some(glam::Vec2::splat(2.0)),
                boundary_layer: Some(1),
                gap_tolerance: 0.0,
            }),
        );
        self.doc = Document::default();
        self.doc.strokes = boundary(1.0);
        let original = crate::app::fills_ui::refreshed_fill(&self.doc, &source).unwrap();
        self.doc.strokes = boundary(1.5);
        self.doc.strokes.push(original.clone());
        self.selected = HashSet::from([original.id]);
        self.fills.draft = None;
        let before_strokes = self.doc.strokes.clone();
        let history_len = self.undo.len();

        self.begin_fill_contour_edit(true);
        let preview = self.fills.draft.as_ref().unwrap().mark.clone();
        assert!(preview.points.iter().any(|point| point.position.x > 1.49));
        assert_eq!(
            self.doc.strokes, before_strokes,
            "refresh preview must not mutate artwork"
        );
        assert_eq!(
            self.undo.len(),
            history_len,
            "preview must not enter history"
        );
        self.finish_fill_contour_edit(false);
        assert_eq!(
            self.doc.strokes, before_strokes,
            "cancel must preserve artwork exactly"
        );
        assert_eq!(
            self.undo.len(),
            history_len,
            "cancel must not enter history"
        );

        self.begin_fill_contour_edit(true);
        assert_eq!(self.fills.draft.as_ref().unwrap().mark, preview);
        self.finish_fill_contour_edit(true);
        assert_eq!(self.doc.strokes.last(), Some(&preview));
        assert_eq!(
            self.undo.len(),
            history_len + 1,
            "Apply must create one history edit"
        );
        self.undo();
        assert_eq!(
            self.doc.strokes, before_strokes,
            "one undo restores the source fill"
        );
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&preview));

        self.doc = saved_doc;
        self.selected = saved_selected;
        self.undo = saved_undo;
        self.redo = saved_redo;
        self.fills = saved_fills;
        self.dirty = saved_dirty;
        self.select_mode = saved_select_mode;
        self.renderer.invalidate();
    }
}
