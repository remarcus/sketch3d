use super::*;

fn click(app: &mut SketchApp, _ctx: &egui::Context, rect: Rect, position: Pos2) {
    app.pointer_action(position, 1.0, rect, true, false);
    if app.curve_tool.enabled {
        app.curve_input(position, 1.0, rect, false);
        app.curve_pointer_up();
    } else {
        app.finish();
    }
}

fn drag(app: &mut SketchApp, _ctx: &egui::Context, rect: Rect, start: Pos2, end: Pos2) {
    app.pointer_action(start, 1.0, rect, true, false);
    app.pointer_action(end, 1.0, rect, false, false);
    assert!(
        app.line.draft.is_some() || app.shape_draft_active(),
        "line and shape drags create an image-plane preview before release"
    );
    if app.shape_draft_active() {
        app.release_shape(end, 1.0, rect);
    }
    if app.line.draft.is_some() {
        app.release_line(end, rect);
    }
    if app.curve_tool.enabled {
        app.curve_input(end, 1.0, rect, false);
        app.curve_pointer_up();
    } else {
        app.finish();
    }
}

impl SketchApp {
    pub(super) fn verify_image_plane_planar_tools(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
    ) -> Vec<&'static str> {
        fn assert_stroke_inside(stroke: &Stroke, image: &crate::image_planes::ImagePlane) {
            let inverse = image.rotation.inverse();
            for point in &stroke.points {
                let local = inverse * (point.position - image.origin);
                assert!(local.z.abs() < 0.002, "tool stroke stays on image plane");
                assert!(
                    local.x.abs() <= image.size.x * 0.5 + 0.002,
                    "tool stroke stays inside image width"
                );
                assert!(
                    local.y.abs() <= image.size.y * 0.5 + 0.002,
                    "tool stroke stays inside image height"
                );
            }
        }

        let image = self
            .doc
            .image_planes
            .iter()
            .find(|image| image.id == 42)
            .expect("verification image plane exists")
            .clone();
        let camera_before = self.camera;
        self.doc.active_image_plane = Some(image.id);
        self.doc.active_model_reference = None;
        self.doc.active_guide = None;
        let active_image = self
            .doc
            .image_planes
            .iter_mut()
            .find(|plane| plane.id == image.id)
            .unwrap();
        active_image.visible = true;
        active_image.locked = false;
        active_image.drawable = true;
        self.select_mode = false;
        self.eraser = false;
        let target = self
            .planar_drawing_target()
            .expect("visible drawable image plane is a planar target");
        let (axis_u, axis_v, _) = target.plane.basis();
        let screen = |app: &SketchApp, x: f32, y: f32| {
            let world = target.plane.origin() + axis_u * x + axis_v * y;
            crate::gizmo::project(&app.camera, rect, world)
                .expect("test point projects to viewport")
                .0
        };
        let mut checks = Vec::new();

        // Straight line drag at the starting zoom.
        self.camera.distance = 9.0;
        self.cancel_line_tool();
        self.start_line_tool();
        let before = self.doc.strokes.len();
        drag(
            self,
            ctx,
            rect,
            screen(self, -0.8, -0.45),
            screen(self, 0.8, -0.35),
        );
        assert_eq!(
            self.doc.strokes.len(),
            before + 1,
            "line drag commits one stroke"
        );
        let line = self.doc.strokes.last().unwrap().clone();
        assert_stroke_inside(&line, &image);
        assert!((line.brush.normal - image.rotation * Vec3::Z).length() < 0.002);
        self.undo();
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&line));
        checks.push("transformed finite image-plane line drag and undo/redo");

        // Click-to-close fill polygon at a different zoom.
        self.cancel_line_tool();
        self.camera.distance = 11.0;
        self.fills.enabled = true;
        let before = self.doc.strokes.len();
        for (index, (x, y)) in [(-0.75, -0.55), (0.75, -0.55), (0.0, 0.55), (-0.75, -0.55)]
            .into_iter()
            .enumerate()
        {
            if index == 3 {
                assert!(self.fills.draft.is_some(), "fill corners build a preview");
            }
            click(self, ctx, rect, screen(self, x, y));
            assert_eq!(
                self.fills.draft.is_some(),
                index < 3,
                "fill closure commits its preview on the first-corner click"
            );
        }
        assert_eq!(
            self.doc.strokes.len(),
            before + 1,
            "fill click closes and commits"
        );
        let fill = self.doc.strokes.last().unwrap().clone();
        assert!(fill.fill.is_some());
        assert_stroke_inside(&fill, &image);
        self.undo();
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&fill));
        self.fills.enabled = false;
        checks.push("image-plane fill click-to-close, clipping, and undo/redo");

        // Circle and ellipse previews are committed by their release events.
        self.camera.distance = 8.0;
        self.start_shape_tool(shape_tool::ShapeMode::Circle);
        let before = self.doc.strokes.len();
        drag(
            self,
            ctx,
            rect,
            screen(self, -0.7, 0.2),
            screen(self, -0.3, 0.2),
        );
        assert_eq!(
            self.doc.strokes.len(),
            before + 1,
            "circle drag commits one stroke"
        );
        let circle = self.doc.strokes.last().unwrap().clone();
        assert_stroke_inside(&circle, &image);
        self.undo();
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&circle));
        self.shape_tool.mode = None;
        checks.push("rotated image-plane circle drag, clipping, and undo/redo");

        self.camera.distance = 10.0;
        self.start_shape_tool(shape_tool::ShapeMode::Ellipse);
        let before = self.doc.strokes.len();
        drag(
            self,
            ctx,
            rect,
            screen(self, -0.6, -0.65),
            screen(self, 0.6, 0.65),
        );
        assert_eq!(
            self.doc.strokes.len(),
            before + 1,
            "ellipse drag commits one stroke"
        );
        let ellipse = self.doc.strokes.last().unwrap().clone();
        assert_stroke_inside(&ellipse, &image);
        self.undo();
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&ellipse));
        self.shape_tool.mode = None;
        checks.push("rotated image-plane ellipse drag, clipping, and undo/redo");

        // Smooth curve anchor clicks at another zoom.
        self.camera.distance = 9.5;
        self.start_curve_tool();
        let before = self.doc.strokes.len();
        for (x, y) in [(-0.85, -0.1), (-0.3, 0.45), (0.3, -0.4), (0.85, 0.1)] {
            click(self, ctx, rect, screen(self, x, y));
        }
        assert!(self.curve_draft_active(), "curve anchors build a preview");
        self.commit_curve();
        assert_eq!(
            self.doc.strokes.len(),
            before + 1,
            "curve finish commits one stroke"
        );
        let curve = self.doc.strokes.last().unwrap().clone();
        assert_stroke_inside(&curve, &image);
        self.undo();
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&curve));
        self.cancel_curve_tool();
        checks.push("image-plane curve anchor clicks, clipping, and undo/redo");

        // Tool previews cancel without creating document edits.
        let before = self.doc.strokes.len();
        self.start_line_tool();
        self.pointer_action(screen(self, 0.0, 0.0), 1.0, rect, true, false);
        assert!(self.line.draft.is_some());
        self.cancel();
        assert!(self.line.draft.is_none());
        assert_eq!(self.doc.strokes.len(), before);
        self.cancel_line_tool();

        self.fills.enabled = true;
        click(self, ctx, rect, screen(self, -0.5, 0.0));
        click(self, ctx, rect, screen(self, 0.5, 0.0));
        assert!(self.fills.draft.is_some());
        self.cancel_fill_tool();
        assert_eq!(self.doc.strokes.len(), before);

        self.start_shape_tool(shape_tool::ShapeMode::Circle);
        self.pointer_action(screen(self, -0.5, 0.0), 1.0, rect, true, false);
        self.pointer_action(screen(self, 0.0, 0.0), 1.0, rect, false, false);
        assert!(self.shape_draft_active());
        self.cancel();
        assert!(!self.shape_draft_active());
        assert_eq!(self.doc.strokes.len(), before);
        self.shape_tool.mode = None;

        self.start_curve_tool();
        click(self, ctx, rect, screen(self, -0.5, 0.0));
        assert!(self.curve_draft_active());
        self.cancel_curve_tool();
        assert_eq!(self.doc.strokes.len(), before);
        checks.push("line, fill, circle, and curve cancel without document edits");

        // Hidden or locked image planes are rejected by each planar tool.
        for locked in [false, true] {
            let plane = self
                .doc
                .image_planes
                .iter_mut()
                .find(|plane| plane.id == image.id)
                .unwrap();
            plane.visible = locked;
            plane.locked = locked;
            self.doc.active_image_plane = Some(image.id);
            let before = self.doc.strokes.len();
            self.cancel_line_tool();
            self.start_line_tool();
            self.line_input(screen(self, 0.0, 0.0), 1.0, None, rect, true);
            assert!(self.line.draft.is_none());
            self.start_shape_tool(shape_tool::ShapeMode::Ellipse);
            self.shape_input(screen(self, 0.0, 0.0), 1.0, rect, true);
            assert!(!self.shape_draft_active());
            self.start_curve_tool();
            self.curve_input(screen(self, 0.0, 0.0), 1.0, rect, true);
            assert!(!self.curve_draft_active());
            self.fills.enabled = true;
            self.fill_corner(screen(self, 0.0, 0.0), rect);
            assert!(self.fills.draft.is_none());
            self.fills.enabled = false;
            assert_eq!(self.doc.strokes.len(), before);
            let plane = self
                .doc
                .image_planes
                .iter_mut()
                .find(|plane| plane.id == image.id)
                .unwrap();
            plane.visible = true;
            plane.locked = false;
        }
        self.doc.active_image_plane = Some(image.id);
        self.cancel_line_tool();
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.shape_tool.mode = None;
        self.cancel_fill_tool();
        checks.push("hidden and locked image planes reject all planar tools");
        self.camera = camera_before;
        self.renderer.invalidate();
        checks
    }
}
