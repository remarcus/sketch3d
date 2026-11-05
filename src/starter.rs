use super::*;
use crate::scenes;

impl SketchApp {
    pub(super) fn load_house(&mut self) {
        self.load_house_example(scenes::japanese_house(), "Japanese garden house");
    }

    pub(super) fn load_pencil_house(&mut self) {
        self.load_house_example(scenes::pencil_house(), "Pencil and pastel house");
        self.camera.target.y = 2.25;
        self.camera.target.z = 0.0;
        self.camera.distance = 12.8;
    }

    fn load_house_example(&mut self, document: Document, title: &'static str) {
        self.reset_recovery_document();
        self.doc = document;
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
        self.starter_scene = Some(title);
        self.cylinder_example = false;
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        self.resume_ink = false;
        self.status = format!(
            "{title} / editable strokes. Space + drag to orbit; V to select; New for a blank drawing."
        );
    }
    pub(super) fn verify_house(&mut self, ctx: &egui::Context, rect: Rect) {
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
                assert!(self.starter_scene.is_none());
                assert_eq!(self.next_id, 1);
                assert_eq!(self.brush, crate::presets::startup_pencil().brush);
                self.verify_pen_barrel_erase(ctx, rect);
                self.dirty = false;
                self.request(Pending::House);
                assert_eq!(self.doc.strokes, scenes::japanese_house().strokes);
                assert_eq!(self.starter_scene, Some("Japanese garden house"));
                self.request(Pending::PencilHouse);
                assert_eq!(self.doc.strokes, original);
                assert_eq!(self.starter_scene, Some("Pencil and pastel house"));
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
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("House screenshot timed out"),
            _ => {}
        }
        self.house_verification_frame = Some(frame + 1);
    }

    fn verify_pen_barrel_erase(&mut self, ctx: &egui::Context, rect: Rect) {
        let center = rect.center();
        let scale = ctx.pixels_per_point();
        let sample = |position: Pos2, phase, barrel| PenSample {
            position: Pos2::new(position.x * scale, position.y * scale),
            pressure: None,
            tilt: [None; 2],
            contact: phase != Phase::Up && phase != Phase::Cancel,
            phase,
            barrel,
            eraser: false,
        };
        let brush = self.brush;
        self.brush = Brush::default();
        self.presentation.preferences.pen_barrel_erases = true;
        let draw = [
            sample(center - Vec2::new(120.0, 0.0), Phase::Down, false),
            sample(center + Vec2::new(120.0, 0.0), Phase::Move, false),
            sample(center + Vec2::new(120.0, 0.0), Phase::Up, false),
        ];
        self.canvas_input(ctx, rect, draw.to_vec());
        assert_eq!(self.doc.strokes.len(), 1);
        let original = self.doc.strokes.clone();
        let down = sample(center, Phase::Down, true);
        let up = sample(center, Phase::Up, true);
        self.canvas_input(ctx, rect, vec![down]);
        assert!(self.partial_erase_active());
        self.canvas_input(ctx, rect, vec![sample(center, Phase::Cancel, false)]);
        assert_eq!(self.doc.strokes, original);
        self.canvas_input(
            ctx,
            rect,
            vec![down, sample(center, Phase::Move, false), up],
        );
        assert_eq!(self.doc.strokes, original);
        assert!(!self.partial_erase_active());
        self.canvas_input(ctx, rect, vec![down, up]);
        assert_ne!(self.doc.strokes, original);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.canvas_input(ctx, rect, draw.to_vec());
        assert_eq!(self.doc.strokes.len(), 2);
        self.presentation.preferences.pen_barrel_erases = false;
        self.brush = brush;
    }
}
