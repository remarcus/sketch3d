//! Pure-data translation transactions for selections spanning document resources.
//!
//! This module deliberately supports translation only. UI gesture capture,
//! resource picking, history integration, and rotation/scaling rules belong to
//! the caller.

use crate::{
    document::{
        guide_resources::{GuideGeometry, GuideResource},
        model_references::ObjReferenceResource,
        Document, Stroke,
    },
    image_planes::ImagePlane,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Kept explicit so unsupported mixed transforms fail before mutation.
pub enum TransformMode {
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveDirection {
    Forward,
    Restore,
}

/// Resource IDs are kept in their own namespaces because document catalogs
/// allocate IDs independently.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceSelection {
    pub stroke_ids: HashSet<u64>,
    pub image_plane_ids: HashSet<u64>,
    pub obj_reference_ids: HashSet<u64>,
    pub guide_ids: HashSet<u64>,
}

impl ResourceSelection {
    pub fn is_empty(&self) -> bool {
        self.stroke_ids.is_empty()
            && self.image_plane_ids.is_empty()
            && self.obj_reference_ids.is_empty()
            && self.guide_ids.is_empty()
    }
}

/// Exact source and translated records. Vectors are sorted by resource ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceState {
    pub strokes: Vec<Stroke>,
    pub image_planes: Vec<ImagePlane>,
    pub obj_references: Vec<ObjReferenceResource>,
    pub guides: Vec<GuideResource>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoveSnapshot {
    pub selection: ResourceSelection,
    pub before: ResourceState,
    pub after: ResourceState,
    pub translation: Vec3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveError {
    UnsupportedTransform(TransformMode),
    EmptySelection,
    InvalidTranslation,
    MissingResource { kind: &'static str, id: u64 },
    ProtectedResource { kind: &'static str, id: u64 },
    StaleSnapshot,
    InvalidDocument(String),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedTransform(mode) => {
                write!(
                    formatter,
                    "{mode:?} is not supported for mixed-resource transforms"
                )
            }
            Self::EmptySelection => formatter.write_str("Select at least one movable resource"),
            Self::InvalidTranslation => {
                formatter.write_str("Translation must be finite and within document bounds")
            }
            Self::MissingResource { kind, id } => write!(formatter, "Missing {kind} {id}"),
            Self::ProtectedResource { kind, id } => {
                write!(formatter, "Selected {kind} {id} is hidden or locked")
            }
            Self::StaleSnapshot => formatter.write_str(
                "Selected resources changed after this mixed-resource preview was created",
            ),
            Self::InvalidDocument(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for MoveError {}

impl MoveSnapshot {
    /// Capture eligible resources and build a translated preview from immutable
    /// originals. No document data is mutated by this operation.
    pub fn capture(
        document: &Document,
        selection: ResourceSelection,
        mode: TransformMode,
        translation: Vec3,
    ) -> Result<Self, MoveError> {
        if mode != TransformMode::Translate {
            return Err(MoveError::UnsupportedTransform(mode));
        }
        if selection.is_empty() {
            return Err(MoveError::EmptySelection);
        }
        if !translation.is_finite() || translation.abs().max_element() > 20_000.0 {
            return Err(MoveError::InvalidTranslation);
        }

        let before = capture_state(document, &selection)?;
        let mut after = before.clone();
        for stroke in &mut after.strokes {
            for point in &mut stroke.points {
                point.position += translation;
            }
        }
        for plane in &mut after.image_planes {
            plane.origin += translation;
        }
        for model in &mut after.obj_references {
            model.center += translation;
        }
        for guide in &mut after.guides {
            translate_guide(guide, translation);
        }

        validate_candidate(document, &after)?;
        Ok(Self {
            selection,
            before,
            after,
            translation,
        })
    }

    /// Atomically apply or restore this preview. The current selected records
    /// must exactly match the expected side of the transaction; unrelated data
    /// and active drawable bindings are retained.
    pub fn apply(
        &self,
        document: &mut Document,
        direction: MoveDirection,
    ) -> Result<(), MoveError> {
        let (expected, replacement) = match direction {
            MoveDirection::Forward => (&self.before, &self.after),
            MoveDirection::Restore => (&self.after, &self.before),
        };
        if capture_state(document, &self.selection)? != *expected {
            return Err(MoveError::StaleSnapshot);
        }

        let mut candidate = document.clone();
        write_state(&mut candidate, replacement);
        validate_document(&candidate)?;
        write_state(document, replacement);
        Ok(())
    }
}

fn capture_state(
    document: &Document,
    selection: &ResourceSelection,
) -> Result<ResourceState, MoveError> {
    let mut state = ResourceState {
        strokes: Vec::with_capacity(selection.stroke_ids.len()),
        image_planes: Vec::with_capacity(selection.image_plane_ids.len()),
        obj_references: Vec::with_capacity(selection.obj_reference_ids.len()),
        guides: Vec::with_capacity(selection.guide_ids.len()),
    };

    for stroke_id in &selection.stroke_ids {
        let stroke = document
            .strokes
            .iter()
            .find(|stroke| stroke.id == *stroke_id)
            .ok_or(MoveError::MissingResource {
                kind: "stroke",
                id: *stroke_id,
            })?;
        if !document.editable(stroke) {
            return Err(MoveError::ProtectedResource {
                kind: "stroke",
                id: *stroke_id,
            });
        }
        state.strokes.push(stroke.clone());
    }
    for id in &selection.image_plane_ids {
        let plane = document
            .image_planes
            .iter()
            .find(|plane| plane.id == *id)
            .ok_or(MoveError::MissingResource {
                kind: "image plane",
                id: *id,
            })?;
        if !plane.visible || plane.locked {
            return Err(MoveError::ProtectedResource {
                kind: "image plane",
                id: *id,
            });
        }
        state.image_planes.push(plane.clone());
    }
    for id in &selection.obj_reference_ids {
        let model = document
            .model_references
            .iter()
            .find(|model| model.id == *id)
            .ok_or(MoveError::MissingResource {
                kind: "OBJ reference",
                id: *id,
            })?;
        if !model.visible || model.locked {
            return Err(MoveError::ProtectedResource {
                kind: "OBJ reference",
                id: *id,
            });
        }
        state.obj_references.push(model.clone());
    }
    for id in &selection.guide_ids {
        let guide = document
            .guide_resources
            .iter()
            .find(|guide| guide.id == *id)
            .ok_or(MoveError::MissingResource {
                kind: "guide",
                id: *id,
            })?;
        if !guide.visible || guide.locked {
            return Err(MoveError::ProtectedResource {
                kind: "guide",
                id: *id,
            });
        }
        state.guides.push(guide.clone());
    }

    state.strokes.sort_by_key(|stroke| stroke.id);
    state.image_planes.sort_by_key(|plane| plane.id);
    state.obj_references.sort_by_key(|model| model.id);
    state.guides.sort_by_key(|guide| guide.id);
    Ok(state)
}

fn translate_guide(guide: &mut GuideResource, translation: Vec3) {
    match &mut guide.geometry {
        GuideGeometry::Plane { origin, .. } => *origin += translation,
        GuideGeometry::Cylinder { center, .. }
        | GuideGeometry::Revolved { center, .. }
        | GuideGeometry::Sphere { center, .. }
        | GuideGeometry::Ring { center, .. }
        | GuideGeometry::Cone { center, .. } => *center += translation,
        GuideGeometry::Freeform { surface } => surface.center += translation,
        GuideGeometry::Loft { surface } => surface.center += translation,
    }
}

fn write_state(document: &mut Document, state: &ResourceState) {
    for replacement in &state.strokes {
        if let Some(current) = document
            .strokes
            .iter_mut()
            .find(|current| current.id == replacement.id)
        {
            *current = replacement.clone();
        }
    }
    for replacement in &state.image_planes {
        if let Some(current) = document
            .image_planes
            .iter_mut()
            .find(|current| current.id == replacement.id)
        {
            *current = replacement.clone();
        }
    }
    for replacement in &state.obj_references {
        if let Some(current) = document
            .model_references
            .iter_mut()
            .find(|current| current.id == replacement.id)
        {
            *current = replacement.clone();
        }
    }
    for replacement in &state.guides {
        if let Some(current) = document
            .guide_resources
            .iter_mut()
            .find(|current| current.id == replacement.id)
        {
            *current = replacement.clone();
        }
    }
}

/// Restore a previously validated transaction snapshot for undo/redo.
pub fn restore_state(document: &mut Document, state: &ResourceState) {
    write_state(document, state);
}

fn validate_candidate(document: &Document, after: &ResourceState) -> Result<(), MoveError> {
    let mut candidate = document.clone();
    write_state(&mut candidate, after);
    validate_document(&candidate)
}

fn validate_document(document: &Document) -> Result<(), MoveError> {
    let json = serde_json::to_string(document)
        .map_err(|error| MoveError::InvalidDocument(error.to_string()))?;
    Document::from_json(&json)
        .map(|_| ())
        .map_err(MoveError::InvalidDocument)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{
            guide_resources::GuideGeometry, model_references::ObjReferenceResource, Brush, Layer,
            Point,
        },
        image_planes::ImagePlane,
        obj_reference_storage::ObjReferenceAsset,
    };
    use glam::{Quat, Vec2};

    fn fixture() -> Document {
        let mut document = Document::default();
        document.strokes.push(Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 11,
            color: [0.5; 3],
            radius: 0.1,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::new(1.0, 2.0, 3.0),
                pressure: 0.7,
                tilt: Some(Vec3::Y),

                time_ms: 0,
            }],
        });
        document.image_planes.push(ImagePlane {
            id: 21,
            name: "Reference image".into(),
            asset_file: "image-21.png".into(),
            pixel_size: Some([16, 8]),
            origin: Vec3::new(1.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            size: Vec2::new(2.0, 1.0),
            opacity: 0.8,
            visible: true,
            locked: false,
            drawable: true,
        });
        document.active_image_plane = Some(21);
        document.model_references.push(ObjReferenceResource {
            id: 31,
            name: "Model".into(),
            visible: true,
            locked: false,
            center: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: 1.0,
            asset: ObjReferenceAsset {
                id: 31,
                bundle_dir: "models/31".into(),
            },
        });
        document.guide_resources.push(GuideResource {
            id: 41,
            name: "Guide".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Plane {
                origin: Vec3::new(0.0, 0.0, 1.0),
                rotation: Quat::IDENTITY,
            },
        });
        document
    }

    fn all_resources() -> ResourceSelection {
        ResourceSelection {
            stroke_ids: HashSet::from([11]),
            image_plane_ids: HashSet::from([21]),
            obj_reference_ids: HashSet::from([31]),
            guide_ids: HashSet::from([41]),
        }
    }

    #[test]
    fn mixed_move_preserves_ids_and_can_be_restored_exactly() {
        let mut document = fixture();
        let original = capture_state(&document, &all_resources()).unwrap();
        let move_edit = MoveSnapshot::capture(
            &document,
            all_resources(),
            TransformMode::Translate,
            Vec3::new(2.0, -1.0, 0.5),
        )
        .unwrap();
        assert_eq!(
            capture_state(&document, &all_resources()).unwrap(),
            original
        );
        move_edit
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        assert_eq!(document.strokes[0].id, 11);
        assert_eq!(document.image_planes[0].id, 21);
        assert_eq!(document.model_references[0].id, 31);
        assert_eq!(document.guide_resources[0].id, 41);
        assert_eq!(
            document.strokes[0].points[0].position,
            Vec3::new(3.0, 1.0, 3.5)
        );
        assert_eq!(document.image_planes[0].origin, Vec3::new(3.0, -1.0, 0.5));
        assert!(document.image_planes[0].drawable);
        assert_eq!(
            document.model_references[0].center,
            Vec3::new(2.0, 0.0, 0.5)
        );
        assert_eq!(
            document.guide_resources[0].geometry,
            GuideGeometry::Plane {
                origin: Vec3::new(2.0, -1.0, 1.5),
                rotation: Quat::IDENTITY,
            }
        );
        let source_stroke = &original.strokes[0];
        let moved_stroke = &move_edit.after.strokes[0];
        assert_eq!(moved_stroke.id, source_stroke.id);
        assert_eq!(moved_stroke.brush, source_stroke.brush);
        assert_eq!(moved_stroke.material, source_stroke.material);
        assert_eq!(moved_stroke.color, source_stroke.color);
        for (moved_point, source_point) in moved_stroke.points.iter().zip(&source_stroke.points) {
            assert_eq!(
                moved_point.position,
                source_point.position + Vec3::new(2.0, -1.0, 0.5)
            );
            assert_eq!(moved_point.pressure, source_point.pressure);
            assert_eq!(moved_point.tilt, source_point.tilt);
        }
        move_edit
            .apply(&mut document, MoveDirection::Restore)
            .unwrap();
        assert_eq!(
            capture_state(&document, &all_resources()).unwrap(),
            original
        );
    }

    #[test]
    fn hidden_or_locked_selected_resources_are_rejected() {
        let mut document = fixture();
        document.image_planes[0].visible = false;
        assert!(matches!(
            MoveSnapshot::capture(
                &document,
                ResourceSelection {
                    image_plane_ids: HashSet::from([21]),
                    ..Default::default()
                },
                TransformMode::Translate,
                Vec3::X,
            ),
            Err(MoveError::ProtectedResource {
                kind: "image plane",
                id: 21
            })
        ));

        document.image_planes[0].visible = true;
        document.model_references[0].locked = true;
        assert!(matches!(
            MoveSnapshot::capture(
                &document,
                ResourceSelection {
                    obj_reference_ids: HashSet::from([31]),
                    ..Default::default()
                },
                TransformMode::Translate,
                Vec3::X,
            ),
            Err(MoveError::ProtectedResource {
                kind: "OBJ reference",
                id: 31
            })
        ));

        document.model_references[0].locked = false;
        document.layers.push(Layer {
            id: 2,
            name: "Hidden".into(),
            visible: false,
            locked: false,
        });
        document.strokes[0].layer_id = 2;
        assert!(matches!(
            MoveSnapshot::capture(
                &document,
                ResourceSelection {
                    stroke_ids: HashSet::from([11]),
                    ..Default::default()
                },
                TransformMode::Translate,
                Vec3::X,
            ),
            Err(MoveError::ProtectedResource {
                kind: "stroke",
                id: 11
            })
        ));
    }

    #[test]
    fn bounds_failure_does_not_mutate_document() {
        let mut document = fixture();
        document.strokes[0].points[0].position.x = 9_999.5;
        let before = capture_state(&document, &all_resources()).unwrap();
        assert!(matches!(
            MoveSnapshot::capture(
                &document,
                all_resources(),
                TransformMode::Translate,
                Vec3::X,
            ),
            Err(MoveError::InvalidDocument(_))
        ));
        assert_eq!(capture_state(&document, &all_resources()).unwrap(), before);
    }

    #[test]
    fn active_drawable_binding_is_unchanged_by_move_and_restore() {
        let mut document = fixture();
        let active_plane = document.active_image_plane;
        let edit = MoveSnapshot::capture(
            &document,
            all_resources(),
            TransformMode::Translate,
            Vec3::new(0.25, 0.5, 0.75),
        )
        .unwrap();
        edit.apply(&mut document, MoveDirection::Forward).unwrap();
        assert_eq!(document.active_image_plane, active_plane);
        edit.apply(&mut document, MoveDirection::Restore).unwrap();
        assert_eq!(document.active_image_plane, active_plane);
    }

    #[test]
    fn mixed_resource_history_edit_undoes_and_redoes_all_resource_kinds() {
        let mut document = fixture();
        let before = capture_state(&document, &all_resources()).unwrap();
        let movement = MoveSnapshot::capture(
            &document,
            all_resources(),
            TransformMode::Translate,
            Vec3::new(0.5, 0.25, -0.75),
        )
        .unwrap();
        movement
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        let after = movement.after.clone();
        let edit = crate::editing::Edit::MixedResources { before, after };

        edit.apply(&mut document, false);
        assert_eq!(
            capture_state(&document, &all_resources()).unwrap(),
            edit_before(&edit)
        );
        edit.apply(&mut document, true);
        assert_eq!(
            capture_state(&document, &all_resources()).unwrap(),
            edit_after(&edit)
        );
    }

    #[test]
    fn native_and_recovery_roundtrips_preserve_moved_resource_records() {
        let mut document = fixture();
        let movement = MoveSnapshot::capture(
            &document,
            all_resources(),
            TransformMode::Translate,
            Vec3::new(-0.25, 0.75, 0.5),
        )
        .unwrap();
        movement
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        let expected = capture_state(&document, &all_resources()).unwrap();

        let native_json = serde_json::to_string(&document).unwrap();
        let native = Document::from_json(&native_json).unwrap();
        assert_eq!(capture_state(&native, &all_resources()).unwrap(), expected);

        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "mixed-resource-test-session".into(),
            None,
            document,
        );
        let recovery_dir = tempfile::tempdir().unwrap();
        crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
        let scan = crate::recovery_store::scan(recovery_dir.path(), "other-session").unwrap();
        let restored = scan
            .candidates
            .into_iter()
            .find(|candidate| candidate.snapshot.session_id == "mixed-resource-test-session")
            .unwrap()
            .snapshot;
        assert_eq!(
            capture_state(&restored.document, &all_resources()).unwrap(),
            expected
        );
    }

    fn edit_before(edit: &crate::editing::Edit) -> ResourceState {
        let crate::editing::Edit::MixedResources { before, .. } = edit else {
            unreachable!()
        };
        before.clone()
    }

    fn edit_after(edit: &crate::editing::Edit) -> ResourceState {
        let crate::editing::Edit::MixedResources { after, .. } = edit else {
            unreachable!()
        };
        after.clone()
    }

    #[test]
    fn rotation_and_scale_return_explicit_unsupported_errors() {
        for mode in [TransformMode::Rotate, TransformMode::Scale] {
            assert_eq!(
                MoveSnapshot::capture(&fixture(), all_resources(), mode, Vec3::ZERO),
                Err(MoveError::UnsupportedTransform(mode))
            );
        }
    }
}
