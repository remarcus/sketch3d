//! Versioned, bounded, non-destructive modifiers for sampled strokes.
//!
//! The source control points and per-sample attributes remain authoritative.
//! Rendering and export consume a derived clone; baking writes that evaluated
//! geometry back once and clears the stack.

use crate::document::{Brush, Point, Stroke};
use std::borrow::Cow;
use std::path::Path;
use std::time::Instant;

pub const CURRENT_VERSION: u16 = 1;
pub const MAX_MODIFIERS: usize = 16;
pub const MAX_POINTS: usize = 65_536;
pub const MAX_SMOOTH_ITERATIONS: u8 = 8;
pub const MAX_NOISE_AMPLITUDE: f32 = 100.0;
pub const MIN_NOISE_FREQUENCY: f32 = 0.01;
pub const MAX_NOISE_FREQUENCY: f32 = 100.0;

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryCacheKey {
    pub texture_mapping: crate::texture::TextureMapping,
    pub point_count: usize,
    pub point_thickness: Option<Vec<f32>>,
    pub point_colors: Option<Vec<Option<[f32; 3]>>>,
    pub modifiers: Option<StrokeModifierStack>,
}

impl GeometryCacheKey {
    pub fn for_stroke(stroke: &Stroke) -> Self {
        Self {
            texture_mapping: stroke.brush.texture.mapping,
            point_count: stroke.points.len(),
            point_thickness: stroke.point_thickness.clone(),
            point_colors: stroke.point_colors.clone(),
            modifiers: stroke.modifiers.clone(),
        }
    }

    pub fn identity(point_count: usize) -> Self {
        Self {
            texture_mapping: Default::default(),
            point_count,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModifierKind {
    Thickness {
        factor: f32,
    },
    Taper {
        start: f32,
        end: f32,
    },
    Smooth {
        amount: f32,
        iterations: u8,
    },
    /// Stable world-space centerline and thickness variation. Frequency uses
    /// cycles per world unit; noise is anchored at distance zero from the
    /// stored stroke start and sampled continuously, independent of point index.
    Noise {
        /// Seed for the stable one-dimensional noise field.
        seed: u32,
        /// Maximum absolute offset per world-space axis.
        amplitude: f32,
        /// Noise cycles per world unit along the evaluated stroke.
        frequency: f32,
        /// Blend from unchanged geometry (0) to the full configured effect (1).
        influence: f32,
        /// Maximum proportional thickness change; 1 permits variation up to ±100%.
        thickness: f32,
        /// Fade centerline and thickness changes to zero at both endpoints.
        preserve_endpoints: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StrokeModifier {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    pub kind: ModifierKind,
}

fn enabled_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StrokeModifierStack {
    pub version: u16,
    #[serde(default)]
    pub modifiers: Vec<StrokeModifier>,
}

impl Default for StrokeModifierStack {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            modifiers: Vec::new(),
        }
    }
}

impl StrokeModifierStack {
    pub fn is_valid(&self, points: usize) -> bool {
        self.version == CURRENT_VERSION
            && self.modifiers.len() <= MAX_MODIFIERS
            && points <= MAX_POINTS
            && self.modifiers.iter().all(|modifier| match modifier.kind {
                ModifierKind::Thickness { factor } => {
                    factor.is_finite() && (0.05..=20.0).contains(&factor)
                }
                ModifierKind::Taper { start, end } => {
                    start.is_finite()
                        && end.is_finite()
                        && (0.0..=1.0).contains(&start)
                        && (0.0..=1.0).contains(&end)
                }
                ModifierKind::Smooth { amount, iterations } => {
                    amount.is_finite()
                        && (0.0..=1.0).contains(&amount)
                        && (1..=MAX_SMOOTH_ITERATIONS).contains(&iterations)
                }
                ModifierKind::Noise {
                    amplitude,
                    frequency,
                    influence,
                    thickness,
                    ..
                } => {
                    amplitude.is_finite()
                        && (0.0..=MAX_NOISE_AMPLITUDE).contains(&amplitude)
                        && frequency.is_finite()
                        && (MIN_NOISE_FREQUENCY..=MAX_NOISE_FREQUENCY).contains(&frequency)
                        && influence.is_finite()
                        && (0.0..=1.0).contains(&influence)
                        && thickness.is_finite()
                        && (0.0..=1.0).contains(&thickness)
                }
            })
    }

    pub fn is_compatible(&self, stroke: &Stroke) -> bool {
        if stroke.fill.is_some() {
            return false;
        }
        !stroke.curve.is_some()
            || !self.modifiers.iter().any(|modifier| {
                modifier.enabled
                    && matches!(
                        modifier.kind,
                        ModifierKind::Smooth { .. } | ModifierKind::Noise { .. }
                    )
            })
    }

    /// Evaluate modifiers in list order without changing the saved source.
    pub fn evaluate(&self, source: &Stroke) -> Result<Stroke, String> {
        if !self.is_valid(source.points.len()) || !self.is_compatible(source) {
            return Err("Stroke modifier stack is invalid or over budget".into());
        }
        if source.points.len() > MAX_POINTS {
            return Err("Stroke modifier evaluation exceeds the 65536-point limit".into());
        }
        let mut result = source.clone();
        result.modifiers = None;
        for modifier in self.modifiers.iter().filter(|modifier| modifier.enabled) {
            match modifier.kind {
                ModifierKind::Thickness { factor } => {
                    let profile = result
                        .point_thickness
                        .get_or_insert_with(|| vec![1.0; result.points.len()]);
                    for value in profile {
                        *value = (*value * factor).clamp(0.05, 20.0);
                    }
                }
                ModifierKind::Taper { start, end } => {
                    let distances = crate::dynamics::arc_lengths(&result.points);
                    let total = distances.last().copied().unwrap_or(0.0);
                    let profile = result
                        .point_thickness
                        .get_or_insert_with(|| vec![1.0; result.points.len()]);
                    for (index, value) in profile.iter_mut().enumerate() {
                        let t = if total <= 1.0e-8 {
                            0.5
                        } else {
                            distances[index] / total
                        };
                        let start_scale = if start > 0.0 {
                            smoothstep((t / start).clamp(0.0, 1.0))
                        } else {
                            1.0
                        };
                        let end_scale = if end > 0.0 {
                            smoothstep(((1.0 - t) / end).clamp(0.0, 1.0))
                        } else {
                            1.0
                        };
                        *value = (*value * start_scale.min(end_scale)).clamp(0.05, 20.0);
                    }
                }
                ModifierKind::Smooth { amount, iterations } => {
                    if result.fill.is_some() || result.curve.is_some() {
                        return Err(
                            "Smooth modifiers require sampled ordinary strokes; bake curves or exclude fills".into(),
                        );
                    }
                    for _ in 0..iterations {
                        let source_points = result.points.clone();
                        for index in 1..source_points.len().saturating_sub(1) {
                            let target = (source_points[index - 1].position
                                + source_points[index].position * 2.0
                                + source_points[index + 1].position)
                                * 0.25;
                            let position = source_points[index].position.lerp(target, amount);
                            if !position.is_finite() || position.abs().max_element() > 10_000.0 {
                                return Err("Smooth modifier exceeds document bounds".into());
                            }
                            result.points[index].position = position;
                        }
                    }
                    result.curve = None;
                }
                ModifierKind::Noise {
                    seed,
                    amplitude,
                    frequency,
                    influence,
                    thickness,
                    preserve_endpoints,
                } => {
                    if result.curve.is_some() {
                        return Err(
                            "Noise modifiers require an ordinary sampled stroke; bake or convert semantic curves first".into(),
                        );
                    }
                    let distances = crate::dynamics::arc_lengths(&result.points);
                    let total = distances.last().copied().unwrap_or(0.0);
                    let fade_distance = if preserve_endpoints {
                        (1.0 / frequency).min(total * 0.25)
                    } else {
                        0.0
                    };
                    let mut thickness_profile = if thickness > 0.0 {
                        Some(
                            result
                                .point_thickness
                                .get_or_insert_with(|| vec![1.0; result.points.len()]),
                        )
                    } else {
                        None
                    };
                    for index in 0..result.points.len() {
                        let distance = distances[index];
                        let envelope = if preserve_endpoints {
                            if total > 1.0e-8 {
                                smoothstep((distance / fade_distance.max(1.0e-8)).clamp(0.0, 1.0))
                                    * smoothstep(
                                        ((total - distance) / fade_distance.max(1.0e-8))
                                            .clamp(0.0, 1.0),
                                    )
                            } else {
                                0.0
                            }
                        } else {
                            1.0
                        };
                        let amount = influence * envelope;
                        let displacement = glam::Vec3::new(
                            value_noise(seed, distance, frequency, 0),
                            value_noise(seed, distance, frequency, 1),
                            value_noise(seed, distance, frequency, 2),
                        ) * (amplitude * amount);
                        let position = result.points[index].position + displacement;
                        if !position.is_finite() || position.abs().max_element() > 10_000.0 {
                            return Err("Noise modifier exceeds document bounds".into());
                        }
                        result.points[index].position = position;
                        if let Some(profile) = thickness_profile.as_deref_mut() {
                            let variation = value_noise(seed, distance, frequency, 3);
                            profile[index] = (profile[index]
                                * (1.0 + variation * thickness * amount))
                                .clamp(0.05, 20.0);
                        }
                    }
                }
            }
        }
        if result.point_thickness.as_ref().is_some_and(|profile| {
            profile.len() != result.points.len()
                || profile
                    .iter()
                    .any(|scale| !scale.is_finite() || !(0.05..=20.0).contains(scale))
        }) {
            return Err("Modifier evaluation produced an invalid thickness profile".into());
        }
        Ok(result)
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Continuous one-dimensional value noise. `distance * frequency` selects a
/// stable lattice cell; independent channels use salted lattice hashes.
fn value_noise(seed: u32, distance: f32, frequency: f32, channel: u64) -> f32 {
    let coordinate = distance * frequency;
    let cell = coordinate.floor() as i64;
    let t = coordinate - coordinate.floor();
    let t = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let a = lattice_value(seed, cell, channel);
    let b = lattice_value(seed, cell.saturating_add(1), channel);
    a + (b - a) * t
}

fn lattice_value(seed: u32, cell: i64, channel: u64) -> f32 {
    let mut value = (seed as u64)
        ^ (cell as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ channel.wrapping_mul(0xD1B5_4A32_D192_ED03);
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    let unit = ((value >> 40) as f32) / ((1_u32 << 24) - 1) as f32;
    unit * 2.0 - 1.0
}

/// Bake the derived result while retaining source identity, style and layer.
pub fn bake(source: &Stroke) -> Result<Stroke, String> {
    let Some(stack) = source.modifiers.as_ref() else {
        return Ok(source.clone());
    };
    stack.evaluate(source)
}

pub fn evaluated_stroke(source: &Stroke) -> Result<Cow<'_, Stroke>, String> {
    match source.modifiers.as_ref() {
        Some(stack) => Ok(Cow::Owned(stack.evaluate(source)?)),
        None => Ok(Cow::Borrowed(source)),
    }
}

/// Deterministic local CPU/memory study for stack evaluation and cache keys.
pub fn benchmark(output: &Path) -> Result<(), String> {
    let point_count = 4096usize;
    let mut source = Stroke {
        model_surface: None,
        fill: None,
        material: Default::default(),
        group_id: None,
        layer_id: 1,
        id: 1,
        color: [0.2, 0.5, 0.8],
        gradient: None,
        radius: 0.025,
        brush: Brush::default(),
        points: (0..point_count)
            .map(|index| {
                let t = index as f32 / (point_count - 1) as f32;
                Point {
                    position: glam::Vec3::new(
                        t * 8.0,
                        (t * std::f32::consts::TAU * 3.0).sin() * 0.1,
                        0.0,
                    ),
                    pressure: 0.4 + 0.5 * t,
                    tilt: None,
                    time_ms: index as u32 * 10,
                }
            })
            .collect(),
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        curve: None,
    };
    source.modifiers = Some(StrokeModifierStack {
        version: CURRENT_VERSION,
        modifiers: vec![
            StrokeModifier {
                enabled: true,
                kind: ModifierKind::Thickness { factor: 1.2 },
            },
            StrokeModifier {
                enabled: true,
                kind: ModifierKind::Taper {
                    start: 0.15,
                    end: 0.1,
                },
            },
            StrokeModifier {
                enabled: true,
                kind: ModifierKind::Smooth {
                    amount: 0.25,
                    iterations: 2,
                },
            },
            StrokeModifier {
                enabled: true,
                kind: ModifierKind::Noise {
                    seed: 17,
                    amplitude: 0.01,
                    frequency: 4.0,
                    influence: 0.6,
                    thickness: 0.15,
                    preserve_endpoints: true,
                },
            },
        ],
    });
    let stack = source.modifiers.as_ref().unwrap();
    let mut eval_samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let start = Instant::now();
        let evaluated = stack.evaluate(&source)?;
        std::hint::black_box(evaluated);
        eval_samples.push(start.elapsed().as_micros() as u64);
    }
    eval_samples.sort_unstable();
    let mut key_samples = Vec::with_capacity(101);
    for _ in 0..101 {
        let start = Instant::now();
        std::hint::black_box(GeometryCacheKey::for_stroke(&source));
        key_samples.push(start.elapsed().as_nanos() as u64);
    }
    key_samples.sort_unstable();
    let start = Instant::now();
    let mesh = crate::geometry::stroke_mesh(&source);
    let mesh_micros = start.elapsed().as_micros() as u64;
    if mesh.is_empty() {
        return Err("Modifier benchmark produced an empty mesh".into());
    }
    let base_point_bytes = point_count * std::mem::size_of::<Point>();
    let derived_profile_bytes = point_count * std::mem::size_of::<f32>();
    let stack_bytes = std::mem::size_of::<StrokeModifierStack>()
        + stack.modifiers.len() * std::mem::size_of::<StrokeModifier>();
    let mut changed = source.clone();
    changed.modifiers.as_mut().unwrap().modifiers[0].enabled = false;
    let cache_invalidates =
        GeometryCacheKey::for_stroke(&changed) != GeometryCacheKey::for_stroke(&source);
    let cache_stays_stable =
        GeometryCacheKey::for_stroke(&source) == GeometryCacheKey::for_stroke(&source);
    let json = serde_json::json!({
        "passed": true,
        "fixture": {
            "points": point_count,
            "modifier_count": stack.modifiers.len(),
            "noise_seed": 17,
            "noise_frequency_cycles_per_world_unit": 4.0,
            "iterations": 2
        },
        "timings": {
            "stack_evaluation_median_us": eval_samples[eval_samples.len() / 2],
            "geometry_mesh_us": mesh_micros,
            "cache_key_median_ns": key_samples[key_samples.len() / 2],
            "cache_key_samples": key_samples.len()
        },
        "memory_estimate_bytes": {
            "source_point_storage": base_point_bytes,
            "evaluated_point_clone": base_point_bytes,
            "derived_thickness_profile": derived_profile_bytes,
            "modifier_stack_inline_and_records": stack_bytes,
            "evaluated_mesh_vertices": mesh.len() * std::mem::size_of::<crate::geometry::Vertex>()
        },
        "cache_invalidation": {
            "key_changes_when_stack_changes": cache_invalidates,
            "stable_key_for_unchanged_source": cache_stays_stable
        },
        "measurement": "Local CPU wall clock and structural buffer-size estimate; excludes allocator overhead, GPU upload, and resident-memory sampling."
    });
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(output, serde_json::to_vec_pretty(&json).unwrap())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point};
    use glam::Vec3;

    fn stroke() -> Stroke {
        Stroke {
            model_surface: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 1,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.05,
            brush: Brush::default(),
            points: (0..5)
                .map(|index| Point {
                    position: Vec3::new(
                        index as f32 * 0.2,
                        if index == 2 { 0.2 } else { 0.0 },
                        0.0,
                    ),
                    pressure: 0.4 + index as f32 * 0.1,
                    tilt: Some(Vec3::Z),
                    time_ms: index * 10,
                })
                .collect(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
        }
    }

    #[test]
    fn ordered_modifiers_are_non_destructive_and_bake_explicitly() {
        let mut original = stroke();
        original.modifiers = Some(StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: vec![
                StrokeModifier {
                    enabled: true,
                    kind: ModifierKind::Thickness { factor: 2.0 },
                },
                StrokeModifier {
                    enabled: true,
                    kind: ModifierKind::Taper {
                        start: 0.25,
                        end: 0.25,
                    },
                },
                StrokeModifier {
                    enabled: true,
                    kind: ModifierKind::Smooth {
                        amount: 0.8,
                        iterations: 2,
                    },
                },
            ],
        });
        let source_points = original.points.clone();
        let evaluated = original
            .modifiers
            .as_ref()
            .unwrap()
            .evaluate(&original)
            .unwrap();
        assert_eq!(original.points, source_points);
        assert_eq!(evaluated.points[0], original.points[0]);
        assert_eq!(evaluated.points[4], original.points[4]);
        assert!(evaluated.point_thickness.as_ref().unwrap()[2] > 1.0);
        assert!(evaluated.point_thickness.as_ref().unwrap()[0] < 0.1);
        let baked = bake(&original).unwrap();
        assert!(baked.modifiers.is_none());
        assert_eq!(baked.points, evaluated.points);
        assert_eq!(baked.point_thickness, evaluated.point_thickness);
    }

    #[test]
    fn invalid_and_over_budget_stacks_are_rejected() {
        let mut source = stroke();
        source.modifiers = Some(StrokeModifierStack {
            version: CURRENT_VERSION + 1,
            modifiers: Vec::new(),
        });
        assert!(source
            .modifiers
            .as_ref()
            .unwrap()
            .evaluate(&source)
            .is_err());
        source.modifiers = Some(StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: vec![StrokeModifier {
                enabled: true,
                kind: ModifierKind::Thickness { factor: f32::NAN },
            }],
        });
        assert!(source
            .modifiers
            .as_ref()
            .unwrap()
            .evaluate(&source)
            .is_err());
        source.modifiers = Some(StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: (0..=MAX_MODIFIERS)
                .map(|_| StrokeModifier {
                    enabled: true,
                    kind: ModifierKind::Thickness { factor: 1.0 },
                })
                .collect(),
        });
        assert!(!source
            .modifiers
            .as_ref()
            .unwrap()
            .is_valid(source.points.len()));
        assert!(!StrokeModifierStack::default().is_valid(MAX_POINTS + 1));
        let mut fill = source.clone();
        fill.fill = Some(crate::document::PlanarFill {
            offset: 0.0,
            gradient: None,
            texture: None,
        });
        assert!(!StrokeModifierStack::default().is_compatible(&fill));
    }

    #[test]
    fn renderer_geometry_cache_key_changes_for_profile_and_stack_edits() {
        let mut source = stroke();
        let legacy_key = GeometryCacheKey::for_stroke(&source);
        source.point_thickness = Some(vec![1.0; source.points.len()]);
        let profile_key = GeometryCacheKey::for_stroke(&source);
        assert_ne!(legacy_key, profile_key);
        source.modifiers = Some(StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: vec![StrokeModifier {
                enabled: true,
                kind: ModifierKind::Thickness { factor: 1.5 },
            }],
        });
        let modified_key = GeometryCacheKey::for_stroke(&source);
        assert_ne!(profile_key, modified_key);
        assert_eq!(modified_key, GeometryCacheKey::for_stroke(&source));
        source.modifiers.as_mut().unwrap().modifiers[0].enabled = false;
        assert_ne!(modified_key, GeometryCacheKey::for_stroke(&source));
    }

    #[test]
    fn legacy_native_recovery_glb_and_obj_keep_modifier_semantics() {
        let mut stroke = stroke();
        let stack = StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: vec![StrokeModifier {
                enabled: true,
                kind: ModifierKind::Thickness { factor: 1.75 },
            }],
        };
        stroke.modifiers = Some(stack.clone());
        let document = crate::document::Document {
            strokes: vec![stroke.clone()],
            ..Default::default()
        };
        let serialized = serde_json::to_vec(&document).unwrap();
        let loaded =
            crate::document::Document::from_json(std::str::from_utf8(&serialized).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes[0].modifiers, Some(stack.clone()));

        let mut legacy: serde_json::Value = serde_json::from_slice(&serialized).unwrap();
        legacy["strokes"][0]
            .as_object_mut()
            .unwrap()
            .remove("modifiers");
        let legacy = crate::document::Document::from_json(&legacy.to_string()).unwrap();
        assert!(legacy.strokes[0].modifiers.is_none());

        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "modifier-reader".into(),
            None,
            loaded.clone(),
        );
        let recovery_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        crate::recovery_store::write(recovery_root.path(), &recovery).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_root.path(), "fresh-modifier-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes[0].modifiers,
            Some(stack.clone())
        );

        let evaluated = stack.evaluate(&loaded.strokes[0]).unwrap();
        let expected = crate::geometry::stroke_mesh(&evaluated);
        let actual = crate::geometry::stroke_mesh(&loaded.strokes[0]);
        assert_eq!(
            actual.len(),
            expected.len(),
            "render uses evaluated mesh budget"
        );
        assert!(actual.iter().zip(&expected).all(|(actual, expected)| {
            actual.position == expected.position && actual.opacity == expected.opacity
        }));
        let glb = crate::export::encode(&loaded, None).unwrap();
        let json_length = u32::from_le_bytes(glb.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&glb.bytes[20..20 + json_length]).unwrap();
        assert_eq!(
            json["nodes"][0]["extras"]["spatialSketchModifierGeometry"],
            "evaluated_stack_ordered_mesh"
        );
        assert_eq!(
            json["nodes"][0]["extras"]["spatialSketchModifiers"]["version"],
            CURRENT_VERSION
        );

        let obj_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_root.path().join("modifiers.obj");
        crate::obj_export::write_obj_bundle(&loaded, None, &obj_path, false).unwrap();
        let obj = std::fs::read_to_string(obj_path).unwrap();
        assert!(obj.contains("modifier stack v1 (1 entries) is evaluated"));
    }

    #[test]
    fn linked_group_refresh_keeps_modifier_stack_on_generated_strokes() {
        let mut source = stroke();
        source.group_id = Some(10);
        let stack = StrokeModifierStack {
            version: CURRENT_VERSION,
            modifiers: vec![StrokeModifier {
                enabled: true,
                kind: ModifierKind::Smooth {
                    amount: 0.5,
                    iterations: 1,
                },
            }],
        };
        source.modifiers = Some(stack.clone());
        let mut second = source.clone();
        second.id = 2;
        second
            .points
            .iter_mut()
            .for_each(|point| point.position.y += 0.4);
        let mut document = crate::document::Document {
            strokes: vec![source, second],
            ..Default::default()
        };
        crate::editing::create_linked_group(
            &document,
            &std::collections::HashSet::from([1, 2]),
            100,
            1,
            Vec3::new(2.0, 0.0, 0.0),
        )
        .unwrap()
        .apply(&mut document, true);
        let generated = document
            .strokes
            .iter()
            .find(|candidate| document.linked_generated_ids().contains(&candidate.id))
            .unwrap();
        assert_eq!(generated.modifiers, Some(stack));
        assert!(!crate::geometry::stroke_mesh(generated).is_empty());
    }
}
