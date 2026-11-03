use super::*;
use crate::export::{self, ExportSummary};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct ExportJob {
    receiver: Receiver<Result<ExportSummary, String>>,
    destination: PathBuf,
}
pub(super) struct ExportSnapshot {
    strokes: Vec<Stroke>,
    file: Option<PathBuf>,
    dirty: bool,
    undo: usize,
    redo: usize,
    selection: HashSet<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlbExportAction {
    InkOnly,
    WholeScene,
    SelectedStrokes,
}

impl GlbExportAction {
    pub(super) fn scope(self) -> (bool, bool) {
        match self {
            Self::InkOnly => (false, false),
            Self::WholeScene => (false, true),
            Self::SelectedStrokes => (true, false),
        }
    }
}

pub(super) struct GlbExportMenuResponse {
    pub action: Option<GlbExportAction>,
    #[cfg(test)]
    pub ink_only_rect: Rect,
    #[cfg(test)]
    pub whole_scene_rect: Rect,
    #[cfg(test)]
    pub selected_rect: Rect,
    #[cfg(test)]
    pub enabled_scopes: [bool; 3],
}

pub(super) fn glb_export_scope_items(
    ui: &mut egui::Ui,
    can_export_ink: bool,
    can_export_scene: bool,
    can_export_selected: bool,
) -> GlbExportMenuResponse {
    let ink_only = ui.add_enabled(
        can_export_ink,
        egui::Button::new("Whole drawing · ink only (.glb)"),
    );
    let whole_scene = ui.add_enabled(
        can_export_scene,
        egui::Button::new("Whole scene · ink + images + OBJ models (.glb)"),
    );
    let selected = ui.add_enabled(
        can_export_selected,
        egui::Button::new("Selected strokes (.glb)"),
    );
    let action = if ink_only.clicked() {
        Some(GlbExportAction::InkOnly)
    } else if whole_scene.clicked() {
        Some(GlbExportAction::WholeScene)
    } else if selected.clicked() {
        Some(GlbExportAction::SelectedStrokes)
    } else {
        None
    };
    GlbExportMenuResponse {
        action,
        #[cfg(test)]
        ink_only_rect: ink_only.rect,
        #[cfg(test)]
        whole_scene_rect: whole_scene.rect,
        #[cfg(test)]
        selected_rect: selected.rect,
        #[cfg(test)]
        enabled_scopes: [can_export_ink, can_export_scene, can_export_selected],
    }
}

pub(super) fn build_export_document_snapshot(
    document: &Document,
    selection: Option<&HashSet<u64>>,
    include_scene_resources: bool,
) -> Document {
    let include_scene_resources = include_scene_resources && selection.is_none();
    Document {
        linked_stroke_groups: Default::default(),
        layers: document.layers.clone(),
        active_layer: document.active_layer,
        version: document.version,
        image_planes: if include_scene_resources {
            document.image_planes.clone()
        } else {
            Vec::new()
        },
        active_image_plane: None,
        guide_resources: Vec::new(),
        active_guide: None,
        model_references: if include_scene_resources {
            document.model_references.clone()
        } else {
            Vec::new()
        },
        active_model_reference: None,
        brush_tip_assets: Vec::new(),
        animation: Default::default(),
        strokes: document
            .strokes
            .iter()
            .filter(|stroke| selection.is_none_or(|ids| ids.contains(&stroke.id)))
            .cloned()
            .collect(),
    }
}

impl SketchApp {
    pub(super) fn can_export(&self, selected: bool) -> bool {
        self.can_export_scope(selected, false)
    }
    pub(super) fn can_export_scene(&self) -> bool {
        self.can_export_scope(false, true)
    }
    pub(super) fn can_export_scope(&self, selected: bool, include_images: bool) -> bool {
        !self.liquify_active()
            && self.export_job.is_none()
            && self.active.is_none()
            && self.drag.is_none()
            && !self.confirm
            && (self
                .doc
                .strokes
                .iter()
                .any(|s| !s.points.is_empty() && (!selected || self.selected.contains(&s.id)))
                || (include_images
                    && !selected
                    && (self.doc.image_planes.iter().any(|plane| plane.visible)
                        || self.doc.model_references.iter().any(|model| model.visible))))
    }
    pub(super) fn export_dialog_with_images(
        &mut self,
        selected: bool,
        include_images: bool,
        ctx: &egui::Context,
    ) {
        if self.fills.draft.is_some() || self.line.draft.is_some() {
            self.status = "Close or cancel the draft contour before exporting.".into();
            return;
        }
        if !self.can_export_scope(selected, include_images) {
            self.status = "Finish editing and select drawable strokes before exporting.".into();
            return;
        }
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|p| p.to_str())
            .map(|s| s.trim_end_matches(".sketch.json").trim_end_matches(".json"))
            .unwrap_or(if self.starter_scene {
                "japanese-house"
            } else if self.cylinder_example {
                "lantern"
            } else {
                "drawing"
            });
        let suffix = if selected {
            "-selection"
        } else if include_images {
            "-scene"
        } else {
            ""
        };
        let filename = format!("{name}{suffix}.glb");
        let Some(path) = rfd::FileDialog::new()
            .add_filter("glTF Binary", &["glb"])
            .set_file_name(filename)
            .save_file()
        else {
            return;
        };
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("glb"))
        {
            self.status = "Choose an export filename ending in .glb.".into();
            return;
        }
        self.start_export_with_images(path, selected, include_images, ctx);
    }
    pub(super) fn start_export(&mut self, path: PathBuf, selected: bool, ctx: &egui::Context) {
        self.start_export_with_images(path, selected, false, ctx);
    }
    pub(super) fn start_export_with_images(
        &mut self,
        path: PathBuf,
        selected: bool,
        include_images: bool,
        ctx: &egui::Context,
    ) {
        if !self.can_export_scope(selected, include_images) {
            self.status = "No drawable strokes available, or another export is running.".into();
            return;
        }
        if self
            .file
            .as_ref()
            .is_some_and(|source| export::same_destination(source, &path))
        {
            let error =
                "Choose an export destination different from the open native drawing.".to_string();
            self.status = error.clone();
            self.last_export_result = Some(Err(error));
            return;
        }
        let selection = selected.then(|| self.selected.clone());
        let snapshot =
            build_export_document_snapshot(&self.doc, selection.as_ref(), include_images);
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        let destination = path.clone();
        let image_root = self.image_asset_root.clone();
        self.last_export_result = None;
        match std::thread::Builder::new()
            .name("glb-export".into())
            .spawn(move || {
                let result = if include_images {
                    export::write_glb_with_images(
                        &snapshot,
                        selection.as_ref(),
                        &image_root,
                        &destination,
                    )
                } else {
                    export::write_glb(&snapshot, selection.as_ref(), &destination)
                };
                let _ = sender.send(result);
                context.request_repaint();
            }) {
            Ok(_) => {
                self.export_job = Some(ExportJob {
                    receiver,
                    destination: path,
                });
                self.status = "Exporting GLB snapshot… You can keep drawing.".into();
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(error) => {
                self.status = format!("Could not start export: {error}");
                self.last_export_result = Some(Err(error.to_string()));
            }
        }
    }
    pub(super) fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = self.export_job.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                Err("Export worker stopped before completion".into())
            }
        };
        let destination = job.destination.clone();
        self.export_job = None;
        self.status = match &result {
            Ok(summary) => format!(
                "Exported {} strokes, {} image planes, {} OBJ models / {} triangles to {}",
                summary.strokes,
                summary.image_planes,
                summary.model_references,
                summary.triangles,
                destination.display()
            ),
            Err(error) => format!("Export failed: {error}"),
        };
        self.last_export_result = Some(result);
    }
    pub(super) fn verify_export_start(&mut self, ctx: &egui::Context) {
        self.export_snapshot = Some(ExportSnapshot {
            strokes: self.doc.strokes.clone(),
            file: self.file.clone(),
            dirty: self.dirty,
            undo: self.undo.len(),
            redo: self.redo.len(),
            selection: self.selected.clone(),
        });
        if let Some(source) = self.file.clone() {
            let original = std::fs::read(&source).unwrap();
            self.start_export(source.clone(), false, ctx);
            assert!(
                self.export_job.is_none(),
                "Export must not overwrite the active native drawing"
            );
            assert!(self.last_export_result.as_ref().unwrap().is_err());
            assert_eq!(std::fs::read(source).unwrap(), original);
        }
        let directory = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&directory).unwrap();
        self.start_export(directory.join("ui-export.glb"), false, ctx);
        assert!(self.export_job.is_some());
    }
    pub(super) fn verify_export_done(&mut self) -> bool {
        if self.export_job.is_some() {
            return false;
        }
        let Some(snapshot) = self.export_snapshot.take() else {
            return false;
        };
        assert!(self.last_export_result.as_ref().unwrap().is_ok());
        assert_eq!(snapshot.strokes, self.doc.strokes);
        assert_eq!(snapshot.file, self.file);
        assert_eq!(snapshot.dirty, self.dirty);
        assert_eq!(snapshot.undo, self.undo.len());
        assert_eq!(snapshot.redo, self.redo.len());
        assert_eq!(snapshot.selection, self.selected);
        let bytes = std::fs::read(
            std::env::current_dir()
                .unwrap()
                .join("verification/ui-export.glb"),
        )
        .unwrap();
        assert_eq!(&bytes[..4], b"glTF");
        self.status="Background GLB export passed; native drawing, file identity, selection, and history are unchanged.".into();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{model_references::ObjReferenceResource, StrokeMaterial},
        image_planes::ImagePlane,
        obj_reference_storage::ObjReferenceAsset,
    };
    use glam::{Quat, Vec3};

    fn stroke(id: u64) -> Stroke {
        Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            point_thickness: None,
            modifiers: None,
            fill: None,
            material: StrokeMaterial::default(),
            group_id: None,
            layer_id: 1,
            id,
            color: [0.2, 0.4, 0.6],
            radius: 0.02,
            brush: Default::default(),
            points: vec![crate::document::Point {
                position: Vec3::new(id as f32, 0.0, 0.0),
                pressure: 1.0,
                tilt: None,

                time_ms: 0,
            }],
        }
    }

    fn document_with_scene_resources() -> Document {
        Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![stroke(1), stroke(2)],
            image_planes: vec![ImagePlane::from_asset(
                7,
                "Backdrop",
                "image-7.png".into(),
                [4, 2],
                Vec3::ZERO,
                Quat::IDENTITY,
                2.0,
            )
            .unwrap()],
            model_references: vec![ObjReferenceResource {
                id: 9,
                name: "Model".into(),
                visible: true,
                locked: false,
                center: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: 1.0,
                asset: ObjReferenceAsset {
                    id: 9,
                    bundle_dir: "models/9".into(),
                },
            }],
            ..Document::default()
        }
    }

    #[test]
    fn export_snapshot_separates_whole_scene_ink_only_and_selected_scopes() {
        let document = document_with_scene_resources();

        let whole_scene = build_export_document_snapshot(&document, None, true);
        assert_eq!(whole_scene.strokes, document.strokes);
        assert_eq!(whole_scene.image_planes, document.image_planes);
        assert_eq!(whole_scene.model_references, document.model_references);
        assert!(whole_scene.guide_resources.is_empty());
        assert_eq!(whole_scene.active_image_plane, None);
        assert_eq!(whole_scene.active_model_reference, None);

        let ink_only = build_export_document_snapshot(&document, None, false);
        assert_eq!(ink_only.strokes, document.strokes);
        assert!(ink_only.image_planes.is_empty());
        assert!(ink_only.model_references.is_empty());

        let selection = HashSet::from([2]);
        let selected = build_export_document_snapshot(&document, Some(&selection), true);
        assert_eq!(selected.strokes, vec![document.strokes[1].clone()]);
        assert!(selected.image_planes.is_empty());
        assert!(selected.model_references.is_empty());
    }

    #[test]
    fn export_scope_clicks_dispatch_each_glb_action() {
        struct ScopeFrame {
            items: GlbExportMenuResponse,
        }

        fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> ScopeFrame {
            let mut items = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        items = Some(glb_export_scope_items(ui, true, true, true));
                    });
                },
            );
            ScopeFrame {
                items: items.expect("scope controls render"),
            }
        }

        fn click(ctx: &egui::Context, position: Pos2) -> ScopeFrame {
            let _ = frame(ctx, vec![egui::Event::PointerMoved(position)]);
            let _ = frame(
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            frame(
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            )
        }

        fn ink_rect(items: &GlbExportMenuResponse) -> Rect {
            items.ink_only_rect
        }
        fn scene_rect(items: &GlbExportMenuResponse) -> Rect {
            items.whole_scene_rect
        }
        fn selected_rect(items: &GlbExportMenuResponse) -> Rect {
            items.selected_rect
        }

        for (expected, expected_scope, rect_for_action) in [
            (
                GlbExportAction::InkOnly,
                (false, false),
                ink_rect as fn(&GlbExportMenuResponse) -> Rect,
            ),
            (GlbExportAction::WholeScene, (false, true), scene_rect),
            (
                GlbExportAction::SelectedStrokes,
                (true, false),
                selected_rect,
            ),
        ] {
            let ctx = egui::Context::default();
            let initial = frame(&ctx, Vec::new());
            let clicked = click(&ctx, rect_for_action(&initial.items).center());
            let action = clicked
                .items
                .action
                .expect("GLB scope item should dispatch an action");
            assert_eq!(action, expected);
            assert_eq!(action.scope(), expected_scope);
        }
    }

    #[test]
    fn scene_only_document_enables_scene_export_without_selectable_ink() {
        let ctx = egui::Context::default();
        let mut response = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(glb_export_scope_items(ui, false, true, false));
                });
            },
        );
        assert_eq!(response.unwrap().enabled_scopes, [false, true, false]);
    }
}
