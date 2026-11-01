use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

struct ObjExportJob {
    receiver: Receiver<Result<usize, String>>,
    destination: PathBuf,
}

#[derive(Default)]
pub(super) struct ObjExportUi {
    request: Option<(PathBuf, bool, bool)>,
    overwrite: Option<(PathBuf, bool)>,
    job: Option<ObjExportJob>,
}

impl ObjExportUi {
    pub fn begin(&mut self, selected: bool) {
        if self.job.is_some() || self.request.is_some() || self.overwrite.is_some() {
            return;
        }
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter("Wavefront OBJ", &["obj"])
            .set_file_name("spatial-sketch.obj")
            .save_file()
        else {
            return;
        };
        path.set_extension("obj");
        if path.exists() {
            self.overwrite = Some((path, selected));
        } else {
            self.request = Some((path, selected, false));
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if let Some((path, selected)) = self.overwrite.clone() {
            let mut replace = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("obj-export-overwrite")).show(ctx, |ui| {
                ui.heading("Replace existing OBJ?");
                ui.label(path.file_name().and_then(|name| name.to_str()).unwrap_or("OBJ file"));
                ui.small("The new export uses unique companion files; older companions are left untouched.");
                ui.horizontal(|ui| {
                    replace = ui.button("Overwrite").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if replace {
                self.overwrite = None;
                self.request = Some((path, selected, true));
            } else if cancel {
                self.overwrite = None;
            }
        }
        if let Some(job) = &self.job {
            egui::Window::new("Exporting OBJ")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.spinner();
                    ui.label(format!("Writing {}", job.destination.display()));
                });
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn take_request(&mut self) -> Option<(PathBuf, bool, bool)> {
        self.request.take()
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn start(
        &mut self,
        destination: PathBuf,
        selected: bool,
        replace: bool,
        document: &Document,
        selection: &HashSet<u64>,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err("An OBJ export is already running".into());
        }
        if destination.exists() && !replace {
            return Err(
                "Destination appeared after overwrite confirmation; choose it again to confirm."
                    .into(),
            );
        }
        if selected && selection.is_empty() {
            return Err("Select at least one stroke before exporting a selection".into());
        }
        let document = document.clone();
        let selection = selected.then(|| selection.clone());
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        let output_path = destination.clone();
        std::thread::Builder::new()
            .name("obj-export".into())
            .spawn(move || {
                let result = crate::obj_export::write_obj_bundle(
                    &document,
                    selection.as_ref(),
                    &output_path,
                    replace,
                );
                let _ = sender.send(result);
                context.request_repaint();
            })
            .map_err(|error| error.to_string())?;
        self.job = Some(ObjExportJob {
            receiver,
            destination,
        });
        Ok(())
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Option<String> {
        let job = self.job.as_ref()?;
        match job.receiver.try_recv() {
            Ok(result) => {
                let path = job.destination.clone();
                self.job = None;
                Some(match result {
                    Ok(bytes) => format!(
                        "Exported OBJ and companions ({} bytes) to {}",
                        bytes,
                        path.display()
                    ),
                    Err(error) => format!("OBJ export failed: {error}"),
                })
            }
            Err(TryRecvError::Disconnected) => {
                let path = job.destination.clone();
                self.job = None;
                Some(format!("OBJ export worker stopped for {}", path.display()))
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                None
            }
        }
    }
}
