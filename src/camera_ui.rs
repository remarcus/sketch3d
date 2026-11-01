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

#[derive(Default)]
struct SequenceUiActions {
    changed: bool,
    start_sequence: Option<u64>,
    stop_restore: bool,
    stop_here: bool,
    status: Option<String>,
    #[cfg(test)]
    create_rect: Option<Rect>,
    #[cfg(test)]
    play_rects: Vec<(u64, Rect)>,
    #[cfg(test)]
    sequence_down_rect: Option<Rect>,
    #[cfg(test)]
    frame_order_rect: Option<Rect>,
    #[cfg(test)]
    frame_down_rect: Option<Rect>,
    #[cfg(test)]
    rename_rects: Vec<(u64, Rect)>,
    #[cfg(test)]
    apply_name_rect: Option<Rect>,
    #[cfg(test)]
    remove_rects: Vec<(u64, Rect)>,
    #[cfg(test)]
    stop_restore_rect: Option<Rect>,
    #[cfg(test)]
    stop_here_rect: Option<Rect>,
}

fn sequence_controls_ui(
    ui: &mut egui::Ui,
    camera_ui: &mut CameraBookmarkUi,
    bookmarks: &mut crate::camera_bookmarks::CameraBookmarkStore,
    playback_active: bool,
) -> SequenceUiActions {
    let mut actions = SequenceUiActions::default();
    let mut remove_sequence = None;
    let mut move_sequence = None;
    let mut move_frame = None;

    ui.separator();
    ui.strong("Sequences");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut camera_ui.sequence_name);
        ui.add(
            egui::DragValue::new(&mut camera_ui.frame_duration)
                .speed(0.1)
                .range(
                    crate::camera_bookmarks::MIN_TRANSITION_SECONDS
                        ..=crate::camera_bookmarks::MAX_TRANSITION_SECONDS,
                )
                .suffix(" sec / view"),
        );
    });
    let selected: Vec<u64> = bookmarks
        .bookmarks
        .iter()
        .filter(|item| camera_ui.selected_frames.contains(&item.id))
        .map(|item| item.id)
        .collect();
    let create = ui.add_enabled(
        selected.len() >= 2 && !camera_ui.sequence_name.trim().is_empty(),
        egui::Button::new("Create sequence from checked views"),
    );
    #[cfg(test)]
    {
        actions.create_rect = Some(create.rect);
    }
    if create.clicked() {
        let mut frames: Vec<_> = selected
            .iter()
            .map(|id| crate::camera_bookmarks::SequenceFrame {
                bookmark_id: *id,
                transition_seconds: camera_ui.frame_duration,
            })
            .collect();
        if let Some(last) = frames.last_mut() {
            last.transition_seconds = 0.0;
        }
        match bookmarks.add_sequence(camera_ui.sequence_name.trim().to_owned(), frames) {
            Ok(_) => {
                camera_ui.sequence_name.clear();
                actions.changed = true;
            }
            Err(error) => actions.status = Some(error),
        }
    }

    let sequence_ids: Vec<_> = bookmarks
        .sequences
        .iter()
        .map(|sequence| sequence.id)
        .collect();
    for (index, sequence) in bookmarks.sequences.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} · {} views · {:.1}s",
                sequence.name,
                sequence.frames.len(),
                sequence.duration_seconds()
            ));
            let play = ui.button("Play");
            #[cfg(test)]
            actions.play_rects.push((sequence.id, play.rect));
            if play.clicked() {
                actions.start_sequence = Some(sequence.id);
            }
            let remove = ui.small_button("Remove");
            #[cfg(test)]
            actions.remove_rects.push((sequence.id, remove.rect));
            if remove.clicked() {
                remove_sequence = Some(sequence.id);
            }
            let rename = ui.small_button("Rename");
            #[cfg(test)]
            actions.rename_rects.push((sequence.id, rename.rect));
            if rename.clicked() {
                camera_ui.rename_sequence_target = Some(sequence.id);
                camera_ui.rename_sequence_text = sequence.name.clone();
            }
            if ui_style::small_icon_button(
                ui,
                "↑",
                &format!("Move sequence {} up", sequence.name),
                index > 0,
            )
            .clicked()
            {
                move_sequence = Some((index, index - 1));
            }
            let down = ui_style::small_icon_button(
                ui,
                "↓",
                &format!("Move sequence {} down", sequence.name),
                index + 1 < sequence_ids.len(),
            );
            #[cfg(test)]
            if index == 0 && down.enabled() {
                actions.sequence_down_rect = Some(down.rect);
            }
            if down.clicked() && index + 1 < sequence_ids.len() {
                move_sequence = Some((index, index + 1));
            }
        });
        let _frame_order = egui::CollapsingHeader::new("Frame order")
            .id_salt(sequence.id)
            .show(ui, |ui| {
                for (frame_index, frame) in sequence.frames.iter().enumerate() {
                    let name = bookmarks
                        .bookmarks
                        .iter()
                        .find(|bookmark| bookmark.id == frame.bookmark_id)
                        .map_or("Missing view", |bookmark| bookmark.name.as_str());
                    ui.horizontal(|ui| {
                        ui.label(format!("{} · {:.1}s", name, frame.transition_seconds));
                        if ui_style::small_icon_button(
                            ui,
                            "↑",
                            &format!("Move {name} frame up"),
                            frame_index > 0,
                        )
                        .clicked()
                        {
                            move_frame = Some((sequence.id, frame_index, frame_index - 1));
                        }
                        let down = ui_style::small_icon_button(
                            ui,
                            "↓",
                            &format!("Move {name} frame down"),
                            frame_index + 1 < sequence.frames.len(),
                        );
                        #[cfg(test)]
                        if frame_index == 0 && down.enabled() {
                            actions.frame_down_rect = Some(down.rect);
                        }
                        if down.clicked() && frame_index + 1 < sequence.frames.len() {
                            move_frame = Some((sequence.id, frame_index, frame_index + 1));
                        }
                    });
                }
            });
        #[cfg(test)]
        if actions.frame_order_rect.is_none() {
            actions.frame_order_rect = Some(_frame_order.header_response.rect);
        }
    }
    if let Some(id) = camera_ui.rename_sequence_target {
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut camera_ui.rename_sequence_text);
            let apply = ui.button("Apply sequence name");
            #[cfg(test)]
            {
                actions.apply_name_rect = Some(apply.rect);
            }
            if apply.clicked() {
                match bookmarks
                    .rename_sequence(id, camera_ui.rename_sequence_text.trim().to_owned())
                {
                    Ok(()) => {
                        camera_ui.rename_sequence_target = None;
                        actions.changed = true;
                    }
                    Err(error) => actions.status = Some(error),
                }
            }
            if ui.button("Cancel").clicked() {
                camera_ui.rename_sequence_target = None;
            }
        });
    }
    if let Some((from, to)) = move_sequence {
        let mut order = sequence_ids;
        let id = order.remove(from);
        order.insert(to, id);
        if bookmarks.reorder_sequences(&order).is_ok() {
            actions.changed = true;
        }
    }
    if let Some((sequence_id, from, to)) = move_frame {
        if let Some(sequence) = bookmarks
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
            if bookmarks
                .reorder_sequence_frames(sequence_id, &order)
                .is_ok()
            {
                actions.changed = true;
            }
        }
    }
    if let Some(id) = remove_sequence {
        if bookmarks.remove_sequence(id).is_ok() {
            actions.changed = true;
        }
    }
    if playback_active {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Sequence playing; drawing and navigation are paused.");
            let stop_restore = ui.button("Stop and restore view");
            #[cfg(test)]
            {
                actions.stop_restore_rect = Some(stop_restore.rect);
            }
            actions.stop_restore = stop_restore.clicked();
            let stop_here = ui.button("Stop here");
            #[cfg(test)]
            {
                actions.stop_here_rect = Some(stop_here.rect);
            }
            actions.stop_here = stop_here.clicked();
        });
    }
    actions
}

impl CameraPlayback {
    fn advance(
        &self,
        store: &crate::camera_bookmarks::CameraBookmarkStore,
        elapsed_seconds: f32,
        camera: &mut Camera,
    ) -> Result<bool, String> {
        let sequence = store
            .sequences
            .iter()
            .find(|sequence| sequence.id == self.sequence_id)
            .ok_or_else(|| "Camera sequence was not found".to_owned())?;
        let finished = elapsed_seconds >= sequence.duration_seconds();
        store
            .sample_sequence(self.sequence_id, elapsed_seconds)?
            .apply(camera)?;
        Ok(finished)
    }

    fn stop(self, camera: &mut Camera, restore: bool) {
        if restore {
            *camera = self.original;
        }
    }
}

impl SketchApp {
    pub(super) fn camera_bookmark_controls(&mut self, ctx: &egui::Context) {
        if !self.camera_ui.open {
            return;
        }
        let mut open = self.camera_ui.open;
        let mut changed = false;
        let mut jump_to = None;
        let mut remove_bookmark = None;
        let mut move_bookmark = None;
        let mut sequence_actions = SequenceUiActions::default();
        egui::Window::new("Camera bookmarks and sequences")
            .open(&mut open)
            .default_width(460.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    "Camera views are stored in local workspace settings, separate from drawings.",
                );
                if let Some(error) = &self.camera_bookmark_error {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, error);
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
                        if ui_style::small_icon_button(
                            ui,
                            "↑",
                            &format!("Move bookmark {} up", bookmark.name),
                            index > 0,
                        )
                        .clicked()
                        {
                            move_bookmark = Some((index, index - 1));
                        }
                        if ui_style::small_icon_button(
                            ui,
                            "↓",
                            &format!("Move bookmark {} down", bookmark.name),
                            index + 1 < ids.len(),
                        )
                        .clicked()
                        {
                            move_bookmark = Some((index, index + 1));
                        }
                        if ui_style::small_icon_button(
                            ui,
                            "×",
                            &format!("Remove bookmark {}", bookmark.name),
                            true,
                        )
                        .clicked()
                        {
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
                sequence_actions = sequence_controls_ui(
                    ui,
                    &mut self.camera_ui,
                    &mut self.camera_bookmarks,
                    self.camera_playback.is_some(),
                );
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
        if let Some(status) = sequence_actions.status.take() {
            self.status = status;
        }
        if let Some(id) = sequence_actions.start_sequence {
            self.start_camera_sequence(id);
        }
        if sequence_actions.stop_restore {
            self.stop_camera_sequence(true);
        }
        if sequence_actions.stop_here {
            self.stop_camera_sequence(false);
        }
        if changed || sequence_actions.changed {
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
            playback.stop(&mut self.camera, restore);
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
        let elapsed = playback.started.elapsed().as_secs_f32();
        match playback.advance(&self.camera_bookmarks, elapsed, &mut self.camera) {
            Ok(true) => self.camera_playback = None,
            Ok(false) => ctx.request_repaint_after(Duration::from_millis(16)),
            Err(error) => {
                self.status = error;
                self.stop_camera_sequence(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{camera_bookmarks::SequenceFrame, geometry::Projection};
    use glam::Vec3;

    fn camera(yaw: f32, target: Vec3) -> Camera {
        Camera {
            yaw,
            pitch: 0.2,
            roll: 0.0,
            distance: 8.0,
            target,
            projection: Projection::Perspective,
        }
    }

    fn assert_camera_eq(actual: Camera, expected: Camera) {
        assert_eq!(actual.projection, expected.projection);
        assert_eq!(actual.yaw, expected.yaw);
        assert_eq!(actual.pitch, expected.pitch);
        assert_eq!(actual.roll, expected.roll);
        assert_eq!(actual.distance, expected.distance);
        assert_eq!(actual.target, expected.target);
    }

    fn sequence_store() -> (crate::camera_bookmarks::CameraBookmarkStore, u64) {
        let mut store = crate::camera_bookmarks::CameraBookmarkStore::default();
        let first = store
            .add_bookmark("First", &camera(0.0, Vec3::ZERO))
            .unwrap();
        let last = store.add_bookmark("Last", &camera(1.0, Vec3::X)).unwrap();
        let sequence = store
            .add_sequence(
                "Move",
                vec![
                    SequenceFrame {
                        bookmark_id: first,
                        transition_seconds: 2.0,
                    },
                    SequenceFrame {
                        bookmark_id: last,
                        transition_seconds: 0.0,
                    },
                ],
            )
            .unwrap();
        (store, sequence)
    }

    #[test]
    fn playback_advances_to_saved_views_and_stop_can_restore_or_keep_current() {
        let (store, sequence_id) = sequence_store();
        let original = camera(-0.5, Vec3::new(-2.0, 0.0, 0.0));
        let playback = CameraPlayback {
            sequence_id,
            started: Instant::now(),
            original,
        };
        let mut current = original;

        assert!(!playback.advance(&store, 1.0, &mut current).unwrap());
        assert!((current.yaw - 0.5).abs() < 1.0e-5);
        assert_eq!(current.target, Vec3::X * 0.5);

        playback.stop(&mut current, true);
        assert_camera_eq(current, original);

        let playback = CameraPlayback {
            sequence_id,
            started: Instant::now(),
            original,
        };
        assert!(playback.advance(&store, 2.0, &mut current).unwrap());
        assert_eq!(current.yaw, 1.0);
        assert_eq!(current.target, Vec3::X);
        playback.stop(&mut current, false);
        assert_eq!(
            current.yaw, 1.0,
            "Stop here keeps the current sequence view"
        );
        assert_eq!(current.target, Vec3::X);
    }

    fn render_sequence_ui(
        ctx: &egui::Context,
        ui: &mut CameraBookmarkUi,
        store: &mut crate::camera_bookmarks::CameraBookmarkStore,
        playback_active: bool,
        events: Vec<egui::Event>,
    ) -> SequenceUiActions {
        let mut actions = SequenceUiActions::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(860.0, 620.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui_widget| {
                    actions = sequence_controls_ui(ui_widget, ui, store, playback_active);
                });
            },
        );
        actions
    }

    fn click_sequence_ui(
        ctx: &egui::Context,
        ui: &mut CameraBookmarkUi,
        store: &mut crate::camera_bookmarks::CameraBookmarkStore,
        playback_active: bool,
        position: Pos2,
    ) -> SequenceUiActions {
        let _ = render_sequence_ui(
            ctx,
            ui,
            store,
            playback_active,
            vec![egui::Event::PointerMoved(position)],
        );
        let _ = render_sequence_ui(
            ctx,
            ui,
            store,
            playback_active,
            vec![egui::Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        render_sequence_ui(
            ctx,
            ui,
            store,
            playback_active,
            vec![egui::Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
    }

    #[test]
    fn sequence_controls_create_rename_reorder_start_stop_and_remove_by_pointer() {
        let mut store = crate::camera_bookmarks::CameraBookmarkStore::default();
        let first = store
            .add_bookmark("First", &camera(0.0, Vec3::ZERO))
            .unwrap();
        let last = store.add_bookmark("Last", &camera(1.0, Vec3::X)).unwrap();
        let mut ui = CameraBookmarkUi {
            sequence_name: "Saved tour".into(),
            selected_frames: HashSet::from([first, last]),
            ..Default::default()
        };
        let context = egui::Context::default();

        let initial = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        let created = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            initial
                .create_rect
                .expect("Create sequence button renders")
                .center(),
        );
        assert!(created.changed);
        assert_eq!(store.sequences.len(), 1);
        let sequence_id = store.sequences[0].id;
        assert_eq!(store.sequences[0].frames.len(), 2);
        assert_eq!(store.sequences[0].frames[1].transition_seconds, 0.0);

        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .frame_order_rect
                .expect("frame order expander renders")
                .center(),
        );
        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .frame_down_rect
                .expect("frame reorder button renders when expanded")
                .center(),
        );
        assert_eq!(store.sequences[0].frames[0].bookmark_id, last);
        assert_eq!(store.sequences[0].frames[1].bookmark_id, first);

        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .rename_rects
                .iter()
                .find(|(id, _)| *id == sequence_id)
                .expect("sequence rename button renders")
                .1
                .center(),
        );
        assert_eq!(ui.rename_sequence_target, Some(sequence_id));
        ui.rename_sequence_text = "Renamed tour".into();
        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        let renamed = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .apply_name_rect
                .expect("sequence name apply button renders")
                .center(),
        );
        assert!(renamed.changed);
        assert_eq!(store.sequences[0].name, "Renamed tour");

        let second = store.add_bookmark("Third", &camera(2.0, Vec3::Y)).unwrap();
        let frames = store.sequences[0].frames.clone();
        let second_sequence = store
            .add_sequence(
                "Second tour",
                vec![
                    frames[0],
                    crate::camera_bookmarks::SequenceFrame {
                        bookmark_id: second,
                        transition_seconds: 0.0,
                    },
                ],
            )
            .unwrap();
        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        let reordered = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .sequence_down_rect
                .expect("sequence reorder button renders")
                .center(),
        );
        assert!(reordered.changed);
        assert_eq!(store.sequences[0].id, second_sequence);
        assert_eq!(store.sequences[1].id, sequence_id);

        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        let play = rendered
            .play_rects
            .iter()
            .find(|(id, _)| *id == sequence_id)
            .expect("target sequence Play button renders")
            .1
            .center();
        let started = click_sequence_ui(&context, &mut ui, &mut store, false, play);
        assert_eq!(started.start_sequence, Some(sequence_id));

        let rendered = render_sequence_ui(&context, &mut ui, &mut store, true, Vec::new());
        let stopped = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            true,
            rendered
                .stop_restore_rect
                .expect("restore stop button renders during playback")
                .center(),
        );
        assert!(stopped.stop_restore);
        let rendered = render_sequence_ui(&context, &mut ui, &mut store, true, Vec::new());
        let stopped_here = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            true,
            rendered
                .stop_here_rect
                .expect("keep-view stop button renders during playback")
                .center(),
        );
        assert!(stopped_here.stop_here);

        let rendered = render_sequence_ui(&context, &mut ui, &mut store, false, Vec::new());
        let remove = click_sequence_ui(
            &context,
            &mut ui,
            &mut store,
            false,
            rendered
                .remove_rects
                .iter()
                .find(|(id, _)| *id == sequence_id)
                .expect("target sequence Remove button renders")
                .1
                .center(),
        );
        assert!(remove.changed);
        assert!(!store
            .sequences
            .iter()
            .any(|sequence| sequence.id == sequence_id));
    }
}
