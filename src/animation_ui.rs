use super::*;

#[derive(Default)]
pub(super) struct AnimationUi {
    playing: bool,
    playback_origin_seconds: f64,
    playback_origin_frame: u32,
}

impl SketchApp {
    fn animation_state(&self) -> crate::editing::ArtworkAnimationState {
        crate::editing::ArtworkAnimationState {
            animation: self.doc.animation.clone(),
            strokes: self.doc.strokes.clone(),
        }
    }

    fn commit_animation_state(
        &mut self,
        animation: crate::animation::ArtworkAnimation,
        strokes: Vec<Stroke>,
        status: &str,
    ) -> Result<(), String> {
        let layer_ids = self.doc.layers.iter().map(|layer| layer.id).collect();
        let tip_ids = self
            .doc
            .brush_tip_assets
            .iter()
            .map(|asset| asset.id)
            .collect();
        animation.validate(&layer_ids, &tip_ids)?;
        let before = self.animation_state();
        let after = crate::editing::ArtworkAnimationState { animation, strokes };
        if before.animation == after.animation && before.strokes == after.strokes {
            return Ok(());
        }
        let edit = Edit::ArtworkAnimation { before, after };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.renderer.invalidate();
        self.selected.clear();
        self.dirty = true;
        self.status = status.into();
        self.record_recovery_edit_labeled("Artwork animation");
        Ok(())
    }

    fn seek_artwork_animation(&mut self, frame: u32, playback: bool) {
        let current = &self.doc.animation;
        let frame = frame.clamp(current.range_start, current.range_end);
        if frame == current.current_frame {
            return;
        }
        if !playback && !current.current_pose_matches(&self.doc.strokes) {
            self.status = "Set or update the current keyframe before changing frames.".into();
            return;
        }
        let mut animation = current.clone();
        animation.current_frame = frame;
        let strokes = animation
            .resolve(frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        self.doc.animation = animation;
        self.doc.strokes = strokes;
        self.renderer.invalidate();
        self.selected.clear();
        if !playback {
            // The undo stack is frame-local. Keeping it across a seek could
            // apply an edit to a different topology or stroke identity.
            self.undo.clear();
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit_labeled("Animation frame seek");
            self.status = format!(
                "Frame {frame}. Frame-local undo history resets when seeking; set a keyframe to save edits."
            );
        }
    }

    pub(super) fn tick_artwork_animation(&mut self, ctx: &egui::Context) {
        if !self.animation_ui.playing {
            return;
        }
        if self.doc.animation.is_empty() {
            self.animation_ui.playing = false;
            return;
        }
        let now = ctx.input(|input| input.time);
        let elapsed = (now - self.animation_ui.playback_origin_seconds).max(0.0);
        let frame_offset = (elapsed * f64::from(self.doc.animation.fps)).floor() as u32;
        let frame_count = self.doc.animation.frame_count().max(1) as u32;
        let frame = self.doc.animation.range_start
            + (self
                .animation_ui
                .playback_origin_frame
                .saturating_sub(self.doc.animation.range_start)
                .saturating_add(frame_offset)
                % frame_count);
        self.seek_artwork_animation(frame, true);
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(
            1.0 / f32::from(self.doc.animation.fps.max(1)),
        ));
    }

    fn toggle_artwork_playback(&mut self, ctx: &egui::Context) {
        if self.animation_ui.playing {
            self.animation_ui.playing = false;
            self.dirty = true;
            self.record_recovery_edit_labeled("Stop artwork playback");
            self.status = format!("Stopped at frame {}.", self.doc.animation.current_frame);
            return;
        }
        if self.doc.animation.is_empty() {
            self.status = "Set at least one artwork keyframe before playback.".into();
            return;
        }
        if !self.doc.animation.current_pose_matches(&self.doc.strokes) {
            self.status = "Set or update the current keyframe before playback.".into();
            return;
        }
        self.animation_ui.playback_origin_seconds = ctx.input(|input| input.time);
        self.animation_ui.playback_origin_frame = self.doc.animation.current_frame;
        self.animation_ui.playing = true;
        self.undo.clear();
        self.redo.clear();
        self.status =
            "Playing artwork keyframes. Undo is frame-local and is cleared for playback.".into();
        ctx.request_repaint();
    }

    fn set_artwork_keyframe(&mut self) {
        if !self.doc.active_editable() {
            self.status = "Choose a visible, unlocked layer before keying its artwork.".into();
            return;
        }
        if !self.doc.linked_stroke_groups.definitions.is_empty()
            || !self.doc.linked_stroke_groups.instances.is_empty()
        {
            self.status = "Make linked instances unique before keyframing artwork. Linked groups are kept static.".into();
            return;
        }
        let layer_id = self.doc.active_layer;
        let layer_strokes = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| stroke.layer_id == layer_id)
            .cloned()
            .collect::<Vec<_>>();
        let next = match self.doc.animation.set_layer_keyframe(
            self.doc.animation.current_frame,
            layer_id,
            &layer_strokes,
            &self.doc.strokes,
        ) {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        match self.commit_animation_state(
            next,
            self.doc.strokes.clone(),
            "Active layer keyframe saved as one undoable edit.",
        ) {
            Ok(()) => {}
            Err(error) => self.status = error,
        }
    }

    fn duplicate_artwork_keyframe(&mut self) {
        if !self.doc.linked_stroke_groups.definitions.is_empty()
            || !self.doc.linked_stroke_groups.instances.is_empty()
        {
            self.status = "Make linked instances unique before keyframing artwork.".into();
            return;
        }
        let source_frame = self
            .doc
            .animation
            .layer_tracks
            .iter()
            .find(|track| track.layer_id == self.doc.active_layer)
            .and_then(|track| {
                track
                    .keyframes
                    .iter()
                    .rev()
                    .find(|key| {
                        key.frame <= self.doc.animation.current_frame
                            && self.doc.animation.current_frame - key.frame < key.exposure
                    })
                    .map(|key| key.frame)
            })
            .unwrap_or(self.doc.animation.current_frame);
        let target_frame = self.doc.animation.current_frame.saturating_add(1);
        let next = match self.doc.animation.duplicate_layer_keyframe(
            self.doc.active_layer,
            source_frame,
            target_frame,
        ) {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let strokes = next.resolve(target_frame).unwrap_or_default();
        if let Err(error) = self.commit_animation_state(
            next,
            strokes,
            "Duplicated the active layer exposure to the next frame.",
        ) {
            self.status = error;
        }
    }

    fn delete_artwork_keyframe(&mut self) {
        let frame = self.doc.animation.current_frame;
        let mut next = match self
            .doc
            .animation
            .delete_layer_keyframe(self.doc.active_layer, frame)
        {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let strokes = next
            .resolve(frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        next.current_frame = frame;
        if let Err(error) = self.commit_animation_state(
            next,
            strokes,
            "Deleted the active layer keyframe as one undoable edit.",
        ) {
            self.status = error;
        }
    }

    fn update_animation_settings(
        &mut self,
        mutate: impl FnOnce(&mut crate::animation::ArtworkAnimation),
    ) {
        let current_pose_is_clean = self.doc.animation.current_pose_matches(&self.doc.strokes);
        let mut next = self.doc.animation.clone();
        mutate(&mut next);
        next.current_frame = next.current_frame.clamp(next.range_start, next.range_end);
        let strokes = if current_pose_is_clean {
            next.resolve(next.current_frame)
                .unwrap_or_else(|| self.doc.strokes.clone())
        } else {
            self.doc.strokes.clone()
        };
        if let Err(error) =
            self.commit_animation_state(next, strokes, "Artwork animation settings changed.")
        {
            self.status = error;
        }
    }

    pub(super) fn artwork_animation_timeline(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("artwork-animation-timeline")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let active_layer_name = self.doc.layers.iter().find(|layer|layer.id==self.doc.active_layer).map(|layer|layer.name.as_str()).unwrap_or("Layer");
                    ui.label(format!("Layer: {active_layer_name}"));
                    let play_label = if self.animation_ui.playing { "Pause" } else { "Play" };
                    if ui
                        .add_enabled(!self.doc.animation.is_empty(), egui::Button::new(play_label))
                        .clicked()
                    {
                        self.toggle_artwork_playback(ctx);
                    }
                    if ui
                        .add_enabled(!self.animation_ui.playing, egui::Button::new("Key active layer"))
                        .clicked()
                    {
                        self.set_artwork_keyframe();
                    }
                    let active_track = self.doc.animation.layer_tracks.iter().find(|track| track.layer_id == self.doc.active_layer);
                    let held_key = active_track.and_then(|track|track.keyframes.iter().rev().find(|key|key.frame<=self.doc.animation.current_frame && self.doc.animation.current_frame-key.frame<key.exposure).map(|key|key.frame));
                    let has_current = active_track.is_some_and(|track| track.keyframes.iter().any(|key|key.frame==self.doc.animation.current_frame));
                    if ui
                        .add_enabled(
                            held_key.is_some() && !self.animation_ui.playing,
                            egui::Button::new("Duplicate exposure → next"),
                        )
                        .clicked()
                    {
                        self.duplicate_artwork_keyframe();
                    }
                    if ui
                        .add_enabled(
                            has_current && !self.animation_ui.playing,
                            egui::Button::new("Delete layer key"),
                        )
                        .clicked()
                    {
                        self.delete_artwork_keyframe();
                    }
                    ui.label(format!(
                        "{} layer key(s)",
                        self.doc.animation.layer_tracks.iter().map(|track|track.keyframes.len()).sum::<usize>()
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label("Frame");
                    let mut frame = self.doc.animation.current_frame;
                    let response = ui.add(
                        egui::Slider::new(
                            &mut frame,
                            self.doc.animation.range_start..=self.doc.animation.range_end,
                        )
                        .show_value(true),
                    );
                    if response.changed() && !self.animation_ui.playing {
                        self.seek_artwork_animation(frame, false);
                    }
                    ui.label(format!(
                        "Range {}–{}",
                        self.doc.animation.range_start, self.doc.animation.range_end
                    ));
                    if ui.small_button("Start −").clicked() {
                        self.update_animation_settings(|animation| {
                            animation.range_start = animation.range_start.saturating_sub(1);
                        });
                    }
                    if ui.small_button("Start +").clicked() {
                        self.update_animation_settings(|animation| {
                            let latest_start = animation
                                .keyframes.first().map(|keyframe|keyframe.frame)
                                .or_else(|| animation.layer_tracks.iter().flat_map(|track|track.keyframes.first()).map(|keyframe|keyframe.frame).min())
                                .unwrap_or(animation.range_end);
                            animation.range_start = animation
                                .range_start
                                .saturating_add(1)
                                .min(animation.range_end.min(latest_start));
                        });
                    }
                    if ui.small_button("End −").clicked() {
                        self.update_animation_settings(|animation| {
                            let earliest_end = animation
                                .keyframes.last().map(|keyframe|keyframe.frame)
                                .or_else(|| animation.layer_tracks.iter().flat_map(|track|track.keyframes.iter().map(|keyframe|keyframe.frame.saturating_add(keyframe.exposure).saturating_sub(1))).max())
                                .unwrap_or(animation.range_start);
                            animation.range_end = animation
                                .range_end
                                .saturating_sub(1)
                                .max(animation.range_start.max(earliest_end));
                        });
                    }
                    if ui.small_button("End +").clicked() {
                        self.update_animation_settings(|animation| {
                            animation.range_end =
                                animation.range_end.saturating_add(1).min(crate::animation::MAX_FRAME);
                        });
                    }
                    ui.label(format!("{} fps", self.doc.animation.fps));
                    if ui.small_button("− fps").clicked() {
                        self.update_animation_settings(|animation| {
                            animation.fps = animation
                                .fps
                                .saturating_sub(1)
                                .max(crate::animation::MIN_FPS);
                        });
                    }
                    if ui.small_button("+ fps").clicked() {
                        self.update_animation_settings(|animation| {
                            animation.fps = animation.fps.saturating_add(1).min(crate::animation::MAX_FPS);
                        });
                    }
                    let mut interpolate = self.doc.animation.interpolate;
                    if ui.add_enabled(self.doc.animation.layer_tracks.is_empty(), egui::Checkbox::new(&mut interpolate, "Interpolate compatible strokes")).changed() {
                        self.update_animation_settings(|animation| animation.interpolate = interpolate);
                    }
                    let mut onion = self.doc.animation.onion_skins;
                    if ui.checkbox(&mut onion, "Onion skin").changed() {
                        self.update_animation_settings(|animation| animation.onion_skins = onion);
                    }
                });
                if let Some((key_frame, key_exposure)) = self.doc.animation.layer_tracks.iter().find(|track|track.layer_id==self.doc.active_layer).and_then(|track|track.keyframes.iter().rev().find(|key|key.frame<=self.doc.animation.current_frame && self.doc.animation.current_frame-key.frame<key.exposure).map(|key|(key.frame,key.exposure))) {
                    ui.horizontal(|ui| {
                        ui.label("Active layer exposure");
                        let mut exposure=key_exposure;
                        let max=self.doc.animation.range_end.saturating_sub(key_frame).saturating_add(1);
                        if ui.add(egui::DragValue::new(&mut exposure).range(1..=max).suffix(" frames")).changed() {
                            match self.doc.animation.set_layer_exposure(self.doc.active_layer,key_frame,exposure) {
                                Ok(next)=>{let strokes=next.resolve(next.current_frame).unwrap_or_else(||self.doc.strokes.clone());if let Err(error)=self.commit_animation_state(next,strokes,"Layer exposure changed."){self.status=error;}},
                                Err(error)=>self.status=error,
                            }
                        }
                    });
                }
                ui.small("Edits belong to the displayed frame. Key the active layer before seeking; frame seeks reset frame-local undo. Exposures hold a layer snapshot for a chosen number of frames. Layer/resource structure stays static; layer moves are disabled while independent tracks exist. Linked-group documents must be made unique before keyframing. Onion skins are display-only.");
            });
    }

    pub(super) fn paint_animation_onion_skins(&self, painter: &egui::Painter, rect: Rect) {
        let animation = &self.doc.animation;
        if !animation.onion_skins || animation.is_empty() {
            return;
        }
        for (frame, color) in [
            (
                animation.current_frame.saturating_sub(1),
                Color32::from_rgba_unmultiplied(80, 180, 255, 105),
            ),
            (
                animation
                    .current_frame
                    .saturating_add(1)
                    .min(animation.range_end),
                Color32::from_rgba_unmultiplied(255, 140, 80, 105),
            ),
        ] {
            if frame == animation.current_frame {
                continue;
            }
            let Some(strokes) = animation.resolve(frame) else {
                continue;
            };
            for stroke in strokes.iter().filter(|stroke| self.doc.visible(stroke)) {
                let positions: Vec<_> = stroke
                    .points
                    .iter()
                    .filter_map(|point| {
                        crate::gizmo::project(&self.camera, rect, point.position)
                            .map(|(position, _)| position)
                    })
                    .collect();
                if positions.len() == 1 {
                    painter.circle_filled(positions[0], 2.0, color);
                }
                for segment in positions.windows(2) {
                    painter
                        .line_segment([segment[0], segment[1]], egui::Stroke::new(1.5_f32, color));
                }
            }
        }
    }
}
