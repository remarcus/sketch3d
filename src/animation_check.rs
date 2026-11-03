use crate::{
    animation::ArtworkAnimation,
    document::{Document, Point, Stroke},
};
use glam::Vec3;
use std::{collections::HashSet, path::Path};

pub fn run(report_path: &Path) -> Result<(), String> {
    let mut document = Document::default();
    let stroke = |x: f32| Stroke {
        model_surface: None,
        id: 1,
        layer_id: document.active_layer,
        points: vec![Point {
            position: Vec3::new(x, 0.0, 0.0),
            pressure: 1.0,
            tilt: None,
            time_ms: 0,
        }],
        color: [0.2, 0.4, 0.6],
        radius: 0.05,
        brush: Default::default(),
        fill: None,
        material: Default::default(),
        group_id: None,
        gradient: None,
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        curve: None,
    };
    let mut animation = ArtworkAnimation {
        range_end: 10,
        interpolate: true,
        ..ArtworkAnimation::default()
    };
    animation = animation
        .set_keyframe(0, &[stroke(0.0)])
        .map_err(|e| e.to_string())?;
    animation = animation
        .set_keyframe(10, &[stroke(10.0)])
        .map_err(|e| e.to_string())?;
    animation.current_frame = 5;
    document.strokes = animation.resolve(5).ok_or("Frame resolution failed")?;
    document.animation = animation;
    let layer_ids: HashSet<_> = document.layers.iter().map(|layer| layer.id).collect();
    let tip_ids: HashSet<_> = document
        .brush_tip_assets
        .iter()
        .map(|asset| asset.id)
        .collect();
    document
        .animation
        .validate(&layer_ids, &tip_ids)
        .map_err(|e| format!("Animation validation failed: {e}"))?;
    if document.strokes[0].points[0].position.x != 5.0 {
        return Err("Deterministic integer frame resolution returned the wrong pose".into());
    }
    let repeated_pose = document
        .animation
        .resolve(5)
        .ok_or("Repeated seek failed")?;
    if repeated_pose != document.strokes || document.animation.current_frame != 5 {
        return Err("Repeated seeking changed the resolved pose or frame".into());
    }
    let serialized = serde_json::to_string(&document).map_err(|e| e.to_string())?;
    let reopened = Document::from_json(&serialized)?;
    if reopened.animation != document.animation || reopened.strokes != document.strokes {
        return Err("Native animation round trip changed the current pose or keyframes".into());
    }
    let recovery = crate::recovery_store::RecoverySnapshot::new(
        "animation-verifier".into(),
        None,
        reopened.clone(),
    );
    let recovery_json = serde_json::to_string(&recovery).map_err(|e| e.to_string())?;
    let recovered: crate::recovery_store::RecoverySnapshot =
        serde_json::from_str(&recovery_json).map_err(|e| e.to_string())?;
    if recovered.document.animation != reopened.animation
        || recovered.document.strokes != reopened.strokes
    {
        return Err("Recovery snapshot lost the saved animation track or current pose".into());
    }

    let before = crate::editing::ArtworkAnimationState {
        animation: reopened.animation.clone(),
        strokes: reopened.strokes.clone(),
    };
    let after_animation = reopened
        .animation
        .set_keyframe(5, &reopened.strokes)
        .map_err(|e| e.to_string())?;
    let after = crate::editing::ArtworkAnimationState {
        animation: after_animation,
        strokes: reopened.strokes.clone(),
    };
    let history_edit = crate::editing::Edit::ArtworkAnimation { before, after };
    let mut history_document = reopened.clone();
    history_edit.apply(&mut history_document, true);
    history_edit.apply(&mut history_document, false);
    if history_document.animation != reopened.animation
        || history_document.strokes != reopened.strokes
    {
        return Err("Atomic animation history undo did not restore track and pose together".into());
    }
    let mut legacy: serde_json::Value =
        serde_json::from_str(&serialized).map_err(|e| e.to_string())?;
    legacy
        .as_object_mut()
        .ok_or("Document JSON root is not an object")?
        .remove("animation");
    legacy["version"] = serde_json::json!(24);
    let upgraded = Document::from_json(&legacy.to_string())?;
    if !upgraded.animation.is_empty() || upgraded.version != 29 {
        return Err(
            "Version 24 native migration did not create an empty version 1 animation track".into(),
        );
    }
    let report = serde_json::json!({
        "passed": true,
        "checks": ["bounded validation", "deterministic seeking", "native round trip", "recovery round trip", "atomic animation undo", "version 24 migration"],
        "keyframes": document.animation.keyframes.len(),
        "current_frame": document.animation.current_frame,
        "schema_version": reopened.version,
    });
    if let Some(parent) = report_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        report_path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
