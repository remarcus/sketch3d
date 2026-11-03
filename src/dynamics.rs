//! Persisted brush responses and deterministic, distance-based dynamics.
use crate::document::Point;
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PressureMapping {
    pub enabled: bool,
    pub minimum: f32,
    pub curve: f32,
}
impl Default for PressureMapping {
    fn default() -> Self {
        Self {
            enabled: true,
            minimum: 0.12,
            curve: 1.0,
        }
    }
}
impl PressureMapping {
    pub fn is_valid(self) -> bool {
        self.minimum.is_finite()
            && (0.0..=1.0).contains(&self.minimum)
            && self.curve.is_finite()
            && (0.3..=3.0).contains(&self.curve)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dynamics {
    pub width: PressureMapping,
    pub thickness: PressureMapping,
    #[serde(default = "default_opacity_mapping")]
    pub opacity: PressureMapping,
    pub tilt_orientation: bool,
    #[serde(default)]
    pub tilt_shading: bool,
    #[serde(default = "default_tilt_shading_scale")]
    pub tilt_shading_scale: f32,
    #[serde(default = "default_manual_shading_scale")]
    pub manual_shading_scale: f32,
    pub taper_start: f32,
    pub taper_end: f32,
}

/// Distance-based dab controls. Distances and scatter are expressed as
/// multiples of the stroke's base radius, independent of pointer frequency.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DabSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_dab_spacing")]
    pub spacing: f32,
    #[serde(default)]
    pub scatter: f32,
    #[serde(default)]
    pub size_variation: f32,
    #[serde(default)]
    pub angle_variation_degrees: f32,
    #[serde(default = "default_dab_pressure_size")]
    pub pressure_size: PressureMapping,
    #[serde(default = "default_dab_algorithm")]
    pub algorithm_version: u8,
    /// Captured analytic target used to resolve scatter and image marks on a
    /// curved drawing guide. None preserves the legacy planar mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_target: Option<DabSurfaceTarget>,
}

/// Serializable snapshot of analytic guide geometry used by dab-based marks.
/// Model meshes and freeform guides remain unsupported by this dab stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // Captured targets stay Copy for the high-frequency dab placement path.
pub enum DabSurfaceTarget {
    Cylinder {
        center: Vec3,
        rotation: glam::Quat,
        radius: f32,
        height: f32,
    },
    Sphere {
        center: Vec3,
        rotation: glam::Quat,
        radius: f32,
    },
    Ring {
        center: Vec3,
        rotation: glam::Quat,
        major_radius: f32,
        minor_radius: f32,
    },
    Cone {
        center: Vec3,
        rotation: glam::Quat,
        radius: f32,
        height: f32,
    },
    Revolved {
        surface: crate::revolved::Revolved,
    },
    Freeform {
        surface: crate::document::guide_resources::FreeformSurface,
    },
    Loft {
        surface: crate::document::guide_resources::LoftSurface,
    },
}

impl DabSurfaceTarget {
    /// Apply a rigid or uniform-scale selection transform to the captured
    /// guide. Non-uniform transforms would change its analytic surface type.
    pub fn transformed_uniform(
        self,
        pivot: Vec3,
        linear: glam::Mat3,
        translation: Vec3,
    ) -> Option<Self> {
        let axes = [linear.x_axis, linear.y_axis, linear.z_axis];
        let lengths = axes.map(Vec3::length);
        let scale = lengths.into_iter().sum::<f32>() / 3.0;
        if !scale.is_finite()
            || scale <= 1.0e-8
            || lengths
                .iter()
                .any(|length| (*length - scale).abs() > scale * 1.0e-4)
            || axes[0].dot(axes[1]).abs() > scale * scale * 1.0e-4
            || axes[0].dot(axes[2]).abs() > scale * scale * 1.0e-4
            || axes[1].dot(axes[2]).abs() > scale * scale * 1.0e-4
            || linear.determinant() <= 0.0
        {
            return None;
        }
        let rotation = glam::Quat::from_mat3(&(linear * scale.recip())).normalize();
        let transform_point = |point: Vec3| pivot + linear * (point - pivot) + translation;
        let transform_rotation = |old: glam::Quat| (rotation * old).normalize();
        let transformed = match self {
            Self::Cylinder {
                center,
                rotation,
                radius,
                height,
            } => Self::Cylinder {
                center: transform_point(center),
                rotation: transform_rotation(rotation),
                radius: radius * scale,
                height: height * scale,
            },
            Self::Sphere {
                center,
                rotation,
                radius,
            } => Self::Sphere {
                center: transform_point(center),
                rotation: transform_rotation(rotation),
                radius: radius * scale,
            },
            Self::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => Self::Ring {
                center: transform_point(center),
                rotation: transform_rotation(rotation),
                major_radius: major_radius * scale,
                minor_radius: minor_radius * scale,
            },
            Self::Cone {
                center,
                rotation,
                radius,
                height,
            } => Self::Cone {
                center: transform_point(center),
                rotation: transform_rotation(rotation),
                radius: radius * scale,
                height: height * scale,
            },
            Self::Revolved { mut surface } => {
                surface.center = transform_point(surface.center);
                let (z, y, x) =
                    transform_rotation(surface.rotation()).to_euler(glam::EulerRot::ZYX);
                surface.rotation_degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
                for point in &mut surface.profile[..surface.count] {
                    point.height *= scale;
                    point.radius *= scale;
                }
                Self::Revolved { surface }
            }
            Self::Freeform { mut surface } => {
                surface.center = transform_point(surface.center);
                surface.rotation = transform_rotation(surface.rotation);
                surface
                    .directrix
                    .iter_mut()
                    .for_each(|point| *point *= scale);
                surface.depth *= scale;
                Self::Freeform { surface }
            }
            Self::Loft { mut surface } => {
                surface.center = transform_point(surface.center);
                surface.rotation = transform_rotation(surface.rotation);
                for section in &mut surface.sections[..surface.section_count as usize] {
                    section
                        .controls
                        .iter_mut()
                        .for_each(|point| *point *= scale);
                }
                Self::Loft { surface }
            }
        };
        transformed.is_valid().then_some(transformed)
    }

    pub fn is_valid(self) -> bool {
        let valid_transform = |center: Vec3, rotation: glam::Quat| {
            center.is_finite()
                && rotation.is_finite()
                && (rotation.length_squared() - 1.0).abs() <= 1.0e-3
        };
        match self {
            Self::Cylinder {
                center,
                rotation,
                radius,
                height,
            }
            | Self::Cone {
                center,
                rotation,
                radius,
                height,
            } => {
                valid_transform(center, rotation)
                    && radius.is_finite()
                    && radius > 0.0
                    && height.is_finite()
                    && height > 0.0
            }
            Self::Sphere {
                center,
                rotation,
                radius,
            } => valid_transform(center, rotation) && radius.is_finite() && radius > 0.0,
            Self::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => {
                valid_transform(center, rotation)
                    && major_radius.is_finite()
                    && minor_radius.is_finite()
                    && major_radius > minor_radius
                    && minor_radius > 0.0
            }
            Self::Revolved { surface } => surface.valid(),
            Self::Freeform { surface } => surface.validate().is_ok(),
            Self::Loft { surface } => surface.validate().is_ok(),
        }
    }

    pub fn surface(self) -> crate::surface::Surface {
        use crate::surface::{ConeSurface, Cylinder, RingSurface, SphereSurface, Surface};
        match self {
            Self::Cylinder {
                center,
                rotation,
                radius,
                height,
            } => {
                let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
                Surface::Cylinder(Cylinder {
                    center,
                    rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                    radius,
                    height,
                })
            }
            Self::Sphere {
                center,
                rotation,
                radius,
            } => Surface::Sphere(SphereSurface {
                center,
                rotation,
                radius,
            }),
            Self::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => Surface::Ring(RingSurface {
                center,
                rotation,
                major_radius,
                minor_radius,
            }),
            Self::Cone {
                center,
                rotation,
                radius,
                height,
            } => Surface::Cone(ConeSurface {
                center,
                rotation,
                radius,
                height,
            }),
            Self::Revolved { surface } => Surface::Revolved(surface),
            Self::Freeform { surface } => Surface::Freeform(surface),
            Self::Loft { surface } => Surface::Loft(surface),
        }
    }
}

/// Persisted layout and response of parallel bristle/rake ribbons.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrandSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_strand_count")]
    pub count: u8,
    #[serde(default = "default_strand_spread")]
    pub spread: f32,
    #[serde(default = "default_strand_thickness")]
    pub thickness: f32,
    #[serde(default = "default_strand_tilt_spread")]
    pub tilt_spread: f32,
}

/// Time-based airbrush flow. The resolved deposit geometry lives in the
/// timestamped stroke samples, not in the render/update cadence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirbrushSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_airbrush_rate")]
    pub rate_per_second: f32,
    #[serde(default = "default_airbrush_algorithm")]
    pub algorithm_version: u8,
}

impl Default for AirbrushSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            rate_per_second: default_airbrush_rate(),
            algorithm_version: default_airbrush_algorithm(),
        }
    }
}

impl AirbrushSettings {
    pub fn is_valid(self) -> bool {
        self.rate_per_second.is_finite()
            && (0.05..=30.0).contains(&self.rate_per_second)
            && self.algorithm_version == 1
    }
}

impl Default for StrandSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            count: default_strand_count(),
            spread: default_strand_spread(),
            thickness: default_strand_thickness(),
            tilt_spread: default_strand_tilt_spread(),
        }
    }
}

impl StrandSettings {
    pub fn is_valid(self) -> bool {
        (2..=16).contains(&self.count)
            && self.spread.is_finite()
            && (0.25..=2.5).contains(&self.spread)
            && self.thickness.is_finite()
            && (0.05..=1.0).contains(&self.thickness)
            && self.tilt_spread.is_finite()
            && (0.0..=1.0).contains(&self.tilt_spread)
    }

    pub fn vertex_estimate(self, points: usize) -> usize {
        let segments = points.saturating_sub(1).max(1);
        usize::from(self.count).saturating_mul(segments.saturating_mul(36).saturating_add(36))
    }
}

fn default_strand_count() -> u8 {
    8
}
fn default_strand_spread() -> f32 {
    1.4
}
fn default_strand_thickness() -> f32 {
    0.45
}
fn default_strand_tilt_spread() -> f32 {
    0.55
}
fn default_airbrush_rate() -> f32 {
    3.0
}
fn default_airbrush_algorithm() -> u8 {
    1
}
impl Default for DabSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            spacing: default_dab_spacing(),
            scatter: 0.0,
            size_variation: 0.0,
            angle_variation_degrees: 0.0,
            pressure_size: default_dab_pressure_size(),
            algorithm_version: default_dab_algorithm(),
            surface_target: None,
        }
    }
}
impl DabSettings {
    pub fn is_valid(self) -> bool {
        self.spacing.is_finite()
            && (0.1..=4.0).contains(&self.spacing)
            && self.scatter.is_finite()
            && (0.0..=4.0).contains(&self.scatter)
            && self.size_variation.is_finite()
            && (0.0..=0.95).contains(&self.size_variation)
            && self.angle_variation_degrees.is_finite()
            && (0.0..=180.0).contains(&self.angle_variation_degrees)
            && self.pressure_size.is_valid()
            && self.algorithm_version == 1
            && self.surface_target.is_none_or(DabSurfaceTarget::is_valid)
    }
}

fn default_dab_spacing() -> f32 {
    0.35
}
fn default_dab_pressure_size() -> PressureMapping {
    PressureMapping {
        enabled: false,
        minimum: 0.1,
        curve: 1.0,
    }
}
fn default_dab_algorithm() -> u8 {
    1
}
impl Default for Dynamics {
    fn default() -> Self {
        Self {
            width: PressureMapping::default(),
            thickness: PressureMapping::default(),
            opacity: default_opacity_mapping(),
            tilt_orientation: false,
            tilt_shading: false,
            tilt_shading_scale: default_tilt_shading_scale(),
            manual_shading_scale: default_manual_shading_scale(),
            taper_start: 0.0,
            taper_end: 0.0,
        }
    }
}
impl Dynamics {
    pub fn is_valid(self) -> bool {
        self.width.is_valid()
            && self.thickness.is_valid()
            && self.opacity.is_valid()
            && self.tilt_shading_scale.is_finite()
            && (1.0..=6.0).contains(&self.tilt_shading_scale)
            && self.manual_shading_scale.is_finite()
            && (1.0..=6.0).contains(&self.manual_shading_scale)
            && self.taper_start.is_finite()
            && (0.0..=10.0).contains(&self.taper_start)
            && self.taper_end.is_finite()
            && (0.0..=10.0).contains(&self.taper_end)
    }
}

fn default_opacity_mapping() -> PressureMapping {
    PressureMapping {
        enabled: false,
        minimum: 1.0,
        curve: 1.0,
    }
}

fn default_tilt_shading_scale() -> f32 {
    4.0
}

fn default_manual_shading_scale() -> f32 {
    1.5
}

/// Widen graphite/charcoal geometry by the reported lean amount. Missing tilt
/// uses a saved manual width so mouse input retains a useful shading control.
pub fn shading_width_scale(
    dynamics: Dynamics,
    texture: crate::texture::TextureKind,
    tilt: Option<Vec3>,
) -> f32 {
    if !dynamics.tilt_shading
        || !matches!(
            texture,
            crate::texture::TextureKind::Graphite | crate::texture::TextureKind::Charcoal
        )
    {
        return 1.0;
    }
    tilt.map_or(dynamics.manual_shading_scale, |lean| {
        1.0 + (dynamics.tilt_shading_scale - 1.0) * lean.length().clamp(0.0, 1.0)
    })
}

pub fn pressure_scale(mapping: PressureMapping, pressure: f32) -> f32 {
    if !mapping.enabled {
        return 1.0;
    }
    let pressure = if pressure.is_finite() {
        pressure.clamp(0.0, 1.0)
    } else {
        1.0
    };
    mapping.minimum + (1.0 - mapping.minimum) * pressure.powf(mapping.curve)
}

/// Map pen pressure to material opacity; disabled mappings preserve the base opacity.
pub fn opacity_scale(mapping: PressureMapping, pressure: f32) -> f32 {
    pressure_scale(mapping, pressure)
}

/// The envelope depends on path distance, not sample index or input frequency.
/// A tiny tip keeps closed profiles exportable. Taps retain their full footprint.
pub fn taper_factor(distance: f32, total: f32, start: f32, end: f32) -> f32 {
    if total <= 1e-8 {
        return 1.0;
    }
    let from_start = if start > 0.0 {
        (distance / start).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let from_end = if end > 0.0 {
        ((total - distance) / end).clamp(0.0, 1.0)
    } else {
        1.0
    };
    from_start.min(from_end).max(0.01)
}

pub fn arc_lengths(points: &[Point]) -> Vec<f32> {
    let mut distances = Vec::with_capacity(points.len());
    let mut distance = 0.0;
    for (index, point) in points.iter().enumerate() {
        if index > 0 {
            distance += point.position.distance(points[index - 1].position);
        }
        distances.push(distance);
    }
    distances
}

/// Windows tilt X/Y angles are relative to the screen normal; Y points down.
/// Below one degree the azimuth is ill-conditioned, so use the manual angle.
pub fn screen_tilt(tilt: [Option<f32>; 2], right: Vec3, up: Vec3) -> Option<Vec3> {
    let [Some(x), Some(y)] = tilt else {
        return None;
    };
    if !x.is_finite()
        || !y.is_finite()
        || x.abs() > 90.0
        || y.abs() > 90.0
        || !right.is_finite()
        || !up.is_finite()
    {
        return None;
    }
    let x = x.clamp(-89.0, 89.0).to_radians().tan();
    let y = y.clamp(-89.0, 89.0).to_radians().tan();
    let lean = x.hypot(y);
    if lean < 1.0_f32.to_radians().tan() {
        return None;
    }
    let magnitude = lean.atan() / std::f32::consts::FRAC_PI_2;
    (right * x - up * y)
        .try_normalize()
        .map(|direction| direction * magnitude)
}

/// Brush orientation is an axis: opposite lean vectors describe the same width.
/// Missing tilt keeps its explicit manual fallback rather than inventing input.
pub fn interpolate_tilt(a: Option<Vec3>, b: Option<Vec3>, t: f32) -> Option<Vec3> {
    let (Some(a), Some(mut b)) = (a, b) else {
        return None;
    };
    if a.dot(b) < 0.0 {
        b = -b;
    }
    let t = t.clamp(0.0, 1.0);
    let amount = a.length() + (b.length() - a.length()) * t;
    a.try_normalize()?
        .lerp(b.try_normalize()?, t)
        .try_normalize()
        .map(|direction| direction * amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(x: f32) -> Point {
        Point {
            position: Vec3::X * x,
            pressure: 1.0,
            tilt: None,
            time_ms: 0,
        }
    }
    #[test]
    fn pressure_mappings_have_independent_minima_curves_and_switches() {
        let mapping = PressureMapping {
            enabled: true,
            minimum: 0.2,
            curve: 2.0,
        };
        assert_eq!(pressure_scale(mapping, 0.0), 0.2);
        assert_eq!(pressure_scale(mapping, 1.0), 1.0);
        assert!((pressure_scale(mapping, 0.5) - 0.4).abs() < 1e-6);
        assert_eq!(pressure_scale(mapping, -1.0), 0.2);
        assert_eq!(pressure_scale(mapping, 2.0), 1.0);
        assert_eq!(
            pressure_scale(
                PressureMapping {
                    enabled: false,
                    ..mapping
                },
                0.0
            ),
            1.0
        );
        assert_eq!(pressure_scale(mapping, f32::NAN), 1.0);
        assert!(PressureMapping::default().is_valid());
        assert!(!PressureMapping {
            minimum: -0.1,
            ..mapping
        }
        .is_valid());
        assert!(!PressureMapping {
            curve: 0.0,
            ..mapping
        }
        .is_valid());
    }
    #[test]
    fn opacity_mapping_defaults_to_identity_and_uses_pressure_response_when_enabled() {
        let mapping = Dynamics::default().opacity;
        assert!(!mapping.enabled);
        assert_eq!(opacity_scale(mapping, 0.0), 1.0);
        assert_eq!(opacity_scale(mapping, 0.5), 1.0);
        let mapping = PressureMapping {
            enabled: true,
            minimum: 0.2,
            curve: 2.0,
        };
        assert_eq!(opacity_scale(mapping, 0.0), 0.2);
        assert!((opacity_scale(mapping, 0.5) - 0.4).abs() < 1e-6);
        assert_eq!(opacity_scale(mapping, 1.0), 1.0);
        assert!(mapping.is_valid());
        assert!(!PressureMapping {
            minimum: 1.1,
            ..mapping
        }
        .is_valid());
    }

    #[test]
    fn graphite_and_charcoal_use_manual_width_without_tilt_and_scaled_width_with_tilt() {
        for (texture, manual, tilt_width) in [
            (crate::texture::TextureKind::Graphite, 1.45, 3.5),
            (crate::texture::TextureKind::Charcoal, 2.1, 5.0),
        ] {
            let settings = Dynamics {
                tilt_shading: true,
                tilt_shading_scale: tilt_width,
                manual_shading_scale: manual,
                ..Default::default()
            };
            assert_eq!(shading_width_scale(settings, texture, None), manual);
            assert_eq!(
                shading_width_scale(settings, texture, Some(Vec3::ZERO)),
                1.0
            );
            assert!(
                (shading_width_scale(settings, texture, Some(Vec3::Y * 0.5))
                    - (1.0 + (tilt_width - 1.0) * 0.5))
                    .abs()
                    < 1.0e-6
            );
            assert_eq!(
                shading_width_scale(
                    Dynamics {
                        tilt_shading: false,
                        ..settings
                    },
                    texture,
                    Some(Vec3::Y)
                ),
                1.0
            );
        }
        assert_eq!(
            shading_width_scale(
                Dynamics {
                    tilt_shading: true,
                    ..Default::default()
                },
                crate::texture::TextureKind::Grain,
                Some(Vec3::Y)
            ),
            1.0,
            "older binary chalk masks do not acquire the new broad-shading behavior"
        );
    }
    #[test]
    fn taper_is_distance_based_and_handles_short_strokes_and_dots() {
        let sparse = arc_lengths(&[point(0.0), point(0.25), point(1.0)]);
        let dense = arc_lengths(&[point(0.0), point(0.1), point(0.25), point(0.5), point(1.0)]);
        let a = taper_factor(sparse[1], *sparse.last().unwrap(), 0.5, 0.4);
        let b = taper_factor(dense[2], *dense.last().unwrap(), 0.5, 0.4);
        assert_eq!(a, b);
        assert_eq!(a, 0.5);
        assert_eq!(taper_factor(0.0, 1.0, 0.0, 0.0), 1.0);
        assert_eq!(taper_factor(0.0, 1.0, 0.5, 0.5), 0.01);
        assert_eq!(taper_factor(1.0, 1.0, 0.5, 0.5), 0.01);
        assert_eq!(taper_factor(0.0, 0.0, 10.0, 10.0), 1.0);
        assert!((taper_factor(0.05, 0.1, 1.0, 1.0) - 0.05).abs() < 1e-6);
        assert!(arc_lengths(&[]).is_empty());
    }
    #[test]
    fn screen_tilt_handles_missing_vertical_invalid_and_rolled_views() {
        for tilt in [
            [None, None],
            [Some(30.0), None],
            [Some(0.0), Some(0.0)],
            [Some(0.1), Some(0.1)],
            [Some(f32::NAN), Some(0.0)],
            [Some(91.0), Some(0.0)],
        ] {
            assert_eq!(screen_tilt(tilt, Vec3::X, Vec3::Y), None);
        }
        assert!(screen_tilt([Some(30.0), Some(0.0)], Vec3::X, Vec3::Y)
            .unwrap()
            .abs_diff_eq(Vec3::X / 3.0, 1e-6));
        assert!(screen_tilt([Some(0.0), Some(30.0)], Vec3::X, Vec3::Y)
            .unwrap()
            .abs_diff_eq(-Vec3::Y / 3.0, 1e-6));
        assert!(screen_tilt([Some(30.0), Some(0.0)], Vec3::Y, -Vec3::X)
            .unwrap()
            .abs_diff_eq(Vec3::Y / 3.0, 1e-6));
        assert!(screen_tilt([Some(90.0), Some(-90.0)], Vec3::X, Vec3::Y)
            .unwrap()
            .is_finite());
        assert_eq!(interpolate_tilt(Some(Vec3::X), None, 0.5), None);
        assert_eq!(
            interpolate_tilt(Some(Vec3::X), Some(-Vec3::X), 0.5),
            Some(Vec3::X)
        );
        assert!(
            (interpolate_tilt(Some(Vec3::X), Some(Vec3::Y), 0.5)
                .unwrap()
                .length()
                - 1.0)
                .abs()
                < 1e-6
        );
    }
}
