use super::*;
use crate::{editing, gizmo};
use glam::Quat;
use std::path::Path;

fn linked_documents_equal(first: &Document, second: &Document) -> bool {
    serde_json::to_vec(first).expect("document serializes")
        == serde_json::to_vec(second).expect("document serializes")
}

fn linked_group_portability_report(
    document: &Document,
    output: &Path,
) -> Result<serde_json::Value, String> {
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let native_path = output.join("linked-source.sketch.json");
    std::fs::write(
        &native_path,
        serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let native_reload = Document::from_json(
        &std::fs::read_to_string(&native_path).map_err(|error| error.to_string())?,
    )?;
    if !linked_documents_equal(&native_reload, document) {
        return Err("Native reload changed linked definitions or baked instance strokes".into());
    }

    let recovery_dir = output.join("recovery");
    let recovery_session = format!("sketch-086-{}", crate::recovery_store::session_id());
    let recovery_file = crate::recovery_store::write(
        &recovery_dir,
        &crate::recovery_store::RecoverySnapshot::new(
            recovery_session,
            Some(native_path.clone()),
            document.clone(),
        ),
    )?;
    let recovery_scan = crate::recovery_store::scan(&recovery_dir, "sketch-086-fresh-reader")?;
    let recovery = recovery_scan
        .candidates
        .iter()
        .find(|candidate| candidate.path.file_name() == recovery_file.file_name())
        .ok_or("Fresh recovery reader did not find the linked-group snapshot")?;
    if !linked_documents_equal(&recovery.snapshot.document, document)
        || !recovery_scan.issues.is_empty()
    {
        return Err("Recovery reload changed linked groups or reported a storage issue".into());
    }

    let destination = Document::default();
    let destination_assets = output.join("merge-assets");
    let inserted = super::native_merge::prepare_insert(
        &destination,
        &native_path,
        &destination_assets,
        10_000,
        2,
        20_000,
        Vec3::new(2.5, -0.25, 0.0),
        false,
        &HashMap::new(),
    )?;
    let mut merged = destination.clone();
    let merge_edit = editing::Edit::Merge(inserted.edit);
    merge_edit.apply(&mut merged, true);
    let merged_after = merged.clone();
    if merged.linked_stroke_groups.definitions.is_empty()
        || merged.linked_stroke_groups.instances.is_empty()
        || merged
            .linked_stroke_groups
            .definitions
            .iter()
            .any(|definition| {
                document
                    .linked_stroke_groups
                    .definitions
                    .iter()
                    .any(|source| source.id == definition.id)
                    || definition
                        .source_stroke_ids
                        .iter()
                        .any(|id| !merged.strokes.iter().any(|stroke| stroke.id == *id))
            })
        || merged
            .linked_stroke_groups
            .instances
            .iter()
            .any(|instance| {
                document
                    .linked_stroke_groups
                    .instances
                    .iter()
                    .any(|source| source.id == instance.id)
                    || document
                        .linked_stroke_groups
                        .instances
                        .iter()
                        .flat_map(|source| source.stroke_ids.iter())
                        .any(|id| instance.stroke_ids.contains(id))
                    || instance.stroke_ids.iter().any(|id| {
                        merged
                            .linked_instance_for_stroke(*id)
                            .is_none_or(|owner| owner.id != instance.id)
                    })
            })
    {
        return Err("Native merge did not remap linked-group ownership and generated IDs".into());
    }
    merge_edit.apply(&mut merged, false);
    if !linked_documents_equal(&merged, &destination) {
        return Err("Undo of linked-group merge did not restore the destination".into());
    }
    merge_edit.apply(&mut merged, true);
    if !linked_documents_equal(&merged, &merged_after) {
        return Err("Redo of linked-group merge did not restore remapped links".into());
    }
    let merged_path = output.join("linked-merged.sketch.json");
    std::fs::write(
        &merged_path,
        serde_json::to_vec_pretty(&merged).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let merged_reload = Document::from_json(
        &std::fs::read_to_string(&merged_path).map_err(|error| error.to_string())?,
    )?;
    if !linked_documents_equal(&merged_reload, &merged) {
        return Err("Reload after linked-group merge changed the merged document".into());
    }

    let visible_ids: Vec<u64> = merged
        .strokes
        .iter()
        .filter(|stroke| merged.visible(stroke) && !stroke.points.is_empty())
        .map(|stroke| stroke.id)
        .collect();
    let encoded = crate::export::encode(&merged, None)?;
    let glb_path = output.join("linked-baked-export.glb");
    std::fs::write(&glb_path, &encoded.bytes).map_err(|error| error.to_string())?;
    let glb_json = glb_json_chunk(&encoded.bytes)?;
    let glb_stroke_ids = glb_json["nodes"]
        .as_array()
        .ok_or("GLB has no node array")?
        .iter()
        .filter_map(|node| {
            node["extras"]["spatialSketchStrokeId"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
        })
        .collect::<Vec<_>>();
    let mut expected_ids = visible_ids.clone();
    expected_ids.sort_unstable();
    let mut glb_ids = glb_stroke_ids;
    glb_ids.sort_unstable();
    let glb_meshes = glb_json["meshes"]
        .as_array()
        .ok_or("GLB has no mesh array")?
        .len();
    if encoded.summary.strokes != visible_ids.len() || glb_ids != expected_ids {
        return Err("GLB omitted or changed baked linked-instance strokes".into());
    }

    let obj_path = output.join("linked-baked-export.obj");
    let obj_bytes = crate::obj_export::write_obj_bundle(&merged, None, &obj_path, false)?;
    let obj_text = std::fs::read_to_string(&obj_path).map_err(|error| error.to_string())?;
    let mut obj_ids: Vec<u64> = obj_text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("g stroke_")
                .and_then(|id| id.parse::<u64>().ok())
        })
        .collect();
    obj_ids.sort_unstable();
    if obj_ids != expected_ids {
        return Err("OBJ omitted or changed baked linked-instance strokes".into());
    }

    let report = serde_json::json!({
        "passed": true,
        "checks": [
            "native save/reload preserves linked definitions and stable baked instance strokes",
            "fresh recovery reader restores linked definitions, instances, and generated geometry",
            "native merge apply/undo/redo/reload remaps link ownership, generated IDs, and layers",
            "GLB JSON and stroke nodes include every baked source and linked-instance stroke",
            "OBJ object groups include every baked source and linked-instance stroke"
        ],
        "native": {
            "file": native_path.file_name().unwrap().to_string_lossy(),
            "version": native_reload.version,
            "strokes": native_reload.strokes.len(),
            "definitions": native_reload.linked_stroke_groups.definitions.len(),
            "instances": native_reload.linked_stroke_groups.instances.len()
        },
        "recovery": {
            "fresh_reader": true,
            "file": recovery_file.file_name().unwrap().to_string_lossy(),
            "strokes": recovery.snapshot.document.strokes.len(),
            "definitions": recovery.snapshot.document.linked_stroke_groups.definitions.len(),
            "instances": recovery.snapshot.document.linked_stroke_groups.instances.len()
        },
        "merge": {
            "file": merged_path.file_name().unwrap().to_string_lossy(),
            "strokes": merged_reload.strokes.len(),
            "definitions": merged_reload.linked_stroke_groups.definitions.len(),
            "instances": merged_reload.linked_stroke_groups.instances.len(),
            "undo_redo": true
        },
        "glb": {
            "file": glb_path.file_name().unwrap().to_string_lossy(),
            "baked_stroke_count": encoded.summary.strokes,
            "mesh_count": glb_meshes,
            "bytes": encoded.summary.bytes
        },
        "obj": {
            "file": obj_path.file_name().unwrap().to_string_lossy(),
            "baked_stroke_groups": obj_ids.len(),
            "bytes_with_materials": obj_bytes
        }
    });
    std::fs::write(
        output.join("linked-group-report.json"),
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(report)
}

fn glb_json_chunk(bytes: &[u8]) -> Result<serde_json::Value, String> {
    if bytes.len() < 20 || &bytes[..4] != b"glTF" || &bytes[16..20] != b"JSON" {
        return Err("Export is not a GLB with a JSON first chunk".into());
    }
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let end = 20usize
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or("GLB JSON chunk length is invalid")?;
    serde_json::from_slice(&bytes[20..end]).map_err(|error| error.to_string())
}

impl SketchApp {
    fn apply_group_action(&mut self, ungroup: bool) {
        self.finish();
        let result = if ungroup {
            editing::ungroup_selected(&self.doc, &self.selected)
        } else {
            editing::group_selected(&self.doc, &self.selected)
        };
        match result {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.renderer.invalidate();
                self.sync_selection();
                self.status = if ungroup {
                    "Selection ungrouped"
                } else {
                    "Selection grouped"
                }
                .into();
            }
            Err(error) => self.status = error,
        }
    }
    pub(super) fn group_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.selected.len() >= 2, egui::Button::new("Group"))
                .on_hover_text("Group selected strokes on one editable layer (Ctrl/Cmd+G)")
                .clicked()
            {
                self.apply_group_action(false);
            }
            let grouped = self
                .doc
                .strokes
                .iter()
                .any(|s| self.selected.contains(&s.id) && s.group_id.is_some());
            if ui
                .add_enabled(grouped, egui::Button::new("Ungroup"))
                .on_hover_text("Ungroup complete selected groups (Ctrl/Cmd+Shift+G)")
                .clicked()
            {
                self.apply_group_action(true);
            }
        });
        let mut individual = self.individual_selection;
        if ui
            .checkbox(&mut individual, "Select individual strokes")
            .changed()
        {
            self.set_individual_selection(individual);
        }
        ui.small("Click selects a whole group; Shift-click toggles it. Individual mode edits members. Groups stay on one layer.");
        ui.separator();
        self.linked_group_controls(ui);
    }

    fn linked_group_controls(&mut self, ui: &mut egui::Ui) {
        let selected_instance = self.selected_linked_instance();
        ui.label("Linked stroke groups");
        if ui
            .add_enabled(
                self.selected.len() >= 2 && selected_instance.is_none(),
                egui::Button::new("Create linked instance"),
            )
            .on_hover_text("Create a linked definition from a complete regular stroke group")
            .clicked()
        {
            self.create_linked_group();
        }

        let definitions: Vec<_> = self
            .doc
            .linked_stroke_groups
            .definitions
            .iter()
            .map(|definition| {
                let instances = self
                    .doc
                    .linked_stroke_groups
                    .instances
                    .iter()
                    .filter(|instance| instance.definition_id == definition.id)
                    .count();
                (definition.id, definition.source_stroke_ids.len(), instances)
            })
            .collect();
        for (definition_id, strokes, instances) in definitions {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Group {definition_id}: {strokes} strokes, {instances} copies"
                ));
                if ui.button("Add copy").clicked() {
                    self.add_linked_group_copy(definition_id, instances);
                }
            });
        }

        if let Some(instance_id) = selected_instance {
            ui.horizontal(|ui| {
                if ui.button("Move −U").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        -self.active_guide_u() * 0.25,
                        Quat::IDENTITY,
                        1.0,
                    );
                }
                if ui.button("Move +U").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        self.active_guide_u() * 0.25,
                        Quat::IDENTITY,
                        1.0,
                    );
                }
                let normal = self
                    .planar_drawing_target()
                    .map_or(Vec3::Z, |target| target.plane.basis().2);
                if ui.button("Rotate −15°").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        Vec3::ZERO,
                        Quat::from_axis_angle(normal, -15.0_f32.to_radians()),
                        1.0,
                    );
                }
                if ui.button("Rotate +15°").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        Vec3::ZERO,
                        Quat::from_axis_angle(normal, 15.0_f32.to_radians()),
                        1.0,
                    );
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Scale 90%").clicked() {
                    self.transform_linked_instance(instance_id, Vec3::ZERO, Quat::IDENTITY, 0.9);
                }
                if ui.button("Scale 110%").clicked() {
                    self.transform_linked_instance(instance_id, Vec3::ZERO, Quat::IDENTITY, 1.1);
                }
                if ui.button("Make unique").clicked() {
                    self.remove_linked_group_instance(instance_id, true);
                }
                if ui.button("Delete instance").clicked() {
                    self.remove_linked_group_instance(instance_id, false);
                }
            });
            ui.small("Linked copies update when their source group changes. Make a copy unique before editing or erasing it directly.");
        }
    }

    fn selected_linked_instance(&self) -> Option<u64> {
        if self.selected.is_empty() {
            return None;
        }
        let instances: HashSet<_> = self
            .selected
            .iter()
            .filter_map(|id| {
                self.doc
                    .linked_instance_for_stroke(*id)
                    .map(|instance| instance.id)
            })
            .collect();
        (instances.len() == 1
            && self
                .selected
                .iter()
                .all(|id| self.doc.linked_instance_for_stroke(*id).is_some()))
        .then(|| *instances.iter().next().unwrap())
    }

    fn apply_linked_group_edit(
        &mut self,
        edit: editing::Edit,
        select_ids: Option<HashSet<u64>>,
        consumed_ids: usize,
        status: &str,
    ) {
        if let Some(end) = self.next_id.checked_add(consumed_ids as u64) {
            self.next_id = end;
        }
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        if let Some(ids) = select_ids {
            self.selected = ids;
        }
        self.sync_selection();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = status.into();
    }

    fn create_linked_group(&mut self) {
        let count = self.selected.len();
        match editing::create_linked_group(
            &self.doc,
            &self.selected,
            self.next_id,
            self.doc.active_layer,
            self.active_guide_u() * 0.5,
        ) {
            Ok(edit) => {
                let (ids, count) = match &edit {
                    editing::Edit::LinkedStrokeGroups { after, .. } => after
                        .instances
                        .last()
                        .map(|instance| {
                            (
                                Some(instance.stroke_ids.iter().copied().collect()),
                                instance.stroke_ids.len(),
                            )
                        })
                        .unwrap_or((None, count)),
                    _ => (None, count),
                };
                self.apply_linked_group_edit(edit, ids, count, "Linked stroke group created.");
            }
            Err(error) => self.status = error,
        }
    }

    fn add_linked_group_copy(&mut self, definition_id: u64, existing_copies: usize) {
        let count = self
            .doc
            .linked_stroke_groups
            .definitions
            .iter()
            .find(|definition| definition.id == definition_id)
            .map_or(0, |definition| definition.source_stroke_ids.len());
        let offset = self.active_guide_u() * (0.5 * (existing_copies as f32 + 1.0));
        match editing::add_linked_instance(
            &self.doc,
            definition_id,
            self.next_id,
            self.doc.active_layer,
            offset,
        ) {
            Ok(edit) => {
                let ids = match &edit {
                    editing::Edit::LinkedStrokeGroups { after, .. } => after
                        .instances
                        .last()
                        .map(|instance| instance.stroke_ids.iter().copied().collect()),
                    _ => None,
                };
                self.apply_linked_group_edit(edit, ids, count, "Linked copy created.");
            }
            Err(error) => self.status = error,
        }
    }

    fn transform_linked_instance(
        &mut self,
        instance_id: u64,
        translation: Vec3,
        rotation: Quat,
        scale: f32,
    ) {
        match editing::transform_linked_instance(
            &self.doc,
            instance_id,
            translation,
            rotation,
            scale,
        ) {
            Ok(edit) => {
                self.apply_linked_group_edit(edit, None, 0, "Linked instance transform applied.")
            }
            Err(error) => self.status = error,
        }
    }

    fn remove_linked_group_instance(&mut self, instance_id: u64, make_unique: bool) {
        match editing::remove_linked_instance(&self.doc, instance_id, make_unique) {
            Ok(edit) => {
                if make_unique {
                    let ids: HashSet<_> = self.selected.clone();
                    self.apply_linked_group_edit(
                        edit,
                        Some(ids),
                        0,
                        "Instance made unique. It can now be edited independently.",
                    );
                } else {
                    self.apply_linked_group_edit(
                        edit,
                        Some(HashSet::new()),
                        0,
                        "Linked instance deleted. Undo restores the link.",
                    );
                }
            }
            Err(error) => self.status = error,
        }
    }
    fn set_individual_selection(&mut self, individual: bool) {
        self.finish();
        self.individual_selection = individual;
        if !individual {
            let groups: HashSet<_> = self
                .doc
                .strokes
                .iter()
                .filter(|s| self.selected.contains(&s.id))
                .filter_map(|s| s.group_id)
                .collect();
            self.selected.extend(
                self.doc
                    .strokes
                    .iter()
                    .filter(|s| {
                        s.group_id.is_some_and(|id| groups.contains(&id)) && self.doc.editable(s)
                    })
                    .map(|s| s.id),
            );
        }
    }
    pub(super) fn group_shortcuts(&mut self, ctx: &egui::Context) {
        let action = ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(Key::G) {
                Some(i.modifiers.shift)
            } else {
                None
            }
        });
        if self.select_mode {
            if let Some(ungroup) = action {
                self.apply_group_action(ungroup);
            }
        }
    }
    pub(super) fn verify_groups(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.group_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.recovery_scan_seen = true;
                self.recovery_prompt = false;
                let original = self.doc.strokes.clone();
                let ids: HashSet<_> = original.iter().take(18).map(|s| s.id).collect();
                self.select_mode = true;
                self.selected = ids.clone();
                self.apply_group_action(false);
                let grouped = self.doc.strokes.clone();
                let group = self.doc.strokes[0].group_id.unwrap();
                assert_eq!(self.doc.group_members(group), ids);
                assert_eq!(self.doc.selection_for_stroke(original[0].id, false), ids);
                assert_eq!(
                    self.doc.selection_for_stroke(original[0].id, true),
                    HashSet::from([original[0].id])
                );
                self.undo();
                assert_eq!(self.doc.strokes, original);
                self.redo();
                assert_eq!(self.doc.strokes, grouped);
                self.selected = ids.clone();
                self.duplicate_selected();
                let copies = self.selected.clone();
                let copy_group = self.doc.strokes.last().unwrap().group_id.unwrap();
                assert_ne!(copy_group, group);
                assert_eq!(self.doc.group_members(copy_group), copies);
                self.transform_mode = Mode::Move;
                self.transform_values = [0.4, 0.3, 0.0];
                self.numeric_transform();
                let moved = self.doc.strokes.clone();
                self.undo();
                self.redo();
                assert_eq!(self.doc.strokes, moved);
                self.apply_group_action(true);
                assert!(self
                    .doc
                    .strokes
                    .iter()
                    .filter(|s| copies.contains(&s.id))
                    .all(|s| s.group_id.is_none()));
                self.undo();
                assert_eq!(self.doc.group_members(copy_group), copies);
                let loaded =
                    Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
                assert_eq!(loaded.version, 27);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.apply_layer_action(editing::LayerAction::Lock(1, true));
                assert!(self
                    .doc
                    .selection_for_stroke(original[0].id, false)
                    .is_empty());
                self.undo();
                self.selected = copies;
                self.select_mode = true;
                self.transform_mode = Mode::Move;
                assert!(
                    gizmo::project(&self.camera, rect, self.doc.strokes[0].points[0].position)
                        .is_some()
                );
                self.status="Grouped house detail / duplicate, transform, ungroup, undo, layers and persistence passed.".into();
                self.verify_linked_group_workflow(&ids);
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Group screenshot timed out"),
            _ => {}
        }
        self.group_verification_frame = Some(frame + 1);
    }

    fn verify_linked_group_workflow(&mut self, source_ids: &HashSet<u64>) {
        assert!(self.doc.linked_stroke_groups.definitions.is_empty());
        assert!(self.doc.linked_stroke_groups.instances.is_empty());
        self.select_mode = true;
        self.selected = source_ids.clone();
        let before_create = self.doc.clone();
        let undo_before_create = self.undo.len();
        self.create_linked_group();
        assert_eq!(self.doc.linked_stroke_groups.definitions.len(), 1);
        assert_eq!(self.doc.linked_stroke_groups.instances.len(), 1);
        assert_eq!(self.undo.len(), undo_before_create + 1);
        let created = self.doc.clone();
        self.undo();
        assert!(
            linked_documents_equal(&self.doc, &before_create),
            "linked group creation undoes exactly"
        );
        self.redo();
        assert!(
            linked_documents_equal(&self.doc, &created),
            "linked group creation redoes exactly"
        );

        let definition_id = self.doc.linked_stroke_groups.definitions[0].id;
        let first_instance = self.doc.linked_stroke_groups.instances[0].clone();
        self.add_linked_group_copy(definition_id, 1);
        assert_eq!(self.doc.linked_stroke_groups.instances.len(), 2);
        let second_instance = self.doc.linked_stroke_groups.instances[1].clone();
        let before_instance_transform = self.doc.clone();
        self.transform_linked_instance(
            second_instance.id,
            Vec3::new(0.5, 0.0, 0.0),
            Quat::from_rotation_z(0.12),
            1.05,
        );
        let after_instance_transform = self.doc.clone();
        assert!(!linked_documents_equal(
            &after_instance_transform,
            &before_instance_transform
        ));
        self.undo();
        assert!(linked_documents_equal(
            &self.doc,
            &before_instance_transform
        ));
        self.redo();
        assert!(linked_documents_equal(&self.doc, &after_instance_transform));

        let generated_before: HashMap<_, _> = first_instance
            .stroke_ids
            .iter()
            .map(|id| {
                let stroke = self
                    .doc
                    .strokes
                    .iter()
                    .find(|stroke| stroke.id == *id)
                    .expect("first linked copy has baked stroke geometry");
                (*id, stroke.points.clone())
            })
            .collect();
        self.selected = source_ids.clone();
        self.transform_mode = Mode::Move;
        self.transform_values = [0.12, 0.08, 0.0];
        let before_source_transform = self.doc.clone();
        let undo_before_source_transform = self.undo.len();
        self.numeric_transform();
        let after_source_transform = self.doc.clone();
        assert_eq!(self.undo.len(), undo_before_source_transform + 1);
        assert!(first_instance.stroke_ids.iter().any(|id| {
            self.doc
                .strokes
                .iter()
                .find(|stroke| stroke.id == *id)
                .is_some_and(|stroke| stroke.points != generated_before[id])
        }));
        self.undo();
        assert!(linked_documents_equal(&self.doc, &before_source_transform));
        self.redo();
        assert!(linked_documents_equal(&self.doc, &after_source_transform));

        let session = crate::recovery_store::session_id();
        let artifact_root = std::env::current_dir()
            .unwrap()
            .join("verification")
            .join("sketch-086")
            .join(format!("run-{session}"));
        let mut report = linked_group_portability_report(&self.doc, &artifact_root)
            .expect("linked-group recovery, merge, and export review succeeds");
        report["interactive"] = serde_json::json!({
            "checks": [
                "production group controls create two linked copies with stable generated stroke IDs",
                "instance transform is one undoable edit and restores exact generated geometry",
                "source numeric transform regenerates linked copies and survives undo/redo"
            ],
            "source_strokes": source_ids.len(),
            "definitions": self.doc.linked_stroke_groups.definitions.len(),
            "instances": self.doc.linked_stroke_groups.instances.len(),
            "generated_strokes": self.doc.linked_generated_ids().len(),
            "screenshot": "verification/group-preview.png"
        });
        std::fs::write(
            artifact_root.join("linked-group-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .expect("linked-group report artifact writes");
        self.selected = first_instance.stroke_ids.into_iter().collect();
        self.select_mode = true;
        self.transform_mode = Mode::Move;
        self.status = format!(
            "Linked-group recovery, merge, and baked GLB/OBJ exports passed. Report: {}",
            artifact_root.join("linked-group-report.json").display()
        );
        self.renderer.invalidate();
    }
}

#[cfg(test)]
mod linked_group_tests {
    use super::*;
    use crate::document::{Point, Stroke, StrokeMaterial};
    use std::collections::HashSet;

    fn source_stroke(id: u64, group_id: u64, y: f32) -> Stroke {
        Stroke {
            model_surface: None,
            curve: None,
            material: StrokeMaterial::Shaded,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: Some(group_id),
            layer_id: 1,
            id,
            color: [0.22, 0.48, 0.82],
            gradient: None,
            radius: 0.045,
            brush: Default::default(),
            points: vec![
                Point {
                    position: Vec3::new(-0.6, y, 0.0),
                    pressure: 0.6,
                    tilt: None,
                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(0.6, y + 0.15, 0.0),
                    pressure: 0.9,
                    tilt: None,
                    time_ms: 30,
                },
            ],
        }
    }

    fn linked_fixture() -> Document {
        let mut document = Document {
            strokes: vec![source_stroke(10, 7, 0.0), source_stroke(11, 7, 0.4)],
            ..Document::default()
        };
        let source_ids = HashSet::from([10, 11]);
        editing::create_linked_group(&document, &source_ids, 100, 1, Vec3::new(2.0, 0.25, 0.0))
            .unwrap()
            .apply(&mut document, true);
        let definition = document.linked_stroke_groups.definitions[0].id;
        editing::add_linked_instance(&document, definition, 200, 1, Vec3::new(4.0, 0.5, 0.0))
            .unwrap()
            .apply(&mut document, true);
        let second = document.linked_stroke_groups.instances[1].id;
        editing::transform_linked_instance(
            &document,
            second,
            Vec3::new(0.25, 0.0, 0.0),
            Quat::from_rotation_z(0.15),
            1.1,
        )
        .unwrap()
        .apply(&mut document, true);

        let before = editing::snapshot(&document, &source_ids);
        let after = editing::transformed(
            &before,
            Vec3::ZERO,
            Vec3::new(0.2, 0.1, 0.0),
            Quat::IDENTITY,
            1.0,
        )
        .unwrap();
        editing::Edit::Change { before, after }.apply(&mut document, true);
        document
    }

    #[test]
    fn undoing_link_creation_removes_generated_baked_strokes() {
        let original = Document {
            strokes: vec![source_stroke(10, 7, 0.0), source_stroke(11, 7, 0.4)],
            ..Document::default()
        };
        let source_ids = HashSet::from([10, 11]);
        let edit =
            editing::create_linked_group(&original, &source_ids, 100, 1, Vec3::new(2.0, 0.25, 0.0))
                .unwrap();
        let mut document = original.clone();
        edit.apply(&mut document, true);
        let created = document.clone();
        assert_eq!(document.linked_generated_ids().len(), 2);
        assert!(document
            .linked_generated_ids()
            .iter()
            .all(|id| document.strokes.iter().any(|stroke| stroke.id == *id)));

        edit.apply(&mut document, false);
        assert!(linked_documents_equal(&document, &original));
        assert!(document.linked_generated_ids().is_empty());
        assert_eq!(document.strokes.len(), original.strokes.len());

        edit.apply(&mut document, true);
        assert!(linked_documents_equal(&document, &created));
    }

    #[test]
    fn linked_groups_survive_recovery_merge_reload_and_baked_glb_obj_exports() {
        let document = linked_fixture();
        let generated_ids = document.linked_generated_ids();
        assert_eq!(generated_ids.len(), 4);
        assert_eq!(document.linked_stroke_groups.definitions.len(), 1);
        assert_eq!(document.linked_stroke_groups.instances.len(), 2);
        assert!(generated_ids
            .iter()
            .all(|id| document.strokes.iter().any(|s| s.id == *id)));

        let output = tempfile::tempdir().unwrap();
        let report = linked_group_portability_report(&document, output.path()).unwrap();
        assert_eq!(report["passed"], true);
        assert_eq!(report["native"]["version"], 27);
        assert_eq!(report["native"]["instances"], 2);
        assert_eq!(report["recovery"]["fresh_reader"], true);
        assert_eq!(report["merge"]["undo_redo"], true);
        assert_eq!(report["merge"]["definitions"], 1);
        assert_eq!(report["merge"]["instances"], 2);
        assert_eq!(report["glb"]["baked_stroke_count"], 6);
        assert_eq!(report["glb"]["mesh_count"], 6);
        assert_eq!(report["obj"]["baked_stroke_groups"], 6);
        for artifact in [
            "linked-source.sketch.json",
            "linked-merged.sketch.json",
            "linked-baked-export.glb",
            "linked-baked-export.obj",
            "linked-group-report.json",
        ] {
            assert!(output.path().join(artifact).is_file(), "{artifact}");
        }
    }
}
