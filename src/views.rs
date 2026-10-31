use super::*;
use crate::geometry::Projection;
impl SketchApp {
    fn set_projection(&mut self, projection: Projection) {
        self.finish();
        self.camera.projection = projection;
    }
    fn face_world_view(&mut self, plane: Plane) {
        self.finish();
        self.camera.align_guide(DrawingPlane {
            anchor: self.camera.target,
            ..DrawingPlane::from_preset(plane)
        });
    }
    pub(super) fn view_shortcuts(&mut self, ctx: &egui::Context) {
        let (mods, toggle, front, side, top) = ctx.input(|i| {
            (
                i.modifiers,
                i.key_pressed(Key::Num5),
                i.key_pressed(Key::Num1),
                i.key_pressed(Key::Num3),
                i.key_pressed(Key::Num7),
            )
        });
        if mods.command || mods.alt {
            return;
        }
        if toggle {
            self.set_projection(if self.camera.projection == Projection::Perspective {
                Projection::Orthographic
            } else {
                Projection::Perspective
            });
        }
        if front {
            self.face_world_view(Plane::Front);
        }
        if side {
            self.face_world_view(Plane::Side);
        }
        if top {
            self.face_world_view(Plane::Ground);
        }
    }
    pub(super) fn show_view(&mut self, ctx: &egui::Context) {
        if !self.show_view
            || self.presentation.artwork
            || self.confirm
            || self.recovery_prompt
            || self.tutorial.offer
        {
            return;
        }
        let mut open = self.show_view;
        egui::Window::new("Camera view").open(&mut open).default_width(300.0).anchor(egui::Align2::RIGHT_TOP,egui::vec2(-255.0,60.0)).show(ctx,|ui| {
            ui.horizontal(|ui| {
                ui.label("Projection");
                if ui.radio(self.camera.projection==Projection::Perspective,"Perspective").clicked() {self.set_projection(Projection::Perspective);}
                if ui.radio(self.camera.projection==Projection::Orthographic,"Orthographic").clicked() {self.set_projection(Projection::Orthographic);}
            });
            ui.horizontal(|ui| {for (label,plane) in [("Front [1]",Plane::Front),("Side [3]",Plane::Side),("Top [7]",Plane::Ground)] {
                if ui.button(label).clicked() {self.face_world_view(plane);}
            }});
            if ui.button("Face active guide [F]").clicked() {self.align_active_guide();}
            ui.small("5 toggles projection. Framing matches at the camera focus plane. Orthographic view keeps parallel edges parallel; depth does not change apparent size. Navigation and guide controls remain available.");
            ui.small("Views change the camera, not the drawing plane or artwork. Camera state is session-only and excluded from native Save and GLB.");
        });
        self.show_view = open;
    }
    pub(super) fn verify_views(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.view_verification_frame else {
            return;
        };
        if frame == 1 {
            let before = serde_json::to_vec(&self.doc).unwrap();
            let export = crate::export::encode(&self.doc, None).unwrap().bytes;
            let guide = self.guide;
            let pose = self.camera;
            self.set_projection(Projection::Orthographic);
            assert_eq!(self.camera.target, pose.target);
            assert_eq!(self.camera.distance, pose.distance);
            assert_eq!(self.guide, guide);
            for plane in [Plane::Front, Plane::Side, Plane::Ground] {
                self.face_world_view(plane);
                assert!(self.camera.matrix(rect.aspect_ratio()).is_finite());
            }
            self.camera = Camera {
                projection: Projection::Orthographic,
                ..pose
            };
            let stroke = &self.doc.strokes[0];
            let point = stroke.points[0].position;
            let (screen, _) = crate::gizmo::project(&self.camera, rect, point).unwrap();
            assert!(crate::gizmo::pick(&self.doc, &self.camera, rect, screen).is_some());
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), before);
            assert_eq!(
                crate::export::encode(&self.doc, None).unwrap().bytes,
                export
            );
            self.dirty = false;
            self.request(Pending::New);
            self.set_projection(Projection::Orthographic);
            self.face_world_view(Plane::Front);
            self.add_point(rect.center() - egui::vec2(70.0, 0.0), 0.4, rect, true);
            self.add_point(rect.center() + egui::vec2(70.0, -30.0), 0.8, rect, false);
            self.finish();
            assert_eq!(self.doc.strokes.len(), 1);
            self.selected = HashSet::from([1]);
            self.transform_values = [0.2, 0.3, 0.0];
            self.numeric_transform();
            self.undo();
            self.redo();
            Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
            self.dirty = false;
            self.load_house();
            self.set_projection(Projection::Orthographic);
            self.show_view = true;
        }
        if frame == 22 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 150, "View screenshot timed out");
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = shot {
            let root = std::env::current_dir().unwrap().join("verification");
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                root.join("views-preview.png"),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            std::fs::write(root.join("views-report.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"checks":["projection switch preserves focus framing","front side top views","orthographic picking drawing transforms and history","native and GLB ink unchanged","GPU orthographic house"],"gpu":self.renderer.adapter})).unwrap()).unwrap();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.view_verification_frame = Some(frame + 1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orthographic_rays_projection_pixel_scale_and_focus_framing() {
        for distance in [0.3, 9.0, 150.0] {
            let perspective = Camera {
                distance,
                yaw: 0.4,
                pitch: 0.3,
                roll: 0.2,
                ..Default::default()
            };
            let ortho = Camera {
                projection: Projection::Orthographic,
                ..perspective
            };
            let (u, v, n) = ortho.basis();
            let point = ortho.target + u * distance * 0.1 + v * distance * 0.08;
            let p = perspective.matrix(1.3).project_point3(point);
            let q = ortho.matrix(1.3).project_point3(point);
            assert!((p.truncate() - q.truncate()).length() < 1e-5);
            let (origin, ray) = ortho.ray(q.truncate(), 1.3);
            let hit = origin + ray * ((point - origin).dot(n) / ray.dot(n));
            assert!(hit.distance(point) < 0.0003);
            assert!(ray.dot(-n) > 0.99999);
            assert!(
                (ortho.world_per_pixel(point, 700.0)
                    - ortho.world_per_pixel(point + n * 20.0, 700.0))
                .abs()
                    < 1e-7
            );
            let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0));
            let layout = crate::gizmo::Layout::new(&ortho, rect, ortho.target, Mode::Move).unwrap();
            assert!(layout.origin.distance(rect.center()) < 0.01);
        }
    }
}
