use super::*;

impl SketchApp {
    /// Run the production mouse event handler through cancel and apply/undo/redo.
    pub(super) fn verify_guide_bend(&mut self, ctx: &egui::Context, rect: Rect) {
        if !self.guide_bend_verification {
            return;
        }
        self.guide_bend_verification = false;
        self.camera.align(crate::geometry::Plane::Front);
        self.doc = Document::default();
        self.doc
            .guide_resources
            .push(crate::document::guide_resources::GuideResource {
                id: 77_201,
                name: "Mouse bent guide example".into(),
                visible: true,
                locked: false,
                geometry: crate::document::guide_resources::GuideGeometry::Plane {
                    origin: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                },
            });
        self.doc.active_guide = Some(77_201);
        self.undo.clear();
        self.redo.clear();
        let event_ctx = egui::Context::default();
        let _ = event_ctx.run(
            egui::RawInput {
                screen_rect: Some(ctx.screen_rect()),
                focused: true,
                ..Default::default()
            },
            |_| {},
        );
        let source = self.doc.guide_resources.clone();
        let controls = [
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::new(-0.7, 1.25, 0.0),
            Vec3::new(0.7, 1.25, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        let mouse_click = |app: &mut SketchApp, world: Vec3| {
            let (position, _) = crate::gizmo::project(&app.camera, rect, world)
                .expect("mouse bend control should project into the canvas");
            assert!(
                canvas_pointer_allowed(&event_ctx, rect, position),
                "projected bend control must be on the canvas: {position:?}"
            );
            let event = egui::Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            };
            app.guide_bend_canvas_input(&event_ctx, rect, &[event], &[]);
        };

        self.begin_guide_bend(77_201);
        mouse_click(self, controls[0]);
        mouse_click(self, controls[1]);
        let escape = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        self.guide_bend_canvas_input(&event_ctx, rect, &[escape.clone()], &[]);
        assert_eq!(
            self.doc.guide_resources, source,
            "Escape cancels a partial mouse bend"
        );
        assert!(self.undo.is_empty(), "cancel must not add history");

        self.begin_guide_bend(77_201);
        for point in controls {
            mouse_click(self, point);
        }
        assert_eq!(
            self.doc.guide_resources, source,
            "mouse preview must not mutate saved guide"
        );
        assert!(
            self.guide_bend_draft
                .as_ref()
                .and_then(|draft| draft.preview)
                .is_some(),
            "mouse curve should produce a valid preview: status={:?}, draft={:?}",
            self.status,
            self.guide_bend_draft
                .as_ref()
                .map(|draft| (&draft.controls, &draft.error))
        );
        self.guide_bend_canvas_input(&event_ctx, rect, &[escape.clone()], &[]);
        assert_eq!(
            self.doc.guide_resources, source,
            "Escape cancels a complete bend preview"
        );
        assert!(
            self.undo.is_empty(),
            "cancelled preview must not add history"
        );

        self.begin_guide_bend(77_201);
        for point in controls {
            mouse_click(self, point);
        }
        assert!(self
            .guide_bend_draft
            .as_ref()
            .and_then(|draft| draft.preview)
            .is_some());
        let before_apply_undo = self.undo.len();
        self.apply_guide_bend();
        assert_eq!(
            self.undo.len(),
            before_apply_undo + 1,
            "Apply creates one undo edit"
        );
        let bent = self.doc.guide_resources.clone();
        assert!(matches!(
            bent[0].geometry,
            crate::document::guide_resources::GuideGeometry::Freeform { .. }
        ));
        self.undo();
        assert_eq!(
            self.doc.guide_resources, source,
            "undo restores the source plane"
        );
        self.redo();
        assert_eq!(
            self.doc.guide_resources, bent,
            "redo restores the bent surface"
        );

        let surface = match &self.doc.guide_resources[0].geometry {
            crate::document::guide_resources::GuideGeometry::Freeform { surface } => *surface,
            _ => unreachable!(),
        };
        let example_stroke = Stroke {
            point_thickness: None,
            modifiers: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 77_202,
            points: (0..=20)
                .map(|index| crate::document::Point {
                    position: surface.point(index as f32 / 20.0, 0.5),
                    pressure: 0.8,
                    tilt: None,
                    time_ms: index,
                })
                .collect(),
            color: [0.88, 0.32, 0.12],
            radius: 0.045,
            gradient: None,
            brush: Brush::default(),
            curve: None,
        };
        self.doc.strokes.push(example_stroke);
        let output = std::env::current_dir()
            .unwrap()
            .join("verification")
            .join("guide-bend");
        std::fs::create_dir_all(&output).unwrap();
        let example_path = output.join("mouse-organic-example.sketch.json");
        std::fs::write(&example_path, serde_json::to_vec_pretty(&self.doc).unwrap()).unwrap();
        let roundtrip =
            Document::from_json(&std::fs::read_to_string(&example_path).unwrap()).unwrap();
        assert_eq!(roundtrip.guide_resources, self.doc.guide_resources);
        assert_eq!(roundtrip.strokes, self.doc.strokes);
        let export = crate::export::encode(&roundtrip, None).expect("bent guide example exports");
        assert!(export.summary.vertices > 0);
        let report = serde_json::json!({
            "passed": true,
            "checks": [
                "four projected mouse clicks create a bounded bend preview without changing the saved guide",
                "Escape cancels a partial mouse bend without history",
                "Escape cancels a complete bend preview without history",
                "Apply commits one guide edit and undo/redo restore exact source and bent resources",
                "mouse-shaped organic guide and new world-space stroke survive native save/reopen and GLB export"
            ],
            "existing_artwork_policy": "Bending changes only the saved guide geometry; existing strokes keep their original world-space positions.",
            "example": example_path,
            "guide_count": roundtrip.guide_resources.len(),
            "stroke_count": roundtrip.strokes.len(),
            "gpu": self.renderer.adapter,
            "physical_tablet_tested": false
        });
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
