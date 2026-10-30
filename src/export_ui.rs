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
impl SketchApp {
    pub(super) fn can_export(&self, selected: bool) -> bool {
        self.export_job.is_none()
            && self.active.is_none()
            && self.drag.is_none()
            && !self.confirm
            && self
                .doc
                .strokes
                .iter()
                .any(|s| !s.points.is_empty() && (!selected || self.selected.contains(&s.id)))
    }
    pub(super) fn export_dialog(&mut self, selected: bool, ctx: &egui::Context) {
        if !self.can_export(selected) {
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
        let filename = format!("{}{}.glb", name, if selected { "-selection" } else { "" });
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
        self.start_export(path, selected, ctx);
    }
    pub(super) fn start_export(&mut self, path: PathBuf, selected: bool, ctx: &egui::Context) {
        if !self.can_export(selected) {
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
        let snapshot = Document {
            version: self.doc.version,
            strokes: self
                .doc
                .strokes
                .iter()
                .filter(|s| selection.as_ref().is_none_or(|ids| ids.contains(&s.id)))
                .cloned()
                .collect(),
        };
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        let destination = path.clone();
        self.last_export_result = None;
        match std::thread::Builder::new()
            .name("glb-export".into())
            .spawn(move || {
                let result = export::write_glb(&snapshot, selection.as_ref(), &destination);
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
                "Exported {} strokes / {} triangles to {}",
                summary.strokes,
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
