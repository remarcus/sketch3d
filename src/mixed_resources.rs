//! Pure-data transform transactions for selections spanning document resources.
//!
//! UI gesture capture, resource picking, and history integration belong to the
//! caller. Transform previews are built from immutable source records.

use crate::{
    document::{
        guide_resources::{GuideGeometry, GuideResource},
        model_references::ObjReferenceResource,
        Document, Stroke,
    },
    image_planes::ImagePlane,
};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Parameters for one atomic mixed-resource transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformSpec {
    pub mode: TransformMode,
    pub translation: Vec3,
    pub pivot: Vec3,
    pub rotation: Quat,
    pub scale: f32,
    pub scale_brush_width: bool,
}

impl TransformSpec {
    pub fn translate(translation: Vec3) -> Self {
        Self {
            mode: TransformMode::Translate,
            translation,
            pivot: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: 1.0,
            scale_brush_width: false,
        }
    }

    pub fn rotate(pivot: Vec3, rotation: Quat) -> Self {
        Self {
            mode: TransformMode::Rotate,
            translation: Vec3::ZERO,
            pivot,
            rotation,
            scale: 1.0,
            scale_brush_width: false,
        }
    }

    pub fn uniform_scale(pivot: Vec3, scale: f32, scale_brush_width: bool) -> Self {
        Self {
            mode: TransformMode::Scale,
            translation: Vec3::ZERO,
            pivot,
            rotation: Quat::IDENTITY,
            scale,
            scale_brush_width,
        }
    }
}

/// Exact source and transformed records. Vectors are sorted by resource ID.
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
    pub transform: TransformSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveError {
    UnsupportedTransform(TransformMode),
    EmptySelection,
    InvalidTranslation,
    InvalidTransform,
    InvalidTransformDetail(String),
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
                    "{mode:?} requires explicit mixed-resource transform parameters"
                )
            }
            Self::EmptySelection => formatter.write_str("Select at least one movable resource"),
            Self::InvalidTranslation => {
                formatter.write_str("Translation must be finite and within document bounds")
            }
            Self::InvalidTransform => {
                formatter.write_str("Rotation pivot, angle, or uniform scale is invalid")
            }
            Self::InvalidTransformDetail(error) => formatter.write_str(error),
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
    /// Capture a translation preview through the legacy move-specific API.
    pub fn capture(
        document: &Document,
        selection: ResourceSelection,
        mode: TransformMode,
        translation: Vec3,
    ) -> Result<Self, MoveError> {
        if mode != TransformMode::Translate {
            return Err(MoveError::UnsupportedTransform(mode));
        }
        Self::capture_transform(document, selection, TransformSpec::translate(translation))
    }

    /// Capture a transform preview from immutable source records. The full
    /// candidate is validated before any document record is changed.
    pub fn capture_transform(
        document: &Document,
        selection: ResourceSelection,
        transform: TransformSpec,
    ) -> Result<Self, MoveError> {
        if selection.is_empty() {
            return Err(MoveError::EmptySelection);
        }
        validate_transform(transform)?;
        let before = capture_state(document, &selection)?;
        let after = transformed_state(&before, transform)?;
        validate_candidate(document, &after)?;
        Ok(Self {
            selection,
            before,
            after,
            translation: transform.translation,
            transform,
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

fn validate_transform(transform: TransformSpec) -> Result<(), MoveError> {
    let valid_translation =
        transform.translation.is_finite() && transform.translation.abs().max_element() <= 20_000.0;
    let valid_pivot =
        transform.pivot.is_finite() && transform.pivot.abs().max_element() <= 10_000.0;
    let valid_rotation = transform.rotation.is_finite()
        && (transform.rotation.length_squared() - 1.0).abs() <= 0.001;
    let valid = match transform.mode {
        TransformMode::Translate => {
            valid_translation
                && transform.pivot == Vec3::ZERO
                && transform.rotation == Quat::IDENTITY
                && transform.scale == 1.0
        }
        TransformMode::Rotate => {
            transform.translation == Vec3::ZERO
                && valid_pivot
                && valid_rotation
                && transform.scale == 1.0
        }
        TransformMode::Scale => {
            transform.translation == Vec3::ZERO
                && valid_pivot
                && transform.rotation == Quat::IDENTITY
                && transform.scale.is_finite()
                && (0.001..=100.0).contains(&transform.scale)
        }
    };
    if valid {
        Ok(())
    } else if transform.mode == TransformMode::Translate {
        Err(MoveError::InvalidTranslation)
    } else {
        Err(MoveError::InvalidTransform)
    }
}

fn transformed_state(
    before: &ResourceState,
    transform: TransformSpec,
) -> Result<ResourceState, MoveError> {
    let mut after = before.clone();
    if transform.mode == TransformMode::Translate {
        for stroke in &mut after.strokes {
            for point in &mut stroke.points {
                point.position += transform.translation;
            }
        }
        for plane in &mut after.image_planes {
            plane.origin += transform.translation;
        }
        for model in &mut after.obj_references {
            model.center += transform.translation;
        }
        for guide in &mut after.guides {
            translate_guide(guide, transform.translation);
        }
        return Ok(after);
    }

    let rotation = transform.rotation.normalize();
    let scale = if transform.mode == TransformMode::Scale {
        transform.scale
    } else {
        1.0
    };
    if rotation == Quat::IDENTITY && scale == 1.0 {
        return Ok(after);
    }

    let stroke_snapshot: crate::editing::Snapshot =
        before.strokes.iter().cloned().enumerate().collect();
    let transformed_strokes = crate::editing::transformed_with_scale(
        &stroke_snapshot,
        transform.pivot,
        Vec3::ZERO,
        rotation,
        Vec3::splat(scale),
        Quat::IDENTITY,
        transform.scale_brush_width,
    )
    .map_err(|error| MoveError::InvalidTransformDetail(error.into()))?;
    after.strokes = transformed_strokes
        .into_iter()
        .map(|(_, stroke)| stroke)
        .collect();

    for plane in &mut after.image_planes {
        plane.origin = transform_point(plane.origin, transform.pivot, rotation, scale);
        if rotation != Quat::IDENTITY {
            plane.rotation = compose_rotation(plane.rotation, rotation);
        }
        plane.size *= scale;
    }
    for model in &mut after.obj_references {
        model.center = transform_point(model.center, transform.pivot, rotation, scale);
        if rotation != Quat::IDENTITY {
            model.rotation = compose_rotation(model.rotation, rotation);
        }
        model.scale *= scale;
    }
    for guide in &mut after.guides {
        transform_guide(guide, transform.pivot, rotation, scale);
    }
    Ok(after)
}

fn transform_point(point: Vec3, pivot: Vec3, rotation: Quat, scale: f32) -> Vec3 {
    pivot + rotation * (point - pivot) * scale
}

fn compose_rotation(local: Quat, world: Quat) -> Quat {
    (world * local).normalize()
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

fn transform_guide(guide: &mut GuideResource, pivot: Vec3, rotation: Quat, scale: f32) {
    let geometry_rotation = |current| {
        if rotation == Quat::IDENTITY {
            current
        } else {
            compose_rotation(current, rotation)
        }
    };
    match &mut guide.geometry {
        GuideGeometry::Plane {
            origin,
            rotation: current_rotation,
        } => {
            *origin = transform_point(*origin, pivot, rotation, scale);
            *current_rotation = geometry_rotation(*current_rotation);
        }
        GuideGeometry::Cylinder {
            center,
            rotation: current_rotation,
            radius,
            height,
        }
        | GuideGeometry::Cone {
            center,
            rotation: current_rotation,
            radius,
            height,
        } => {
            *center = transform_point(*center, pivot, rotation, scale);
            *current_rotation = geometry_rotation(*current_rotation);
            *radius *= scale;
            *height *= scale;
        }
        GuideGeometry::Revolved {
            center,
            rotation: current_rotation,
            profile,
        } => {
            *center = transform_point(*center, pivot, rotation, scale);
            *current_rotation = geometry_rotation(*current_rotation);
            for point in profile {
                point.height *= scale;
                point.radius *= scale;
            }
        }
        GuideGeometry::Sphere {
            center,
            rotation: current_rotation,
            radius,
        } => {
            *center = transform_point(*center, pivot, rotation, scale);
            *current_rotation = geometry_rotation(*current_rotation);
            *radius *= scale;
        }
        GuideGeometry::Ring {
            center,
            rotation: current_rotation,
            major_radius,
            minor_radius,
        } => {
            *center = transform_point(*center, pivot, rotation, scale);
            *current_rotation = geometry_rotation(*current_rotation);
            *major_radius *= scale;
            *minor_radius *= scale;
        }
        GuideGeometry::Freeform { surface } => {
            surface.center = transform_point(surface.center, pivot, rotation, scale);
            surface.rotation = geometry_rotation(surface.rotation);
            for point in &mut surface.directrix {
                *point *= scale;
            }
            surface.depth *= scale;
        }
        GuideGeometry::Loft { surface } => {
            surface.center = transform_point(surface.center, pivot, rotation, scale);
            surface.rotation = geometry_rotation(surface.rotation);
            for section in surface
                .sections
                .iter_mut()
                .take(surface.section_count as usize)
            {
                for point in &mut section.controls {
                    *point *= scale;
                }
            }
        }
    }
}

/// Return the world-space bounds used for a mixed selection's Selection Center
/// pivot. Strokes use their brush extent; finite resources include their full
/// visible bounds. Infinite planes contribute their origins.
pub fn selection_bounds(
    document: &Document,
    selection: &ResourceSelection,
    models: &HashMap<u64, crate::obj_reference::ObjModel>,
) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for stroke in document
        .strokes
        .iter()
        .filter(|stroke| selection.stroke_ids.contains(&stroke.id))
    {
        let Ok(evaluated) = crate::stroke_modifiers::evaluated_stroke(stroke) else {
            continue;
        };
        let stroke = evaluated.as_ref();
        let extent = Vec3::splat(crate::geometry::stroke_extent(stroke));
        for point in &stroke.points {
            let position = point.position
                - stroke
                    .fill
                    .map_or(Vec3::ZERO, |fill| stroke.brush.normal * fill.offset);
            include_point(&mut bounds, position - extent);
            include_point(&mut bounds, position + extent);
        }
    }
    for plane in document
        .image_planes
        .iter()
        .filter(|plane| selection.image_plane_ids.contains(&plane.id))
    {
        for x in [-0.5, 0.5] {
            for y in [-0.5, 0.5] {
                include_point(
                    &mut bounds,
                    plane.origin
                        + plane.rotation * Vec3::new(plane.size.x * x, plane.size.y * y, 0.0),
                );
            }
        }
    }
    for resource in document
        .model_references
        .iter()
        .filter(|resource| selection.obj_reference_ids.contains(&resource.id))
    {
        let Some(model) = models.get(&resource.id) else {
            include_point(&mut bounds, resource.center);
            continue;
        };
        include_oriented_local_box(
            &mut bounds,
            resource.center,
            resource.rotation,
            model.bounds[0] * resource.scale,
            model.bounds[1] * resource.scale,
        );
    }
    for guide in document
        .guide_resources
        .iter()
        .filter(|guide| selection.guide_ids.contains(&guide.id))
    {
        include_guide_bounds(&mut bounds, guide);
    }
    bounds
}

fn include_point(bounds: &mut Option<(Vec3, Vec3)>, point: Vec3) {
    if !point.is_finite() {
        return;
    }
    *bounds = Some(match *bounds {
        Some((min, max)) => (min.min(point), max.max(point)),
        None => (point, point),
    });
}

fn include_oriented_local_box(
    bounds: &mut Option<(Vec3, Vec3)>,
    center: Vec3,
    rotation: Quat,
    min: Vec3,
    max: Vec3,
) {
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                include_point(bounds, center + rotation * Vec3::new(x, y, z));
            }
        }
    }
}

fn include_guide_bounds(bounds: &mut Option<(Vec3, Vec3)>, guide: &GuideResource) {
    use GuideGeometry::*;
    match &guide.geometry {
        Plane { origin, .. } => include_point(bounds, *origin),
        Cylinder {
            center,
            rotation,
            radius,
            height,
        }
        | Cone {
            center,
            rotation,
            radius,
            height,
        } => include_oriented_local_box(
            bounds,
            *center,
            *rotation,
            Vec3::new(-radius, -height * 0.5, -radius),
            Vec3::new(*radius, *height * 0.5, *radius),
        ),
        Sphere { center, radius, .. } => {
            let extent = Vec3::splat(*radius);
            include_point(bounds, *center - extent);
            include_point(bounds, *center + extent);
        }
        Ring {
            center,
            major_radius,
            minor_radius,
            ..
        } => {
            let extent = Vec3::splat(major_radius + minor_radius);
            include_point(bounds, *center - extent);
            include_point(bounds, *center + extent);
        }
        Revolved {
            center,
            rotation,
            profile,
        } => {
            let radius = profile
                .iter()
                .map(|point| point.radius)
                .fold(0.0_f32, f32::max);
            let min_height = profile
                .iter()
                .map(|point| point.height)
                .fold(f32::INFINITY, f32::min);
            let max_height = profile
                .iter()
                .map(|point| point.height)
                .fold(f32::NEG_INFINITY, f32::max);
            include_oriented_local_box(
                bounds,
                *center,
                *rotation,
                Vec3::new(-radius, min_height, -radius),
                Vec3::new(radius, max_height, radius),
            );
        }
        Freeform { surface } => {
            let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, -surface.depth * 0.725);
            let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, surface.depth * 0.725);
            for point in surface.directrix {
                min.x = min.x.min(point.x);
                min.y = min.y.min(point.y);
                max.x = max.x.max(point.x);
                max.y = max.y.max(point.y);
            }
            include_oriented_local_box(bounds, surface.center, surface.rotation, min, max);
        }
        Loft { surface } => {
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for point in surface
                .sections
                .iter()
                .take(surface.section_count as usize)
                .flat_map(|section| section.controls)
            {
                min = min.min(point);
                max = max.max(point);
            }
            include_oriented_local_box(bounds, surface.center, surface.rotation, min, max);
        }
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
            model_surface: None,
            gradient: None,
            curve: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
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

    fn document_json(document: &Document) -> serde_json::Value {
        serde_json::to_value(document).unwrap()
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
            opacity: 1.0,
            blend_mode: crate::document::LayerBlendMode::Normal,
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
        let selection = all_resources();
        let rotation = MoveSnapshot::capture_transform(
            &document,
            selection.clone(),
            TransformSpec::rotate(Vec3::new(0.2, -0.3, 0.4), Quat::from_rotation_y(0.37)),
        )
        .unwrap();
        rotation
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        let scale = MoveSnapshot::capture_transform(
            &document,
            selection.clone(),
            TransformSpec::uniform_scale(Vec3::new(-0.1, 0.4, 0.2), 1.6, true),
        )
        .unwrap();
        scale.apply(&mut document, MoveDirection::Forward).unwrap();
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
    fn rotation_and_uniform_scale_transform_every_selected_resource_about_each_pivot() {
        let document = fixture();
        let selection = all_resources();
        let selection_center = selection_bounds(&document, &selection, &HashMap::new())
            .map(|(min, max)| (min + max) * 0.5)
            .unwrap();
        let custom_pivot = Vec3::new(-0.5, 0.75, 0.25);
        let source = capture_state(&document, &selection).unwrap();
        let cases = [
            (selection_center, "selection center"),
            (Vec3::ZERO, "world origin"),
            (custom_pivot, "custom pivot"),
        ];
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        for (pivot, label) in cases {
            for (transform, scale) in [
                (TransformSpec::rotate(pivot, rotation), 1.0),
                (TransformSpec::uniform_scale(pivot, 1.4, true), 1.4),
            ] {
                let snapshot =
                    MoveSnapshot::capture_transform(&document, selection.clone(), transform)
                        .unwrap_or_else(|error| {
                            panic!("{label} transform should validate: {error}")
                        });
                assert_eq!(snapshot.before, source, "preview source stays immutable");
                assert_eq!(
                    document_json(&document),
                    document_json(&fixture()),
                    "capture must not mutate any resource"
                );

                let orient = if transform.mode == TransformMode::Rotate {
                    rotation
                } else {
                    Quat::IDENTITY
                };
                let point = |position: Vec3| pivot + orient * (position - pivot) * scale;
                assert!(
                    snapshot.after.strokes[0].points[0]
                        .position
                        .distance(point(source.strokes[0].points[0].position))
                        < 1.0e-5
                );
                assert!(
                    snapshot.after.image_planes[0]
                        .origin
                        .distance(point(source.image_planes[0].origin))
                        < 1.0e-5
                );
                assert!(
                    snapshot.after.obj_references[0]
                        .center
                        .distance(point(source.obj_references[0].center))
                        < 1.0e-5
                );
                let (
                    GuideGeometry::Plane {
                        origin,
                        rotation: guide_rotation,
                    },
                    GuideGeometry::Plane {
                        rotation: original_guide_rotation,
                        ..
                    },
                ) = (
                    &snapshot.after.guides[0].geometry,
                    &source.guides[0].geometry,
                )
                else {
                    panic!("fixture guide is a plane")
                };
                assert!(origin.distance(point(Vec3::Z)) < 1.0e-5);
                assert!(guide_rotation
                    .abs_diff_eq((orient * *original_guide_rotation).normalize(), 1.0e-5));
                assert!(snapshot.after.image_planes[0]
                    .size
                    .abs_diff_eq(source.image_planes[0].size * scale, 1.0e-5));
                assert!(
                    (snapshot.after.obj_references[0].scale
                        - source.obj_references[0].scale * scale)
                        .abs()
                        < 1.0e-5
                );
                assert!(
                    (snapshot.after.strokes[0].radius - source.strokes[0].radius * scale).abs()
                        < 1.0e-5
                );
                assert_eq!(document.active_image_plane, Some(21));
            }
        }
    }

    #[test]
    fn mixed_transform_preview_rebuild_cancel_and_apply_are_atomic() {
        let mut document = fixture();
        let original = document.clone();
        let selection = all_resources();
        let transform =
            TransformSpec::rotate(Vec3::new(0.2, 0.4, -0.5), Quat::from_rotation_x(0.6));
        let first_preview =
            MoveSnapshot::capture_transform(&document, selection.clone(), transform).unwrap();
        first_preview
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        assert_ne!(document_json(&document), document_json(&original));
        first_preview
            .apply(&mut document, MoveDirection::Restore)
            .unwrap();
        assert_eq!(document_json(&document), document_json(&original));

        let second_preview = MoveSnapshot::capture_transform(
            &document,
            selection.clone(),
            TransformSpec::uniform_scale(Vec3::new(-0.2, 0.1, 0.4), 1.5, true),
        )
        .unwrap();
        second_preview
            .apply(&mut document, MoveDirection::Forward)
            .unwrap();
        let after = document.clone();
        let edit = crate::editing::Edit::MixedResources {
            before: second_preview.before.clone(),
            after: second_preview.after.clone(),
        };
        edit.apply(&mut document, false);
        assert_eq!(
            document_json(&document),
            document_json(&original),
            "one-step undo restores all record types"
        );
        edit.apply(&mut document, true);
        assert_eq!(
            document_json(&document),
            document_json(&after),
            "one-step redo reapplies all record types"
        );
        assert_eq!(document.active_image_plane, Some(21));
    }

    #[test]
    fn invalid_mixed_member_or_uniform_scale_is_rejected_without_mutation() {
        let document = fixture();
        let original = document.clone();
        let mut missing_member = all_resources();
        missing_member.obj_reference_ids.insert(999);
        assert!(matches!(
            MoveSnapshot::capture_transform(
                &document,
                missing_member,
                TransformSpec::uniform_scale(Vec3::ZERO, 2.0, true),
            ),
            Err(MoveError::MissingResource {
                kind: "OBJ reference",
                id: 999
            })
        ));
        assert!(matches!(
            MoveSnapshot::capture_transform(
                &document,
                all_resources(),
                TransformSpec::uniform_scale(Vec3::ZERO, 0.0, true),
            ),
            Err(MoveError::InvalidTransform)
        ));
        assert_eq!(document_json(&document), document_json(&original));

        let mut near_boundary = fixture();
        near_boundary.strokes[0].points[0].position.x = 6_000.0;
        let before = near_boundary.clone();
        assert!(matches!(
            MoveSnapshot::capture_transform(
                &near_boundary,
                all_resources(),
                TransformSpec::uniform_scale(Vec3::ZERO, 2.0, true),
            ),
            Err(MoveError::InvalidTransformDetail(_))
        ));
        assert_eq!(
            document_json(&near_boundary),
            document_json(&before),
            "candidate rejection must be atomic"
        );
    }
}
