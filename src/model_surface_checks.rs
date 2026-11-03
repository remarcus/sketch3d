use super::*;
use crate::{
    document::{model_references::ObjReferenceResource, Brush},
    geometry::Plane,
};

impl SketchApp {
    /// Exercise the production brush pointer path against a synthetic OBJ mesh.
    pub(super) fn verify_model_surfaces(&mut self, ctx: &egui::Context, rect: Rect) {
        if self.model_surface_verification_frame != Some(0) {
            return;
        }

        self.doc = Document::default();
        self.next_id = 1;
        self.undo.clear();
        self.redo.clear();
        self.active = None;
        self.model_stroke_contact = None;
        self.resume_ink = false;
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.curve_tool.enabled = false;
        self.fills.enabled = false;
        self.shape_tool.mode = None;
        self.stabilization = 0.0;
        self.camera.target = Vec3::ZERO;
        self.camera.align(Plane::Front);
        self.camera.distance = 6.0;
        self.brush = Brush {
            dabs: crate::dynamics::DabSettings {
                enabled: true,
                spacing: 0.5,
                scatter: 0.25,
                ..Default::default()
            },
            ..Brush::default()
        };
        self.radius = 0.1;

        const MODEL_ID: u64 = 9401;
        let resource = ObjReferenceResource {
            id: MODEL_ID,
            name: "Model-surface verification mesh".into(),
            visible: true,
            locked: false,
            center: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: 1.0,
            asset: crate::obj_reference_storage::ObjReferenceAsset {
                id: MODEL_ID,
                bundle_dir: format!("models/{MODEL_ID}"),
            },
        };
        self.doc.model_references.push(resource);
        self.doc.active_model_reference = Some(MODEL_ID);

        let adjacent_triangles =
            parse_fixture("v -1 -1 0\nv 1 -1 0\nv 1 1 0\nv -1 1 0\nf 1 2 3\nf 1 3 4\n");
        self.obj_reference_models
            .insert(MODEL_ID, adjacent_triangles);
        self.obj_surface_indices.clear();
        self.refresh_obj_reference_meshes();

        let camera = self.camera;
        let screen_at = |x: f32, y: f32| {
            crate::gizmo::project(&camera, rect, Vec3::new(x, y, 0.0))
                .expect("model pointer sample projects")
                .0
        };
        let across_shared_edge: Vec<_> = (0..=64)
            .map(|step| {
                let value = -0.8 + step as f32 * 0.025;
                screen_at(value, value)
            })
            .collect();
        assert!(across_shared_edge.iter().all(|point| rect.contains(*point)));
        for (index, point) in across_shared_edge.iter().copied().enumerate() {
            self.pointer_action(point, 0.7, rect, index == 0, false);
        }
        self.finish();
        assert_eq!(
            self.doc.strokes.len(),
            1,
            "a shared manifold edge stays in one stroke"
        );
        let first = &self.doc.strokes[0];
        assert!(
            first.points.len() >= 60,
            "dense pointer samples survive surface drawing"
        );
        assert!(first
            .points
            .iter()
            .all(|point| point.position.z.abs() < 0.0001));
        let first_dabs = crate::dab_placement::sample(first).unwrap();
        assert!(!first_dabs.is_empty());
        assert!(first_dabs.iter().all(|dab| dab.position.z.abs() < 0.0001));
        assert!(
            first.model_surface.is_some(),
            "model dabs retain their surface patch"
        );
        assert_eq!(
            first_dabs,
            crate::dab_placement::sample(first).unwrap(),
            "seeded model scatter is deterministic"
        );
        let first_mesh = crate::geometry::stroke_mesh(first);
        assert!(
            !first_mesh.is_empty(),
            "model dabs produce resolved geometry"
        );
        assert!(first_mesh
            .iter()
            .all(|vertex| vertex.position[2].abs() < 0.0001));
        let adjacent_strokes = self.doc.strokes.clone();
        assert_eq!(self.undo.len(), 1, "model stroke creates one history edit");
        self.undo();
        assert!(
            self.doc.strokes.is_empty(),
            "one undo removes the model stroke"
        );
        self.redo();
        assert_eq!(
            self.doc.strokes, adjacent_strokes,
            "redo restores exact model samples"
        );

        let separated_triangles = parse_fixture(
            "v -2 -1 0\nv -1 -1 0\nv -1 1 0\nv 1 -1 0\nv 2 -1 0\nv 1 1 0\nf 1 2 3\nf 4 5 6\n",
        );
        self.obj_reference_models
            .insert(MODEL_ID, separated_triangles);
        self.obj_surface_indices.clear();
        self.refresh_obj_reference_meshes();
        self.doc.strokes.clear();
        self.undo.clear();
        self.redo.clear();
        self.next_id = 1;

        let left_run: Vec<_> = (0..=6)
            .map(|step| screen_at(-1.7 + step as f32 * 0.05, -0.3))
            .collect();
        let miss = screen_at(0.0, 0.0);
        let right_run: Vec<_> = (0..=6)
            .map(|step| screen_at(1.1 + step as f32 * 0.05, -0.3))
            .collect();
        assert!(left_run
            .iter()
            .chain(&right_run)
            .all(|point| rect.contains(*point)));
        self.pointer_action(left_run[0], 0.5, rect, true, false);
        for point in left_run.iter().copied().skip(1) {
            self.pointer_action(point, 0.6, rect, false, false);
        }
        self.pointer_action(miss, 0.7, rect, false, false);
        assert!(
            self.active.is_none(),
            "a surface miss commits the first run"
        );
        self.pointer_action(right_run[0], 0.8, rect, false, false);
        for point in right_run.iter().copied().skip(1) {
            self.pointer_action(point, 0.9, rect, false, false);
        }
        self.finish();
        assert_eq!(
            self.doc.strokes.len(),
            2,
            "a missed region splits a pointer gesture"
        );
        assert_eq!(
            self.undo.len(),
            2,
            "each separated run is independently undoable"
        );
        assert!(self.doc.strokes[0]
            .points
            .iter()
            .all(|point| point.position.x < -1.0));
        assert!(self.doc.strokes[1]
            .points
            .iter()
            .all(|point| point.position.x > 1.0));
        for stroke in &self.doc.strokes {
            let dabs = crate::dab_placement::sample(stroke).unwrap();
            assert!(!dabs.is_empty());
            assert!(
                if stroke.points[0].position.x < 0.0 {
                    dabs.iter().all(|dab| dab.position.x < -1.0)
                } else {
                    dabs.iter().all(|dab| dab.position.x > 1.0)
                },
                "zero-scatter dabs remain on each separate model-surface run"
            );
        }
        let split_strokes = self.doc.strokes.clone();
        self.undo();
        assert_eq!(
            self.doc.strokes.len(),
            1,
            "undo removes only the resumed run"
        );
        self.redo();
        assert_eq!(
            self.doc.strokes, split_strokes,
            "redo restores both split runs"
        );

        for stroke in &self.doc.strokes {
            assert!(stroke.model_surface.is_some());
            let dabs = crate::dab_placement::sample(stroke).unwrap();
            assert!(!dabs.is_empty());
            assert!(
                if stroke.points[0].position.x < 0.0 {
                    dabs.iter().all(|dab| dab.position.x < -1.0)
                } else {
                    dabs.iter().all(|dab| dab.position.x > 1.0)
                },
                "scattered model dabs do not bridge disconnected components"
            );
            assert!(crate::geometry::stroke_mesh(stroke)
                .iter()
                .all(|vertex| vertex.position[2].abs() < 0.0001));
        }
        let serialized = serde_json::to_string(&self.doc).unwrap();
        let reopened = Document::from_json(&serialized).unwrap();
        assert_eq!(
            reopened.strokes, self.doc.strokes,
            "model patches survive native save/reload"
        );

        self.status = "Model-surface pointer checks passed.".into();
        self.dirty = false;
        self.renderer.invalidate();
        self.model_surface_verification_frame = Some(1);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
}

fn parse_fixture(source: &str) -> crate::obj_reference::ObjModel {
    crate::obj_reference::parse(source.as_bytes(), std::path::Path::new("verification.obj"))
        .expect("synthetic OBJ fixture parses")
}
