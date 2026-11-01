use super::*;

fn presentation_still_exports(
    app: &mut SketchApp,
    ctx: &egui::Context,
    theme: crate::presentation::UiTheme,
    output: &std::path::Path,
) -> std::collections::BTreeMap<String, Vec<u8>> {
    app.presentation.set_ui_theme(theme);
    app.presentation.apply_theme(ctx);

    let size = [192, 144];
    let camera = app.camera;
    let lighting = app.renderer.lighting;
    let canvas = app.presentation.preferences.background;
    let mut outputs = std::collections::BTreeMap::new();

    let png = app
        .renderer
        .render_png_pixels(&app.doc, &camera, size, canvas, lighting)
        .expect("theme-invariance PNG render");
    let png_path = output.join(format!("{theme:?}-opaque.png"));
    super::png_export_ui::write_png_atomic(&png_path, size, &png)
        .expect("theme-invariance production PNG encoder");
    outputs.insert(
        "opaque-png".into(),
        std::fs::read(png_path).expect("read theme-invariance PNG"),
    );

    let transparent = app
        .renderer
        .render_transparent_png_pixels(&app.doc, &camera, size, lighting)
        .expect("theme-invariance transparent PNG render");
    let transparent_path = output.join(format!("{theme:?}-transparent.png"));
    super::png_export_ui::write_png_atomic(&transparent_path, size, &transparent)
        .expect("theme-invariance transparent PNG encoder");
    outputs.insert(
        "transparent-png".into(),
        std::fs::read(transparent_path).expect("read theme-invariance transparent PNG"),
    );

    for (name, background) in [("canvas", canvas), ("white", [1.0; 3]), ("black", [0.0; 3])] {
        let pixels = app
            .renderer
            .render_png_pixels(&app.doc, &camera, size, background, lighting)
            .expect("theme-invariance JPEG render");
        let path = output.join(format!("{theme:?}-{name}.jpg"));
        super::png_export_ui::write_jpeg_atomic(&path, size, &pixels)
            .expect("theme-invariance production JPEG encoder");
        outputs.insert(
            format!("jpeg-{name}"),
            std::fs::read(path).expect("read theme-invariance JPEG"),
        );
    }

    outputs
}

impl SketchApp {
    pub(super) fn verify_presentation(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.presentation.verification_frame else {
            return;
        };
        if frame == 1 {
            let original = serde_json::to_vec(&self.doc).unwrap();
            let export = crate::export::encode(&self.doc, None).unwrap().bytes;
            let still_output = tempfile::tempdir_in(
                std::env::current_dir()
                    .expect("workspace directory")
                    .join("verification"),
            )
            .expect("theme-invariance export folder");
            let dark_stills = presentation_still_exports(
                self,
                ctx,
                crate::presentation::UiTheme::Dark,
                still_output.path(),
            );
            let light_stills = presentation_still_exports(
                self,
                ctx,
                crate::presentation::UiTheme::Light,
                still_output.path(),
            );
            assert_eq!(
                dark_stills.len(),
                5,
                "all expected still formats are covered"
            );
            assert_eq!(
                dark_stills, light_stills,
                "opaque/transparent PNG and Canvas/White/Black JPEG outputs are independent of UI theme"
            );
            let history = (self.undo.len(), self.redo.len());
            let dirty = self.dirty;
            let lighting = self.renderer.lighting;
            let camera = self.camera;
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
            let fill_draft = self
                .fills
                .draft
                .as_ref()
                .map(|draft| serde_json::to_vec(&draft.mark).unwrap())
                .unwrap();
            let canvas_background = self.presentation.preferences.background;
            let scene_background = self.renderer.background;
            self.presentation
                .set_ui_theme(crate::presentation::UiTheme::Light);
            self.presentation.apply_theme(ctx);
            assert!(!ctx.style().visuals.dark_mode, "Light is applied live");
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            assert_eq!(
                crate::export::encode(&self.doc, None).unwrap().bytes,
                export
            );
            assert_eq!((self.undo.len(), self.redo.len()), history);
            assert_eq!(self.dirty, dirty);
            assert_eq!(self.camera.yaw, camera.yaw);
            assert_eq!(self.camera.pitch, camera.pitch);
            assert_eq!(self.camera.roll, camera.roll);
            assert_eq!(self.camera.distance, camera.distance);
            assert_eq!(self.camera.target, camera.target);
            assert_eq!(self.camera.projection, camera.projection);
            assert_eq!(
                serde_json::to_vec(&self.fills.draft.as_ref().unwrap().mark).unwrap(),
                fill_draft
            );
            assert_eq!(self.presentation.preferences.background, canvas_background);
            assert_eq!(self.renderer.background, scene_background);
            assert_eq!(self.renderer.lighting, lighting);
            self.presentation
                .set_ui_theme(crate::presentation::UiTheme::Dark);
            self.presentation.apply_theme(ctx);
            assert!(ctx.style().visuals.dark_mode, "Dark is restored live");
            assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
            assert_eq!((self.undo.len(), self.redo.len()), history);
            assert_eq!(self.dirty, dirty);
            assert_eq!(self.camera.distance, camera.distance);
            assert_eq!(
                serde_json::to_vec(&self.fills.draft.as_ref().unwrap().mark).unwrap(),
                fill_draft
            );
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
            assert!(
                self.line.draft.is_none()
                    && self.fills.draft.is_none()
                    && !self.placement.active
                    && self.tutorial.practice.is_none()
                    && !self.tutorial.offer
                    && !self.show_help
                    && !self.recovery_prompt
                    && !self.confirm,
                "artwork-only verifier is blocked: line={}, fill={}, placement={}, tutorial_practice={}, tutorial_offer={}, help={}, recovery={}, confirm={}",
                self.line.draft.is_some(),
                self.fills.draft.is_some(),
                self.placement.active,
                self.tutorial.practice.is_some(),
                self.tutorial.offer,
                self.show_help,
                self.recovery_prompt,
                self.confirm
            );
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
            self.status="Presentation passed: live theme switching preserves camera, fill draft, history, unsaved artwork, PNG/JPEG/GLB exports, and stored artwork colors; GIF frame invariance, sRGB backgrounds, restart preference, artwork-only focus, workspace restore and recovery guards also pass.".into();
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
            let report = serde_json::json!({"passed":true,"checks":["live Dark/Light application switching preserves camera, fill draft, undo/redo, unsaved document and GLB output","opaque and transparent PNG files from the production encoders are byte-identical across Dark/Light UI themes","Canvas, White, and Black JPEG files from the production encoder are byte-identical across Dark/Light UI themes","scene background and renderer lighting remain independent from UI theme","blue light dark sRGB GPU colors","explicit background preference survives restart","focus never persisted","draft and preview entry guards","read-only focus and selection/guide/history restoration","recovery and unsaved prompts leave focus","native ink GLB materials and lighting unchanged","GPU artwork-only canvas and return controls"],"gpu":self.renderer.adapter,"benchmarks_deferred":true});
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
