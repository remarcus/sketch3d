use super::*;
impl SketchApp {
    pub(super) fn verify_placement(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.placement.verification_frame else {
            return;
        };
        if frame == 1 {
            self.use_cylinder = true;
            self.doc.layers[0].locked = true;
            let original = serde_json::to_vec(&self.doc).unwrap();
            let export = crate::export::encode(&self.doc, None).unwrap().bytes;
            let previous = self.guide;
            let modes = (self.use_cylinder, self.use_revolved, self.guide_visible);
            let undo = self.undo.len();
            let dirty = self.dirty;
            self.begin_placement();
            self.pointer_action(
                rect.left_top() + egui::vec2(8.0, 8.0),
                1.0,
                rect,
                true,
                false,
            );
            assert!(self.placement.hit.is_none());
            assert_eq!(self.guide, previous);
            let surface = Vec3::new(-0.7, 0.6, 0.897);
            let (screen, _) = crate::gizmo::project(&self.camera, rect, surface).unwrap();
            self.pointer_action(screen, 1.0, rect, true, false);
            let hit = self.placement.hit.unwrap();
            assert_eq!(hit.id, 1);
            assert_eq!(self.guide, previous);
            assert!(self.placement_preview().is_some());
            self.placement.flip = true;
            assert!(self.placement_preview().unwrap().basis().2.dot(hit.normal) < -0.999);
            self.placement.offset = 0.025;
            assert!(
                self.placement_preview()
                    .unwrap()
                    .origin()
                    .distance(hit.position - hit.normal * 0.025)
                    < 1e-5
            );
            self.cancel_placement();
            assert_eq!(self.guide, previous);
            assert_eq!(
                (self.use_cylinder, self.use_revolved, self.guide_visible),
                modes
            );
            assert_eq!(self.undo.len(), undo);
            assert_eq!(self.dirty, dirty);
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            assert_eq!(
                crate::export::encode(&self.doc, None).unwrap().bytes,
                export
            );
            self.begin_placement();
            self.pick_placement(screen, rect);
            self.placement.offset = 0.01;
            let accepted = self.placement_preview().unwrap();
            self.accept_placement(true);
            assert_eq!(self.guide, accepted);
            assert!(!self.placement.active);
            assert!((self.camera.basis().2.dot(accepted.basis().2) - 1.0).abs() < 1e-5);
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            self.doc.layers[0].locked = false;
            let count = self.doc.strokes.len();
            self.add_point(rect.center() + egui::vec2(-45.0, 0.0), 0.5, rect, true);
            self.add_point(rect.center() + egui::vec2(45.0, 25.0), 0.8, rect, false);
            self.finish();
            assert_eq!(self.doc.strokes.len(), count + 1);
            for point in &self.doc.strokes.last().unwrap().points {
                assert!(
                    (point.position - self.guide.origin())
                        .dot(self.guide.basis().2)
                        .abs()
                        < 1e-4
                );
            }
            self.undo();
            assert_eq!(self.doc.strokes.len(), count);
            self.redo();
            self.dirty = false;
            self.load_fill_study();
            self.doc.layers[0].visible = false;
            self.begin_placement();
            self.pick_placement(screen, rect);
            assert!(self.placement.hit.is_none());
            self.cancel_placement();
            self.doc.layers[0].visible = true;
            self.begin_placement();
            let mesh = crate::geometry::stroke_mesh(&self.doc.strokes[6]);
            let roof = mesh
                .iter()
                .take(3)
                .map(|v| Vec3::from(v.position))
                .sum::<Vec3>()
                / 3.0;
            let (screen, _) = crate::gizmo::project(&self.camera, rect, roof).unwrap();
            self.pick_placement(screen, rect);
            assert!(self.placement.hit.is_some());
            self.placement.offset = 0.015;
            self.status="Placement passed: locked/hidden rules, surface preview, normal/offset, cancellation, unchanged artwork/export, and drawing on the accepted plane.".into();
        }
        if frame == 22 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 100, "Placement screenshot timed out");
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
            let root = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                root.join("placement-preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["visible locked reference picking","hidden and missed picks preserve guide","preview flip offset cancel","native GLB and history unchanged by placement","accept and face plane","new ink constrained to accepted plane","GPU roof plane preview"],"gpu":self.renderer.adapter,"physical_tablet_tested":false,"benchmarks_deferred":true});
            std::fs::write(
                root.join("placement-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            self.cancel_placement();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.placement.verification_frame = Some(frame + 1);
    }
}
