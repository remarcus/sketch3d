use super::*;

pub(super) struct CameraBookmarkUi {
    pub open: bool,
    pub bookmark_name: String,
    pub sequence_name: String,
    pub frame_duration: f32,
    pub selected_frames: HashSet<u64>,
    pub rename_target: Option<u64>,
    pub rename_text: String,
    pub rename_sequence_target: Option<u64>,
    pub rename_sequence_text: String,
}

impl Default for CameraBookmarkUi {
    fn default() -> Self {
        Self {
            open: false,
            bookmark_name: String::new(),
            sequence_name: String::new(),
            frame_duration: 2.0,
            selected_frames: HashSet::new(),
            rename_target: None,
            rename_text: String::new(),
            rename_sequence_target: None,
            rename_sequence_text: String::new(),
        }
    }
}

pub(super) struct CameraPlayback {
    sequence_id: u64,
    started: Instant,
    original: Camera,
}

impl SketchApp {
    pub(super) fn camera_bookmark_controls(&mut self, ctx: &egui::Context) {
        if !self.camera_ui.open {
            return;
        }
        let mut open = self.camera_ui.open;
        let mut changed = false;
        let mut jump_to = None;
        let mut start_sequence = None;
        let mut stop_restore = false;
        let mut stop_here = false;
        let mut remove_bookmark = None;
        let mut remove_sequence = None;
        let mut move_bookmark = None;
        let mut move_sequence = None;
        let mut move_frame = None;
        egui::Window::new("Camera bookmarks and sequences")
            .open(&mut open)
            .default_width(460.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    "Camera views are stored in local workspace settings, separate from drawings.",
                );
                if let Some(error) = &self.camera_bookmark_error {
                    ui.colored_label(Color32::YELLOW, error);
                }
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.camera_ui.bookmark_name);
                    if ui
                        .add_enabled(
                            !self.camera_ui.bookmark_name.trim().is_empty(),
                            egui::Button::new("Save current view"),
                        )
                        .clicked()
                    {
                        let name = self.camera_ui.bookmark_name.trim().to_owned();
                        match self.camera_bookmarks.add_bookmark(name, &self.camera) {
                            Ok(_) => {
                                self.camera_ui.bookmark_name.clear();
                                changed = true;
                            }
                            Err(error) => self.status = error,
                        }
                    }
                });
                ui.separator();
                ui.strong("Bookmarks");
                if self.camera_bookmarks.bookmarks.is_empty() {
                    ui.label("No saved camera views.");
                }
                let ids: Vec<_> = self
                    .camera_bookmarks
                    .bookmarks
                    .iter()
                    .map(|item| item.id)
                    .collect();
                for (index, bookmark) in self.camera_bookmarks.bookmarks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(&bookmark.name);
                        if ui.button("Go").clicked() {
                            jump_to = Some(bookmark.id);
                        }
                        if ui.small_button("Rename").clicked() {
                            self.camera_ui.rename_target = Some(bookmark.id);
                            self.camera_ui.rename_text = bookmark.name.clone();
                        }
                        if ui.small_button("↑").clicked() && index > 0 {
                            move_bookmark = Some((index, index - 1));
                        }
                        if ui.small_button("↓").clicked() && index + 1 < ids.len() {
                            move_bookmark = Some((index, index + 1));
                        }
                        if ui.small_button("×").clicked() {
                            remove_bookmark = Some(bookmark.id);
                        }
                        let mut selected = self.camera_ui.selected_frames.contains(&bookmark.id);
                        if ui.checkbox(&mut selected, "Frame").changed() {
                            if selected {
                                self.camera_ui.selected_frames.insert(bookmark.id);
                            } else {
                                self.camera_ui.selected_frames.remove(&bookmark.id);
                            }
                        }
                    });
                }
                if let Some(id) = self.camera_ui.rename_target {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.camera_ui.rename_text);
                        if ui.button("Apply name").clicked() {
                            match self
                                .camera_bookmarks
                                .rename_bookmark(id, self.camera_ui.rename_text.trim().to_owned())
                            {
                                Ok(()) => {
                                    changed = true;
                                    self.camera_ui.rename_target = None;
                                }
                                Err(error) => self.status = error,
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.camera_ui.rename_target = None;
                        }
                    });
                }
                if let Some((from, to)) = move_bookmark {
                    let mut order = ids.clone();
                    let id = order.remove(from);
                    order.insert(to, id);
                    if self.camera_bookmarks.reorder_bookmarks(&order).is_ok() {
                        changed = true;
                    }
                }
                if let Some(id) = remove_bookmark {
                    if self.camera_bookmarks.remove_bookmark(id).is_ok() {
                        self.camera_ui.selected_frames.remove(&id);
                        changed = true;
                    }
                }
                ui.separator();
                ui.strong("Sequences");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.camera_ui.sequence_name);
                    ui.add(
                        egui::DragValue::new(&mut self.camera_ui.frame_duration)
                            .speed(0.1)
                            .range(
                                crate::camera_bookmarks::MIN_TRANSITION_SECONDS
                                    ..=crate::camera_bookmarks::MAX_TRANSITION_SECONDS,
                            )
                            .suffix(" sec / view"),
                    );
                });
                let selected: Vec<u64> = self
                    .camera_bookmarks
                    .bookmarks
                    .iter()
                    .filter(|item| self.camera_ui.selected_frames.contains(&item.id))
                    .map(|item| item.id)
                    .collect();
                if ui
                    .add_enabled(
                        selected.len() >= 2 && !self.camera_ui.sequence_name.trim().is_empty(),
                        egui::Button::new("Create sequence from checked views"),
                    )
                    .clicked()
                {
                    let mut frames: Vec<_> = selected
                        .iter()
                        .map(|id| crate::camera_bookmarks::SequenceFrame {
                            bookmark_id: *id,
                            transition_seconds: self.camera_ui.frame_duration,
                        })
                        .collect();
                    if let Some(last) = frames.last_mut() {
                        last.transition_seconds = 0.0;
                    }
                    match self
                        .camera_bookmarks
                        .add_sequence(self.camera_ui.sequence_name.trim().to_owned(), frames)
                    {
                        Ok(_) => {
                            self.camera_ui.sequence_name.clear();
                            changed = true;
                        }
                        Err(error) => self.status = error,
                    }
                }
                let sequence_ids: Vec<_> = self
                    .camera_bookmarks
                    .sequences
                    .iter()
                    .map(|sequence| sequence.id)
                    .collect();
                for (index, sequence) in self.camera_bookmarks.sequences.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} · {} views · {:.1}s",
                            sequence.name,
                            sequence.frames.len(),
                            sequence.duration_seconds()
                        ));
                        if ui.button("Play").clicked() {
                            start_sequence = Some(sequence.id);
                        }
                        if ui.small_button("Remove").clicked() {
                            remove_sequence = Some(sequence.id);
                        }
                        if ui.small_button("Rename").clicked() {
                            self.camera_ui.rename_sequence_target = Some(sequence.id);
                            self.camera_ui.rename_sequence_text = sequence.name.clone();
                        }
                        if ui.small_button("↑").clicked() && index > 0 {
                            move_sequence = Some((index, index - 1));
                        }
                        if ui.small_button("↓").clicked() && index + 1 < sequence_ids.len() {
                            move_sequence = Some((index, index + 1));
                        }
                    });
                    egui::CollapsingHeader::new("Frame order")
                        .id_salt(sequence.id)
                        .show(ui, |ui| {
                            for (frame_index, frame) in sequence.frames.iter().enumerate() {
                                let name = self
                                    .camera_bookmarks
                                    .bookmarks
                                    .iter()
                                    .find(|bookmark| bookmark.id == frame.bookmark_id)
                                    .map_or("Missing view", |bookmark| bookmark.name.as_str());
                                ui.horizontal(|ui| {
                                    ui.label(format!(
                                        "{} · {:.1}s",
                                        name, frame.transition_seconds
                                    ));
                                    if ui.small_button("↑").clicked() && frame_index > 0 {
                                        move_frame =
                                            Some((sequence.id, frame_index, frame_index - 1));
                                    }
                                    if ui.small_button("↓").clicked()
                                        && frame_index + 1 < sequence.frames.len()
                                    {
                                        move_frame =
                                            Some((sequence.id, frame_index, frame_index + 1));
                                    }
                                });
                            }
                        });
                }
                if let Some(id) = self.camera_ui.rename_sequence_target {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.camera_ui.rename_sequence_text);
                        if ui.button("Apply sequence name").clicked() {
                            match self.camera_bookmarks.rename_sequence(
                                id,
                                self.camera_ui.rename_sequence_text.trim().to_owned(),
                            ) {
                                Ok(()) => {
                                    self.camera_ui.rename_sequence_target = None;
                                    changed = true;
                                }
                                Err(error) => self.status = error,
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.camera_ui.rename_sequence_target = None;
                        }
                    });
                }
                if let Some((from, to)) = move_sequence {
                    let mut order = sequence_ids.clone();
                    let id = order.remove(from);
                    order.insert(to, id);
                    if self.camera_bookmarks.reorder_sequences(&order).is_ok() {
                        changed = true;
                    }
                }
                if let Some((sequence_id, from, to)) = move_frame {
                    if let Some(sequence) = self
                        .camera_bookmarks
                        .sequences
                        .iter()
                        .find(|sequence| sequence.id == sequence_id)
                    {
                        let mut order: Vec<_> = sequence
                            .frames
                            .iter()
                            .map(|frame| frame.bookmark_id)
                            .collect();
                        let id = order.remove(from);
                        order.insert(to, id);
                        if self
                            .camera_bookmarks
                            .reorder_sequence_frames(sequence_id, &order)
                            .is_ok()
                        {
                            changed = true;
                        }
                    }
                }
                if let Some(id) = remove_sequence {
                    if self.camera_bookmarks.remove_sequence(id).is_ok() {
                        changed = true;
                    }
                }
                if let Some(playback) = &self.camera_playback {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Sequence playing; drawing and navigation are paused.");
                        if ui.button("Stop and restore view").clicked() {
                            stop_restore = true;
                        }
                        if ui.button("Stop here").clicked() {
                            stop_here = true;
                        }
                    });
                    let _ = playback;
                }
            });
        self.camera_ui.open = open;
        if let Some(id) = jump_to {
            self.camera_playback = None;
            if let Some(bookmark) = self
                .camera_bookmarks
                .bookmarks
                .iter()
                .find(|item| item.id == id)
            {
                if let Err(error) = bookmark.view.apply(&mut self.camera) {
                    self.status = error;
                }
            }
        }
        if let Some(id) = start_sequence {
            self.start_camera_sequence(id);
        }
        if stop_restore {
            self.stop_camera_sequence(true);
        }
        if stop_here {
            self.stop_camera_sequence(false);
        }
        if changed {
            self.save_camera_bookmarks();
        }
    }

    fn save_camera_bookmarks(&mut self) {
        match self
            .camera_bookmark_path
            .as_ref()
            .ok_or_else(|| "Camera bookmark storage is unavailable".to_owned())
            .and_then(|path| self.camera_bookmarks.save(path))
        {
            Ok(()) => self.camera_bookmark_error = None,
            Err(error) => {
                self.camera_bookmark_error = Some(error.clone());
                self.status = format!("Camera bookmarks could not be saved: {error}");
            }
        }
    }

    fn start_camera_sequence(&mut self, id: u64) {
        self.finish();
        if self.camera_bookmarks.sample_sequence(id, 0.0).is_err() {
            self.status = "Camera sequence is invalid.".into();
            return;
        }
        self.camera_playback = Some(CameraPlayback {
            sequence_id: id,
            started: Instant::now(),
            original: self.camera,
        });
    }

    fn stop_camera_sequence(&mut self, restore: bool) {
        if let Some(playback) = self.camera_playback.take() {
            if restore {
                self.camera = playback.original;
            }
        }
    }

    pub(super) fn tick_camera_sequence(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(Key::Escape)) && self.camera_playback.is_some() {
            self.stop_camera_sequence(true);
            return;
        }
        let Some(playback) = &self.camera_playback else {
            return;
        };
        let Some(sequence) = self
            .camera_bookmarks
            .sequences
            .iter()
            .find(|s| s.id == playback.sequence_id)
        else {
            self.stop_camera_sequence(true);
            return;
        };
        let elapsed = playback.started.elapsed().as_secs_f32();
        let total = sequence.duration_seconds();
        match self
            .camera_bookmarks
            .sample_sequence(playback.sequence_id, elapsed)
        {
            Ok(view) => {
                let _ = view.apply(&mut self.camera);
            }
            Err(error) => {
                self.status = error;
                self.stop_camera_sequence(true);
                return;
            }
        }
        if elapsed >= total {
            self.camera_playback = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
