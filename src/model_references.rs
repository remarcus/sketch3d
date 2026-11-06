//! Persisted metadata for drawing-local OBJ reference models.

use crate::obj_reference_storage::ObjReferenceAsset;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_MODEL_REFERENCES: usize = 64;
pub const MAX_MODEL_NAME_CHARS: usize = 96;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInkAttachment {
    pub model_id: u64,
    pub anchors: Vec<ModelInkAnchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInkAnchor {
    pub triangle: u32,
    pub barycentric: Vec3,
}

impl From<crate::obj_mesh_index::ObjMeshHit> for ModelInkAnchor {
    fn from(hit: crate::obj_mesh_index::ObjMeshHit) -> Self {
        Self {
            triangle: hit.triangle_index,
            barycentric: hit.barycentric,
        }
    }
}

impl ModelInkAnchor {
    pub fn is_valid(self) -> bool {
        self.barycentric.is_finite()
            && self.barycentric.min_element() >= -1e-4
            && (self.barycentric.element_sum() - 1.0).abs() <= 1e-4
    }
}

impl ModelInkAttachment {
    pub fn is_valid(&self) -> bool {
        !self.anchors.is_empty()
            && self.anchors.len() <= 2_000_000
            && self.anchors.iter().all(|anchor| anchor.is_valid())
    }
}

impl crate::document::Stroke {
    pub fn attached_model(&self) -> Option<u64> {
        self.model_surface
            .as_ref()
            .and_then(|patch| patch.attachment.as_ref())
            .map(|attachment| attachment.model_id)
            .or_else(|| Some(self.fill.as_ref()?.mesh.as_ref()?.source?.model_id))
    }

    pub fn detach_model(&mut self) {
        if let Some(mesh) = self.fill.as_mut().and_then(|fill| fill.mesh.as_mut()) {
            mesh.source = None;
        }
        if let Some(patch) = &mut self.model_surface {
            patch.attachment = None;
            if patch.triangles.is_empty() {
                self.model_surface = None;
            }
        }
    }
}

pub fn update_attached_ink(
    document: &crate::document::Document,
    previous_models: &[ObjReferenceResource],
) -> Result<Option<Box<(crate::editing::Snapshot, crate::editing::Snapshot)>>, String> {
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (index, stroke) in document.strokes.iter().enumerate() {
        let Some(id) = stroke.attached_model() else {
            continue;
        };
        let Some(old) = previous_models.iter().find(|model| model.id == id) else {
            continue;
        };
        let mut updated = stroke.clone();
        match document
            .model_references
            .iter()
            .find(|model| model.id == id)
        {
            Some(new) if old.asset == new.asset => {
                let rotation = (new.rotation * old.rotation.inverse()).normalize();
                let scale = new.scale / old.scale;
                if old.center == new.center && old.rotation == new.rotation && scale == 1.0 {
                    continue;
                }
                let transformed = crate::editing::transformed_with_scale(
                    &vec![(index, updated)],
                    old.center,
                    new.center - old.center,
                    rotation,
                    Vec3::splat(scale),
                    Quat::IDENTITY,
                    true,
                )?;
                updated = transformed.into_iter().next().unwrap().1;
            }
            _ => updated.detach_model(),
        }
        before.push((index, stroke.clone()));
        after.push((index, updated));
    }
    Ok((!before.is_empty()).then(|| Box::new((before, after))))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjReferenceResource {
    pub id: u64,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    pub center: Vec3,
    pub rotation: Quat,
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Optional camera-dependent line-art source settings owned by this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_line_art: Option<crate::model_line_art::Settings>,
    pub asset: ObjReferenceAsset,
}

fn default_true() -> bool {
    true
}

fn default_scale() -> f32 {
    1.0
}

impl ObjReferenceResource {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.id == u64::MAX {
            return Err("OBJ reference ID is invalid".into());
        }
        let name_len = self.name.chars().count();
        if self.name.trim() != self.name
            || name_len == 0
            || name_len > MAX_MODEL_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(format!(
                "OBJ reference name must contain 1–{MAX_MODEL_NAME_CHARS} printable characters"
            ));
        }
        if !self.center.is_finite()
            || self.center.abs().max_element() > 10_000.0
            || !self.rotation.is_finite()
            || (self.rotation.length_squared() - 1.0).abs() > 0.001
        {
            return Err("OBJ reference transform is invalid or outside document bounds".into());
        }
        if !self.scale.is_finite() || !(0.001..=10_000.0).contains(&self.scale) {
            return Err("OBJ reference scale must be between 0.001 and 10000".into());
        }
        self.asset.validate()?;
        if let Some(settings) = self.live_line_art {
            settings.validate()?;
        }
        if self.asset.id != self.id {
            return Err("OBJ reference asset ID does not match its resource ID".into());
        }
        Ok(())
    }
}

pub fn validate_catalog(
    resources: &[ObjReferenceResource],
    active: Option<u64>,
) -> Result<(), String> {
    if resources.len() > MAX_MODEL_REFERENCES {
        return Err(format!(
            "A drawing can contain at most {MAX_MODEL_REFERENCES} OBJ references"
        ));
    }
    let mut ids = HashSet::with_capacity(resources.len());
    let mut names = HashSet::with_capacity(resources.len());
    let mut live_sources = 0usize;
    for resource in resources {
        resource.validate()?;
        live_sources += usize::from(resource.live_line_art.is_some());
        if !ids.insert(resource.id) {
            return Err("OBJ reference IDs must be unique".into());
        }
        if !names.insert(resource.name.to_lowercase()) {
            return Err("OBJ reference names must be unique".into());
        }
    }
    if live_sources > crate::model_line_art::MAX_LIVE_SOURCES {
        return Err(format!(
            "A drawing can contain at most {} live model line-art sources",
            crate::model_line_art::MAX_LIVE_SOURCES
        ));
    }
    if active.is_some_and(|id| {
        !resources
            .iter()
            .any(|resource| resource.id == id && resource.visible && !resource.locked)
    }) {
        return Err("Active OBJ reference must exist, be visible, and be unlocked".into());
    }
    Ok(())
}
