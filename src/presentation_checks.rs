use super::*;
impl SketchApp {
    pub(super) fn verify_presentation(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.presentation.verification_frame else {
            return;
        };
        if frame == 1 {
            let original = serde_json::to_vec(&self.doc).unwrap();
            let export = crate::export::encode(&self.doc, None).unwrap().bytes;
            let history = (self.undo.len(), self.redo.len());
            let dirty = self.dirty;
            let lighting = self.renderer.lighting;
            for color in [
                crate::presentation::BLUE,
                crate::presentation::LIGHT,
                crate::presentation::Preferences::default().background,
            ] {
                self.presentation.preferences.background = color;
                self.renderer.background = color;
                self.renderer.render(
                    &Document::default(),
                    &self.camera,
                    GuidePreview::Hidden,
                    [64, 32],
                );
                let pixels = self.renderer.verification_pixels();
                for (actual, expected) in pixels[0..3].iter().zip(color) {
                    assert!(
                        (*actual as f32 - expected * 255.0).abs() < 1.5,
                        "Incorrect sRGB clear color: {actual} vs {expected}"
                    );
                }
            }
            self.presentation.preferences.background = crate::presentation::BLUE;
            self.presentation.remember();
            let restarted = super::presentation_ui::PresentationUi::new();
            assert_eq!(restarted.preferences.background, crate::presentation::BLUE);
            assert!(!restarted.artwork);
            self.fills.enabled = true;
            self.fill_corner(rect.center(), rect);
            assert!(self.fills.draft.is_some());
            self.toggle_artwork();
            assert!(!self.presentation.artwork);
            self.cancel_fill_tool();
            self.begin_placement();
            self.toggle_artwork();
            assert!(!self.presentation.artwork);
            self.cancel_placement();
            self.selected = self.doc.strokes.iter().take(2).map(|s| s.id).collect();
            self.select_mode = true;
            let selection = self.selected.clone();
            let guide = self.guide;
            let visible = self.guide_visible;
            self.show_scene = true;
            self.toggle_artwork();
            assert!(self.presentation.artwork);
            self.pointer_action(rect.center(), 1.0, rect, true, false);
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            self.resume_ink = true;
            self.toggle_artwork();
            assert!(!self.presentation.artwork);
            assert!(!self.resume_ink && !self.mouse_down);
            assert_eq!(self.selected, selection);
            assert_eq!(self.guide, guide);
            assert_eq!(self.guide_visible, visible);
            assert!(self.show_scene);
            assert_eq!((self.undo.len(), self.redo.len()), history);
            assert_eq!(self.dirty, dirty);
            assert_eq!(self.renderer.lighting, lighting);
            self.toggle_artwork();
            let directory = tempfile::tempdir().unwrap();
            let source = self.file.clone();
            self.file = Some(directory.path().join("missing/drawing.sketch.json"));
            assert!(!self.save(false));
            assert!(!self.presentation.artwork);
            self.file = source;
            self.toggle_artwork();
            self.recovery_prompt = true;
            self.ensure_workspace();
            assert!(!self.presentation.artwork);
            self.recovery_prompt = false;
            self.toggle_artwork();
            self.dirty = true;
            self.request(Pending::New);
            assert!(!self.presentation.artwork && self.confirm);
            self.pending = None;
            self.confirm = false;
            self.dirty = dirty;
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            assert_eq!(
                crate::export::encode(&self.doc, None).unwrap().bytes,
                export
            );
            self.renderer.invalidate();
            self.toggle_artwork();
            self.status="Presentation passed: sRGB backgrounds, explicit restart preference, read-only focus, workspace restore and recovery/unsaved guards.".into();
        }
        if frame == 22 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 150, "Presentation screenshot timed out");
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
                root.join("presentation-preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["blue light dark sRGB GPU colors","explicit background preference survives restart","focus never persisted","draft and preview entry guards","read-only focus and selection/guide/history restoration","recovery and unsaved prompts leave focus","native ink GLB materials and lighting unchanged","GPU artwork-only canvas and return controls"],"gpu":self.renderer.adapter,"benchmarks_deferred":true});
            std::fs::write(
                root.join("presentation-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.presentation.verification_frame = Some(frame + 1);
    }
}
