use super::*;
use std::collections::BTreeSet;

pub(super) struct AnimationUi {
    playing: bool,
    playback_origin_seconds: f64,
    playback_origin_frame: u32,
    selected_key_frames: BTreeSet<u32>,
    multi_key_translation: [f32; 3],
    multi_key_point_scale: f32,
    multi_key_radius_scale: f32,
    multi_key_tint: [f32; 3],
    preview_animation: Option<crate::animation::ArtworkAnimation>,
    preview_source: Option<(crate::animation::ArtworkAnimation, Vec<Stroke>)>,
    retime_preview: Option<(u64, u32, u32)>,
    retime_entry: Option<(u64, u32, u32)>,
    rig_pose_frame: Option<u32>,
    rig_root_degrees: f32,
    rig_child_degrees: f32,
}

impl Default for AnimationUi {
    fn default() -> Self {
        Self {
            playing: false,
            playback_origin_seconds: 0.0,
            playback_origin_frame: 0,
            selected_key_frames: BTreeSet::new(),
            multi_key_translation: [0.0; 3],
            multi_key_point_scale: 1.0,
            multi_key_radius_scale: 1.0,
            multi_key_tint: [1.0; 3],
            preview_animation: None,
            preview_source: None,
            retime_preview: None,
            retime_entry: None,
            rig_pose_frame: None,
            rig_root_degrees: 0.0,
            rig_child_degrees: 0.0,
        }
    }
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

    fn set_artwork_keyframe(&mut self, blank: bool) {
        if self
            .doc
            .strokes
            .iter()
            .any(|stroke| stroke.guide_attachment.is_some())
        {
            self.status = "Detach ink from saved guides before keyframing.".into();
            return;
        }
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
            .filter(|stroke| !blank && stroke.layer_id == layer_id)
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
        let strokes = if blank {
            next.resolve(next.current_frame).unwrap_or_default()
        } else {
            self.doc.strokes.clone()
        };
        match self.commit_animation_state(
            next,
            strokes,
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

    fn create_two_bone_prototype(&mut self) {
        if self.animation_ui.playing || !self.doc.active_editable() {
            self.status = "Pause playback and choose a visible, unlocked layer before creating the prototype.".into();
            return;
        }
        let layer_strokes = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| stroke.layer_id == self.doc.active_layer)
            .cloned()
            .collect::<Vec<_>>();
        let next = match self.doc.animation.create_two_bone_rig(
            self.doc.animation.current_frame,
            self.doc.active_layer,
            &layer_strokes,
            &self.doc.strokes,
        ) {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let strokes = next
            .resolve(next.current_frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        if let Err(error) = self.commit_animation_state(
            next,
            strokes,
            "Created a two-bone cutout prototype for the active layer.",
        ) {
            self.status = error;
        } else {
            self.animation_ui.rig_pose_frame = None;
        }
    }

    fn key_two_bone_pose(&mut self) {
        if self.animation_ui.playing {
            self.status = "Pause playback before keying a rig pose.".into();
            return;
        }
        let frame = self.doc.animation.current_frame;
        let next = match self.doc.animation.set_two_bone_pose(
            frame,
            self.animation_ui.rig_root_degrees.to_radians(),
            self.animation_ui.rig_child_degrees.to_radians(),
        ) {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let strokes = next
            .resolve(frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        if let Err(error) = self.commit_animation_state(
            next,
            strokes,
            "Keyed the two-bone pose as one undoable animation edit.",
        ) {
            self.status = error;
        }
    }

    fn remove_two_bone_prototype(&mut self) {
        let next = match self.doc.animation.remove_two_bone_rig() {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let strokes = next
            .resolve(next.current_frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        if let Err(error) = self.commit_animation_state(
            next,
            strokes,
            "Removed the two-bone prototype and restored its source strokes.",
        ) {
            self.status = error;
        }
        self.animation_ui.rig_pose_frame = None;
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

    fn build_animation_keys_edit(&mut self) -> Option<crate::animation::ArtworkAnimation> {
        if self.animation_ui.playing {
            self.status = "Pause playback before editing multiple keys.".into();
            return None;
        }
        if !self.doc.active_editable() {
            self.status = "Choose a visible, unlocked layer before editing animation keys.".into();
            return None;
        }
        if !self.doc.linked_stroke_groups.definitions.is_empty()
            || !self.doc.linked_stroke_groups.instances.is_empty()
        {
            self.status = "Make linked instances unique before editing animation keys.".into();
            return None;
        }
        let frames = self
            .animation_ui
            .selected_key_frames
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let translation = glam::Vec3::from_array(self.animation_ui.multi_key_translation);
        let next = match self.doc.animation.edit_layer_keys(
            self.doc.active_layer,
            &frames,
            translation,
            self.animation_ui.multi_key_point_scale,
            self.animation_ui.multi_key_radius_scale,
            self.animation_ui.multi_key_tint,
        ) {
            Ok(animation) => animation,
            Err(error) => {
                self.status = error;
                return None;
            }
        };
        Some(next)
    }

    fn preview_animation_keys_edit(&mut self) {
        self.animation_ui.preview_animation = self.build_animation_keys_edit();
        if self.animation_ui.preview_animation.is_some() {
            self.animation_ui.preview_source =
                Some((self.doc.animation.clone(), self.doc.strokes.clone()));
            self.status = "Previewing the selected key edit. Inspect each selected key, then Apply or Cancel.".into();
        }
    }

    fn apply_animation_keys_edit(&mut self) {
        let Some(animation) = self.animation_ui.preview_animation.take() else {
            self.status = "Preview the multi-key edit before applying it.".into();
            return;
        };
        let Some((source_animation, source_strokes)) = self.animation_ui.preview_source.take()
        else {
            self.status =
                "The multi-key edit preview expired. Preview it again before applying.".into();
            return;
        };
        if self.doc.animation != source_animation || self.doc.strokes != source_strokes {
            self.status =
                "The drawing changed after the preview. Preview the multi-key edit again.".into();
            return;
        }
        let strokes = animation
            .resolve(animation.current_frame)
            .unwrap_or_else(|| self.doc.strokes.clone());
        if let Err(error) = self.commit_animation_state(
            animation,
            strokes,
            "Applied one point and appearance edit across the selected animation keys.",
        ) {
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
                        self.set_artwork_keyframe(false);
                    }
                    if ui.add_enabled(!self.animation_ui.playing && self.doc.active_editable(), egui::Button::new("Blank key")).clicked() {
                        self.set_artwork_keyframe(true);
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
                self.layer_key_strip(ui);
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
                    if let Some(layer) = self.doc.layers.iter().find(|layer| layer.id == self.doc.active_layer) {
                        let mut show_layer = !layer.hide_onion_skin;
                        if ui.add_enabled(onion && !self.animation_ui.playing, egui::Checkbox::new(&mut show_layer, "Onion skin for active layer")).changed() {
                            self.apply_layer_action(crate::editing::LayerAction::HideOnionSkin(layer.id, !show_layer));
                        }
                    }
                });
                if let Some((key_frame, key_exposure)) = self.doc.animation.layer_tracks.iter().find(|track|track.layer_id==self.doc.active_layer).and_then(|track|track.keyframes.iter().rev().find(|key|key.frame<=self.doc.animation.current_frame).filter(|key|self.doc.animation.current_frame-key.frame<key.exposure).map(|key|(key.frame,key.exposure))) {
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
                        self.layer_key_frame_entry(ui, key_frame);
                    });
                } else {
                    self.animation_ui.retime_entry = None;
                }
                let rig_pose = self.doc.animation.two_bone_rig.as_ref()
                    .filter(|rig| rig.layer_id == self.doc.active_layer)
                    .map(|rig| rig.pose_at(self.doc.animation.current_frame));
                if let Some(pose) = rig_pose {
                    if self.animation_ui.rig_pose_frame != Some(self.doc.animation.current_frame) {
                        self.animation_ui.rig_pose_frame = Some(self.doc.animation.current_frame);
                        self.animation_ui.rig_root_degrees = pose.root_angle.to_degrees();
                        self.animation_ui.rig_child_degrees = pose.child_angle.to_degrees();
                    }
                    ui.horizontal(|ui| {
                        ui.label("Two-bone pose");
                        ui.label("Root °");
                        ui.add(egui::DragValue::new(&mut self.animation_ui.rig_root_degrees).speed(0.5).range(-3600.0..=3600.0));
                        ui.label("Child °");
                        ui.add(egui::DragValue::new(&mut self.animation_ui.rig_child_degrees).speed(0.5).range(-3600.0..=3600.0));
                        if ui.add_enabled(!self.animation_ui.playing, egui::Button::new("Key pose")).clicked() {
                            self.key_two_bone_pose();
                        }
                        if ui.add_enabled(!self.animation_ui.playing, egui::Button::new("Remove prototype")).clicked() {
                            self.remove_two_bone_prototype();
                        }
                    });
                    ui.small("Root rotates the active layer; Child bends the distal 58% of each bound stroke with a smooth per-point weight. Pose keys interpolate between frames. The rigged layer is protected from direct edits; remove the prototype to restore its source strokes. Blender handoff includes editable centerlines, weights and armature pose keys.");
                } else if self.doc.animation.two_bone_rig.is_none() {
                    let has_strokes = self.doc.strokes.iter().any(|stroke| stroke.layer_id == self.doc.active_layer);
                    if ui.add_enabled(has_strokes && !self.animation_ui.playing, egui::Button::new("Create two-bone cutout prototype")).clicked() {
                        self.create_two_bone_prototype();
                    }
                }
                let key_rows = self
                    .doc
                    .animation
                    .layer_tracks
                    .iter()
                    .find(|track| track.layer_id == self.doc.active_layer)
                    .map(|track| {
                        track
                            .keyframes
                            .iter()
                            .map(|key| (key.frame, key.exposure, key.strokes.len()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if key_rows.len() > 1 {
                    ui.collapsing("Edit strokes across selected keys", |ui| {
                        ui.small("Keys match strokes by stable ID. Only keys containing the same stroke IDs are edited; held exposures follow their source key. Protected layers and linked artwork are excluded.");
                        ui.horizontal_wrapped(|ui| {
                            for (frame, exposure, count) in &key_rows {
                                let mut selected = self.animation_ui.selected_key_frames.contains(frame);
                                if ui.checkbox(&mut selected, format!("{frame} ({exposure}f, {count})")).changed() {
                                    self.animation_ui.preview_animation = None;
                                    if selected {
                                        self.animation_ui.selected_key_frames.insert(*frame);
                                    } else {
                                        self.animation_ui.selected_key_frames.remove(frame);
                                    }
                                }
                            }
                            if ui.small_button("Select all").clicked() {
                                self.animation_ui.selected_key_frames = key_rows.iter().map(|row| row.0).collect();
                            }
                            if ui.small_button("Clear").clicked() {
                                self.animation_ui.selected_key_frames.clear();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Move XYZ");
                            for component in &mut self.animation_ui.multi_key_translation {
                                if ui.add(egui::DragValue::new(component).speed(0.01).range(-10_000.0..=10_000.0)).changed() { self.animation_ui.preview_animation = None; }
                            }
                            ui.label("Point scale");
                            if ui.add(egui::DragValue::new(&mut self.animation_ui.multi_key_point_scale).speed(0.01).range(0.01..=100.0)).changed() { self.animation_ui.preview_animation = None; }
                            ui.label("Brush size");
                            if ui.add(egui::DragValue::new(&mut self.animation_ui.multi_key_radius_scale).speed(0.01).range(0.01..=10.0)).changed() { self.animation_ui.preview_animation = None; }
                        });
                        ui.horizontal(|ui| {
                            ui.label("RGB tint");
                            for component in &mut self.animation_ui.multi_key_tint {
                                if ui.add(egui::DragValue::new(component).speed(0.01).range(0.0..=2.0)).changed() { self.animation_ui.preview_animation = None; }
                            }
                            let count = self.animation_ui.selected_key_frames.len();
                            if ui.add_enabled((2..=16).contains(&count) && !self.animation_ui.playing, egui::Button::new("Preview edit")).clicked() {
                                self.preview_animation_keys_edit();
                            }
                            if ui.add_enabled(self.animation_ui.preview_animation.is_some() && !self.animation_ui.playing, egui::Button::new("Apply to selected keys")).clicked() {
                                self.apply_animation_keys_edit();
                            }
                            if self.animation_ui.preview_animation.is_some() && ui.small_button("Cancel preview").clicked() {
                                self.animation_ui.preview_animation = None;
                                self.animation_ui.preview_source = None;
                                self.status = "Multi-key edit preview canceled.".into();
                            }
                        });
                    });
                }
                ui.small("Edits belong to the displayed frame. Key the active layer before seeking; frame seeks reset frame-local undo. Exposures hold a layer snapshot for a chosen number of frames. Layer/resource structure stays static; layer moves are disabled while independent tracks exist. Linked-group documents must be made unique before keyframing. Onion skins are display-only.");
            });
    }

    fn layer_key_strip(&mut self, ui: &mut egui::Ui) {
        let animation = &self.doc.animation;
        let layer_id = self.doc.active_layer;
        let Some(track) = animation
            .layer_tracks
            .iter()
            .find(|track| track.layer_id == layer_id)
        else {
            return;
        };
        let frame = animation.current_frame;
        let previous = track.keyframes.iter().rev().find(|key| key.frame < frame);
        let next = track.keyframes.iter().find(|key| key.frame > frame);
        let mut jump = None;
        ui.horizontal(|ui| {
            for (label, key) in [("Previous layer key", previous), ("Next layer key", next)] {
                if ui
                    .add_enabled(
                        key.is_some() && !self.animation_ui.playing,
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    jump = key.map(|key| key.frame);
                }
            }
        });
        if let Some(frame) = jump {
            self.seek_artwork_animation(frame, false);
            return;
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
        let first = animation.range_start;
        let step = rect.width() / (animation.range_end - first + 1) as f32;
        let x = |frame: u32| rect.left() + (frame - first) as f32 * step;
        let editable = !self.animation_ui.playing && self.doc.active_editable();
        let mut retime = None;
        let mut seek = None;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
        for (index, key) in track.keyframes.iter().enumerate() {
            let end = track
                .keyframes
                .get(index + 1)
                .map_or(key.frame + key.exposure, |next| {
                    next.frame.min(key.frame + key.exposure)
                });
            let left = x(key.frame);
            let handle = Rect::from_min_max(
                egui::pos2(left, rect.top()),
                egui::pos2((left + 8.0).min(rect.right()), rect.bottom()),
            );
            let response = ui.interact(
                handle,
                ui.id().with(("retime", layer_id, key.frame)),
                if editable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::hover()
                },
            );
            let color = if key.strokes.is_empty() {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().selection.bg_fill
            };
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(left, rect.top() + 8.0),
                    egui::pos2(x(end), rect.bottom() - 4.0),
                ),
                1.0,
                color,
            );
            painter.rect_filled(handle.shrink2(egui::vec2(1.0, 2.0)), 2.0, color);
            if response.clicked() {
                seek = Some(key.frame);
            }
            if response.dragged() && step > 0.0 {
                let offset = ui.input(|input| {
                    input
                        .pointer
                        .interact_pos()
                        .zip(input.pointer.press_origin())
                        .map_or(0.0, |(current, start)| current.x - start.x)
                });
                let to = (i64::from(key.frame) + (offset / step).round() as i64)
                    .clamp(i64::from(first), i64::from(animation.range_end))
                    as u32;
                painter.vline(
                    x(to),
                    rect.y_range(),
                    egui::Stroke::new(2.0_f32, Color32::YELLOW),
                );
                response
                    .clone()
                    .on_hover_text(format!("Move key to frame {to}"));
                self.animation_ui.retime_preview = Some((layer_id, key.frame, to));
            } else if response.drag_stopped() {
                if let Some((_, _, to)) =
                    self.animation_ui
                        .retime_preview
                        .take()
                        .filter(|(layer, from, to)| {
                            *layer == layer_id && *from == key.frame && *to != key.frame
                        })
                {
                    retime = Some((key.frame, to));
                }
            } else {
                response.on_hover_text(format!(
                    "Frame {} · {} frame hold. Drag to retime.",
                    key.frame, key.exposure
                ));
            }
        }
        painter.vline(
            x(animation.current_frame),
            rect.y_range(),
            egui::Stroke::new(1.0_f32, ui.visuals().text_color()),
        );
        if let Some((from, to)) = retime {
            self.place_active_layer_key(from, to, false);
        } else if let Some(frame) = seek {
            self.seek_artwork_animation(frame, false);
        }
    }

    fn layer_key_frame_entry(&mut self, ui: &mut egui::Ui, from: u32) {
        let layer = self.doc.active_layer;
        let mut to = self
            .animation_ui
            .retime_entry
            .filter(|&(id, frame, _)| (id, frame) == (layer, from))
            .map_or(from, |(_, _, destination)| destination);
        let editable = !self.animation_ui.playing && self.doc.active_editable();
        ui.add_enabled_ui(editable, |ui| {
            ui.label("Destination frame");
            ui.add(
                egui::DragValue::new(&mut to)
                    .range(self.doc.animation.range_start..=self.doc.animation.range_end),
            );
            self.animation_ui.retime_entry = Some((layer, from, to));
            if ui
                .add_enabled(to != from, egui::Button::new("Move key"))
                .clicked()
            {
                self.place_active_layer_key(from, to, false);
            }
            if ui
                .add_enabled(to != from, egui::Button::new("Duplicate key"))
                .on_hover_text("Copy the drawing and its hold to the destination frame.")
                .clicked()
            {
                self.place_active_layer_key(from, to, true);
            }
        });
    }

    fn place_active_layer_key(&mut self, from: u32, to: u32, duplicate: bool) {
        if self.animation_ui.playing || !self.doc.active_editable() {
            return;
        }
        let animation = &self.doc.animation;
        if !animation.current_pose_matches(&self.doc.strokes) {
            self.status =
                "Set or update the current keyframe before moving or duplicating it.".into();
            return;
        }
        let result = animation
            .place_layer_key(self.doc.active_layer, from, to, duplicate)
            .and_then(|next| {
                let strokes = next
                    .resolve(next.current_frame)
                    .ok_or("Cannot resolve the destination drawing")?;
                let status = if duplicate {
                    "Layer key duplicated."
                } else {
                    "Layer key retimed."
                };
                self.commit_animation_state(next, strokes, status)
            });
        match result {
            Ok(()) => {
                self.animation_ui.retime_entry = None;
                self.animation_ui.preview_animation = None;
                self.animation_ui.preview_source = None;
                self.animation_ui.selected_key_frames.clear();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn paint_animation_onion_skins(&self, painter: &egui::Painter, rect: Rect) {
        let animation = &self.doc.animation;
        if self.animation_ui.preview_animation.is_none()
            && (!animation.onion_skins || animation.is_empty())
        {
            return;
        }
        if animation.onion_skins {
            let previous_color = Color32::from_rgba_unmultiplied(80, 180, 255, 105);
            let next_color = Color32::from_rgba_unmultiplied(255, 140, 80, 105);
            let frame = animation.current_frame;
            let paint = |strokes: &[Stroke], color| {
                for stroke in strokes.iter().filter(|stroke| {
                    self.doc.visible(stroke)
                        && self
                            .doc
                            .layers
                            .iter()
                            .any(|layer| layer.id == stroke.layer_id && !layer.hide_onion_skin)
                }) {
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
                        painter.line_segment(
                            [segment[0], segment[1]],
                            egui::Stroke::new(1.5_f32, color),
                        );
                    }
                }
            };
            for track in &animation.layer_tracks {
                let upper = track.keyframes.partition_point(|key| key.frame <= frame);
                let held = upper.checked_sub(1).filter(|&index| {
                    let key = &track.keyframes[index];
                    frame - key.frame < key.exposure
                });
                // The first key is displayed before its start by the pose resolver.
                if upper == 0 {
                    if let Some(key) = track.keyframes.get(1) {
                        paint(&key.strokes, next_color);
                    }
                    continue;
                }
                let previous = held.unwrap_or(upper).checked_sub(1);
                if let Some(index) = previous {
                    paint(&track.keyframes[index].strokes, previous_color);
                }
                if let Some(key) = track.keyframes.get(upper) {
                    paint(&key.strokes, next_color);
                }
            }
            if animation.layer_tracks.is_empty() {
                let upper = animation
                    .keyframes
                    .partition_point(|key| key.frame <= frame);
                if let Some(index) = upper.checked_sub(2) {
                    paint(&animation.keyframes[index].strokes, previous_color);
                }
                if let Some(key) = animation.keyframes.get(upper.max(1)) {
                    paint(&key.strokes, next_color);
                }
            }
        }
        if let Some(preview) = &self.animation_ui.preview_animation {
            let frame = animation.current_frame;
            if self.animation_ui.selected_key_frames.contains(&frame) {
                if let Some(strokes) = preview.resolve(frame) {
                    let color = Color32::from_rgba_unmultiplied(255, 210, 70, 225);
                    for stroke in strokes.iter().filter(|stroke| {
                        stroke.layer_id == self.doc.active_layer && self.doc.visible(stroke)
                    }) {
                        let positions: Vec<_> = stroke
                            .points
                            .iter()
                            .filter_map(|point| {
                                crate::gizmo::project(&self.camera, rect, point.position)
                                    .map(|(position, _)| position)
                            })
                            .collect();
                        if positions.len() == 1 {
                            painter.circle_filled(positions[0], 3.0, color);
                        }
                        for segment in positions.windows(2) {
                            painter.line_segment(
                                [segment[0], segment[1]],
                                egui::Stroke::new(2.5_f32, color),
                            );
                        }
                    }
                }
            }
        }
    }
}
