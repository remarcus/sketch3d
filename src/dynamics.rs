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
    pub tilt_orientation: bool,
    pub taper_start: f32,
    pub taper_end: f32,
}
impl Default for Dynamics {
    fn default() -> Self {
        Self {
            width: PressureMapping::default(),
            thickness: PressureMapping::default(),
            tilt_orientation: false,
            taper_start: 0.0,
            taper_end: 0.0,
        }
    }
}
impl Dynamics {
    pub fn is_valid(self) -> bool {
        self.width.is_valid()
            && self.thickness.is_valid()
            && self.taper_start.is_finite()
            && (0.0..=10.0).contains(&self.taper_start)
            && self.taper_end.is_finite()
            && (0.0..=10.0).contains(&self.taper_end)
    }
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
    if x.hypot(y) < 1.0_f32.to_radians().tan() {
        return None;
    }
    (right * x - up * y).try_normalize()
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
    a.lerp(b, t.clamp(0.0, 1.0)).try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(x: f32) -> Point {
        Point {
            position: Vec3::X * x,
            pressure: 1.0,
            tilt: None,
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
        assert_eq!(
            screen_tilt([Some(30.0), Some(0.0)], Vec3::X, Vec3::Y),
            Some(Vec3::X)
        );
        assert_eq!(
            screen_tilt([Some(0.0), Some(30.0)], Vec3::X, Vec3::Y),
            Some(-Vec3::Y)
        );
        assert_eq!(
            screen_tilt([Some(30.0), Some(0.0)], Vec3::Y, -Vec3::X),
            Some(Vec3::Y)
        );
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
