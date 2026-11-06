use super::*;
use crate::scenes;

impl SketchApp {
    pub(super) fn load_house(&mut self) {
        self.load_house_example(scenes::japanese_house(), "Japanese garden house");
    }

    pub(super) fn load_pencil_house(&mut self) {
        self.load_house_example(scenes::pencil_house(), "Pencil and pastel house");
        self.camera.yaw = 0.66;
        self.camera.pitch = 0.31;
        self.camera.target = Vec3::new(0.0, 2.35, 0.12);
        self.camera.distance = 14.1;
        self.guide_visible = false;
        self.doc.active_guide = None;
        for guide in &mut self.doc.guide_resources {
            guide.visible = false;
        }
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
                assert!(original.len() >= 30);
                assert!(
                    original
                        .iter()
                        .filter(|stroke| stroke.fill.is_some())
                        .count()
                        >= 9
                );
                assert!(original.iter().any(|stroke| stroke.layer_id == 3));
                assert!(original.iter().any(|stroke| stroke.layer_id == 4));
                self.verify_house_guides(ctx, rect);
                assert_eq!(self.doc.strokes, original);
                let id = original[original.len() / 2].id;
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
                self.verify_quick_palette();
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

    fn verify_quick_palette(&mut self) {
        let context = egui::Context::default();
        let mut shortcut = |key: Option<Key>, focused| {
            let events = key.map_or_else(Vec::new, |key| {
                [true, false]
                    .map(|pressed| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .to_vec()
            });
            let _ = context.run(
                egui::RawInput {
                    focused,
                    events,
                    ..Default::default()
                },
                |ctx| self.quick_palette_shortcuts(ctx),
            );
            self.quick_palette.is_some()
        };
        assert!(shortcut(Some(Key::Q), true));
        assert!(shortcut(Some(Key::ArrowDown), true));
        assert!(!shortcut(Some(Key::Enter), true));
        assert!(shortcut(Some(Key::Q), true));
        assert!(!shortcut(Some(Key::Escape), true));
        assert!(shortcut(Some(Key::Q), true));
        assert!(!shortcut(None, false));
        assert_eq!(self.quick_palette_active_tool(), "Line");
        self.choose_tool("Brush");
        assert!(std::mem::take(&mut self.suppress_canvas_input_once));
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

    fn verify_house_guides(&mut self, ctx: &egui::Context, rect: Rect) {
        let initial_camera = self.camera;
        let initial_guide = self.doc.active_guide;
        let initial_stroke_count = self.doc.strokes.len();
        let initial_undo_count = self.undo.len();
        let scale = ctx.pixels_per_point();
        let sample = |position: Pos2, phase| PenSample {
            position: Pos2::new(position.x * scale, position.y * scale),
            pressure: None,
            tilt: [None; 2],
            contact: phase != Phase::Up,
            phase,
            barrel: false,
            eraser: false,
        };
        for id in 1..=3 {
            let index = self
                .doc
                .guide_resources
                .iter()
                .position(|guide| guide.id == id)
                .unwrap();
            let was_visible = self.doc.guide_resources[index].visible;
            self.doc.guide_resources[index].visible = true;
            self.camera.yaw += 0.1;
            self.begin_plane_picker();
            assert!(self.plane_picker.active);
            let mut positions = None;
            'search: for y in (32..rect.height() as i32 - 32).step_by(8) {
                for x in (32..rect.width() as i32 - 40).step_by(8) {
                    let start = rect.min + Vec2::new(x as f32, y as f32);
                    let end = start + Vec2::new(12.0, 0.0);
                    self.update_plane_picker_hover(Some(start), rect);
                    if self.plane_picker.hovered_guide.map(|(id, _)| id) != Some(id) {
                        continue;
                    }
                    self.update_plane_picker_hover(Some(end), rect);
                    if self.plane_picker.hovered_guide.map(|(id, _)| id) == Some(id) {
                        positions = Some((start, end));
                        break 'search;
                    }
                }
            }
            let (start, end) = positions.unwrap_or_else(|| panic!("Guide {id} is not pickable"));
            self.doc.guide_resources[index].locked = true;
            self.update_plane_picker_hover(Some(start), rect);
            assert_ne!(self.plane_picker.hovered_guide.map(|(id, _)| id), Some(id));
            self.doc.guide_resources[index].locked = false;
            self.doc.guide_resources[index].visible = false;
            self.update_plane_picker_hover(Some(start), rect);
            assert_ne!(self.plane_picker.hovered_guide.map(|(id, _)| id), Some(id));
            self.doc.guide_resources[index].visible = true;
            let previous = self.doc.active_guide;
            self.cancel_plane_picker();
            assert_eq!(self.doc.active_guide, previous);
            self.begin_plane_picker();
            self.canvas_input(
                ctx,
                rect,
                vec![sample(start, Phase::Down), sample(start, Phase::Up)],
            );
            assert_eq!(self.doc.active_guide, Some(id));
            assert!(!self.plane_picker.active);
            self.canvas_input(
                ctx,
                rect,
                vec![
                    sample(start, Phase::Down),
                    sample(end, Phase::Move),
                    sample(end, Phase::Up),
                ],
            );
            assert_eq!(self.doc.strokes.len(), initial_stroke_count + id as usize);
            assert_eq!(
                self.doc.strokes.last().unwrap().brush.texture.kind,
                self.brush.texture.kind
            );
            self.doc.guide_resources[index].visible = was_visible;
        }
        self.doc.active_guide = initial_guide;
        if let Some(active_id) = initial_guide {
            if let Some(guide) = self
                .doc
                .guide_resources
                .iter_mut()
                .find(|guide| guide.id == active_id)
            {
                guide.visible = true;
            }
        }
        let reopened = Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes, self.doc.strokes);
        assert_eq!(
            self.undo.len(),
            initial_undo_count + 5 + usize::from(initial_guide.is_none())
        );
        while self.undo.len() > initial_undo_count {
            self.undo();
        }
        assert_eq!(self.doc.active_guide, initial_guide);
        assert_eq!(self.doc.strokes.len(), initial_stroke_count);
        self.camera = initial_camera;
    }
}
