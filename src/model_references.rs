//! Persisted metadata for drawing-local OBJ reference models.

use crate::obj_reference_storage::ObjReferenceAsset;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_MODEL_REFERENCES: usize = 64;
pub const MAX_MODEL_NAME_CHARS: usize = 96;

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
    for resource in resources {
        resource.validate()?;
        if !ids.insert(resource.id) {
            return Err("OBJ reference IDs must be unique".into());
        }
        if !names.insert(resource.name.to_lowercase()) {
            return Err("OBJ reference names must be unique".into());
        }
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
