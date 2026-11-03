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
            assert_eq!(loaded.version, 27);
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
            self.status="Filled shapes passed: concavity, draft safety, picking, groups, transforms, history, native save, and depth-tested outlines.".into();
        }
        if frame == 3 {
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
    }
}
