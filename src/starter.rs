use super::*;
use crate::scenes;

impl SketchApp {
    pub(super) fn load_house(&mut self) {
        self.reset_recovery_document();
        self.doc = scenes::japanese_house();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.camera = scenes::house_camera();
        self.guide = DrawingPlane::from_preset(Plane::Ground);
        self.guide.depth = -0.08;
        self.file = None;
        self.dirty = false;
        self.active = None;
        self.drag = None;
        self.mouse_down = false;
        self.stabilizer.reset();
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.renderer.invalidate();
        self.starter_scene = true;
        self.cylinder_example = false;
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        self.resume_ink = false;
        self.status="Japanese garden house / editable strokes. Space + drag to orbit; V to select; New for a blank drawing.".into();
    }
    pub(super) fn verify_house(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.house_verification_frame else {
            return;
        };
        match frame {
            1 => {
                let original = self.doc.strokes.clone();
                assert!(original.len() > 300);
                let id = original[60].id;
                self.selected = HashSet::from([id]);
                self.duplicate_selected();
                assert_eq!(self.doc.strokes.len(), original.len() + 1);
                self.undo();
                assert_eq!(self.doc.strokes, original);
                self.selected.clear();
                self.dirty = true;
                self.request(Pending::New);
                assert!(self.confirm);
                assert_eq!(self.doc.strokes, original);
                self.pending = None;
                self.confirm = false;
                self.dirty = false;
                self.request(Pending::New);
                assert!(self.doc.strokes.is_empty());
                assert!(!self.starter_scene);
                assert_eq!(self.next_id, 1);
                self.request(Pending::House);
                assert_eq!(self.doc.strokes, original);
                assert!(self.starter_scene);
                assert_eq!(self.next_id, original.len() as u64 + 1);
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("house.sketch.json"));
                assert!(self.save(false));
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.strokes, original);
                self.file = None;
                self.status = "Japanese garden house / editable strokes. Space + drag to orbit; V to select; New for a blank drawing.".into();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("House screenshot timed out"),
            _ => {}
        }
        self.house_verification_frame = Some(frame + 1);
    }
}
