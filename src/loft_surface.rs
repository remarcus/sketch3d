//! Bounded lofted sheet through ordered cubic Bézier section curves.
//!
//! Section control points are stored in a shared local frame. Sections must
//! progress monotonically along local Z and preserve their left-to-right
//! parameter direction. These constraints intentionally reject twisted and
//! folding lofts instead of guessing how to repair their topology.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const MIN_SECTION_SPACING: f32 = 0.02;
pub const MAX_COORDINATE: f32 = 10_000.0;
pub const MIN_TENSION: f32 = -1.0;
pub const MAX_TENSION: f32 = 1.0;
const HIT_U_STEPS: usize = 64;
const HIT_V_STEPS_PER_SPAN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoftSection {
    /// Cubic Bézier controls for one cross-section, in loft-local coordinates.
    pub controls: [Vec3; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoftSurface {
    pub center: Vec3,
    pub rotation: Quat,
    /// Ordered along local +Z. Each section uses the same left-to-right `u` direction.
    pub sections: [LoftSection; 8],
    pub section_count: u8,
    /// Bounded ease amount between adjacent sections. Zero is linear interpolation.
    pub tension: f32,
}

impl LoftSurface {
    pub fn validate(&self) -> Result<(), String> {
        if !self.center.is_finite()
            || self.center.abs().max_element() > MAX_COORDINATE
            || !self.rotation.is_finite()
            || (self.rotation.length_squared() - 1.0).abs() > 0.001
        {
            return Err("Loft transform is invalid or outside document bounds".into());
        }
        let section_count = self.section_count as usize;
        if !(2..=8).contains(&section_count) {
            return Err("A loft requires 2–8 ordered section curves".into());
        }
        if !self.tension.is_finite() || !(MIN_TENSION..=MAX_TENSION).contains(&self.tension) {
            return Err(format!(
                "Loft tension must be between {MIN_TENSION} and {MAX_TENSION}"
            ));
        }

        let mut previous_z = [0.0; 4];
        let mut previous_direction = None;
        let mut reference_tangents = [Vec3::ZERO; 5];
        for (section_index, section) in self.sections.iter().take(section_count).enumerate() {
            if section
                .controls
                .iter()
                .any(|point| !point.is_finite() || point.abs().max_element() > MAX_COORDINATE)
            {
                return Err(
                    "Loft section controls must be finite and within document bounds".into(),
                );
            }

            let chord = section.controls[3] - section.controls[0];
            let chord_length = chord.length();
            if chord_length < MIN_SECTION_SPACING {
                return Err("Loft section curves must have non-zero width".into());
            }
            let direction = chord / chord_length;
            if previous_direction.is_some_and(|old: Vec3| old.dot(direction) < 0.5) {
                return Err(
                    "Loft sections change direction too sharply; align their endpoints before applying".into(),
                );
            }

            for (control, previous) in section.controls.iter().zip(previous_z.iter_mut()) {
                let z = control.z;
                if section_index > 0 && z - *previous < MIN_SECTION_SPACING {
                    return Err(
                        "Loft sections must progress along local +Z without crossing or folding"
                            .into(),
                    );
                }
                *previous = z;
            }

            for (sample_index, u) in [0.0_f32, 0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
                let tangent = cubic_derivative(section.controls, u);
                let tangent_length = tangent.length();
                if tangent_length < 1e-5 || tangent.dot(direction) <= chord_length * 1e-5 {
                    return Err(
                        "Loft section curves must be non-degenerate and must not loop back".into(),
                    );
                }
                let tangent = tangent / tangent_length;
                if section_index == 0 {
                    reference_tangents[sample_index] = tangent;
                } else if tangent.dot(reference_tangents[sample_index]) < 0.25 {
                    return Err(
                        "Loft section parameter directions diverge and could twist the surface"
                            .into(),
                    );
                }
            }
            previous_direction = Some(direction);
        }
        Ok(())
    }

    /// Evaluate a surface point. `u` follows each cubic section; `v` spans the
    /// ordered sections from zero to one.
    pub fn point(&self, u: f32, v: f32) -> Vec3 {
        self.center + self.rotation * self.local_point(u.clamp(0.0, 1.0), v)
    }

    /// Return the world-space section tangent and surface normal at `(u, v)`.
    pub fn frame(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let (along, across) = self.local_derivatives(u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let tangent = along.normalize_or_zero();
        let normal = tangent.cross(across.normalize_or_zero()).normalize_or_zero();
        (self.rotation * tangent, self.rotation * normal)
    }

    /// Find the nearest surface coordinates for a world-space point.
    /// Returns `None` when the point is farther than a scale-aware picking tolerance.
    pub fn coordinates(&self, world: Vec3) -> Option<(f32, f32)> {
        if !world.is_finite() || self.validate().is_err() {
            return None;
        }
        let local = self.rotation.inverse() * (world - self.center);
        // At fixed u the ordered sections give a monotone local Z coordinate.
        // Start each candidate at its own section span, even on slanted lofts.
        let candidate = |u| {
            let v = self.v_at_height(u, local.z);
            (self.local_point(u, v).distance_squared(local), u, v)
        };
        let mut best = (0..=32)
            .map(|i| candidate(i as f32 / 32.0))
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let mut step = 1.0 / 32.0;
        for _ in 0..8 {
            step *= 0.5;
            for u in [(best.1 - step).max(0.0), (best.1 + step).min(1.0)] {
                let next = candidate(u);
                if next.0 < best.0 {
                    best = next;
                }
            }
        }
        // Refine against the analytic sheet so an attached stroke can recover
        // the same coordinates after successive edits. Keep the best result
        // when a step meets an edge or a change in tangent at a section seam.
        for _ in 0..12 {
            let (distance, u, v) = best;
            let residual = local - self.local_point(u, v);
            let (along, across) = self.local_derivatives(u, v);
            let uu = along.length_squared();
            let vv = across.length_squared();
            let uv = along.dot(across);
            let determinant = uu * vv - uv * uv;
            if determinant <= uu * vv * 1e-6 {
                break;
            }
            let ru = residual.dot(along);
            let rv = residual.dot(across);
            let step_u = (vv * ru - uv * rv) / determinant;
            let step_v = (uu * rv - uv * ru) / determinant;
            for amount in [1.0, 0.5, 0.25, 0.125] {
                let next_u = (u + amount * step_u).clamp(0.0, 1.0);
                let next_v = (v + amount * step_v).clamp(0.0, 1.0);
                let next_distance = self.local_point(next_u, next_v).distance_squared(local);
                if next_distance < distance {
                    best = (next_distance, next_u, next_v);
                    break;
                }
            }
            if best.0 >= distance {
                break;
            }
        }
        let scale = self.local_bounds_extent().length().max(1.0);
        (best.0.sqrt() <= scale * 0.01).then_some((best.1, best.2))
    }

    /// Nearest positive ray hit against a deterministic triangle tessellation.
    pub fn hit(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        if !origin.is_finite()
            || !direction.is_finite()
            || direction.length_squared() < 1.0e-12
            || self.validate().is_err()
        {
            return None;
        }
        let local_origin = self.rotation.inverse() * (origin - self.center);
        let local_direction = self.rotation.inverse() * direction;
        let triangles = self.triangles(
            HIT_U_STEPS,
            (self.section_count as usize - 1) * HIT_V_STEPS_PER_SPAN,
        )?;
        let mut nearest: Option<f32> = None;
        for [a, b, c] in triangles {
            let a = self.rotation.inverse() * (a - self.center);
            let b = self.rotation.inverse() * (b - self.center);
            let c = self.rotation.inverse() * (c - self.center);
            if let Some(t) = ray_triangle(local_origin, local_direction, a, b, c) {
                if nearest.is_none_or(|previous| t < previous) {
                    nearest = Some(t);
                }
            }
        }
        nearest.map(|t| origin + direction * t)
    }

    /// Return world-space triangle vertices for a bounded preview/picking mesh.
    pub fn triangles(&self, u_steps: usize, v_steps: usize) -> Option<Vec<[Vec3; 3]>> {
        if self.validate().is_err()
            || u_steps == 0
            || v_steps == 0
            || u_steps.saturating_mul(v_steps) > 4096
        {
            return None;
        }
        let mut triangles = Vec::with_capacity(u_steps * v_steps * 2);
        for iu in 0..u_steps {
            let u0 = iu as f32 / u_steps as f32;
            let u1 = (iu + 1) as f32 / u_steps as f32;
            for iv in 0..v_steps {
                let v0 = iv as f32 / v_steps as f32;
                let v1 = (iv + 1) as f32 / v_steps as f32;
                let p00 = self.point(u0, v0);
                let p10 = self.point(u1, v0);
                let p11 = self.point(u1, v1);
                let p01 = self.point(u0, v1);
                triangles.push([p00, p10, p11]);
                triangles.push([p00, p11, p01]);
            }
        }
        Some(triangles)
    }

    /// Local-space control-hull bounds, useful for resource inspection.
    pub fn bounds(&self) -> Option<[Vec3; 2]> {
        if self.validate().is_err() {
            return None;
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for section in self.sections.iter().take(self.section_count as usize) {
            for point in section.controls {
                min = min.min(point);
                max = max.max(point);
            }
        }
        Some([min, max])
    }

    fn span(&self, v: f32) -> (usize, f32) {
        let scaled = v.clamp(0.0, 1.0) * (self.section_count as usize - 1) as f32;
        let index = (scaled.floor() as usize).min(self.section_count as usize - 2);
        (index, if v >= 1.0 { 1.0 } else { scaled - index as f32 })
    }

    fn local_point(&self, u: f32, v: f32) -> Vec3 {
        let (index, fraction) = self.span(v);
        let first = cubic(self.sections[index].controls, u);
        let second = cubic(self.sections[index + 1].controls, u);
        first.lerp(second, ease(fraction, self.tension))
    }

    fn v_at_height(&self, u: f32, z: f32) -> f32 {
        let spans = usize::from(self.section_count) - 1;
        let height = |index: usize| cubic(self.sections[index].controls, u).z;
        let mut low = 0;
        let mut high = spans;
        while high - low > 1 {
            let mid = (low + high) / 2;
            if height(mid) <= z {
                low = mid;
            } else {
                high = mid;
            }
        }
        let start = height(low);
        let eased = ((z - start) / (height(high) - start)).clamp(0.0, 1.0);
        let mut t = eased;
        for _ in 0..4 {
            let derivative = 1.0 + 0.5 * self.tension * (6.0 * t * (1.0 - t) - 1.0);
            t = (t - (ease(t, self.tension) - eased) / derivative).clamp(0.0, 1.0);
        }
        (low as f32 + t) / spans as f32
    }

    fn local_derivatives(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let h = 1.0e-3;
        let u0 = (u - h).max(0.0);
        let u1 = (u + h).min(1.0);
        let v0 = (v - h).max(0.0);
        let v1 = (v + h).min(1.0);
        (
            (self.local_point(u1, v) - self.local_point(u0, v)) / (u1 - u0),
            (self.local_point(u, v1) - self.local_point(u, v0)) / (v1 - v0),
        )
    }

    fn local_bounds_extent(&self) -> Vec3 {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for section in self.sections.iter().take(self.section_count as usize) {
            for point in section.controls {
                min = min.min(point);
                max = max.max(point);
            }
        }
        max - min
    }
}

fn ease(t: f32, tension: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t + tension * 0.5 * t * (1.0 - t) * (2.0 * t - 1.0)
}

fn cubic(control: [Vec3; 4], t: f32) -> Vec3 {
    let s = 1.0 - t;
    control[0] * (s * s * s)
        + control[1] * (3.0 * s * s * t)
        + control[2] * (3.0 * s * t * t)
        + control[3] * (t * t * t)
}

fn cubic_derivative(control: [Vec3; 4], t: f32) -> Vec3 {
    let s = 1.0 - t;
    (control[1] - control[0]) * (3.0 * s * s)
        + (control[2] - control[1]) * (6.0 * s * t)
        + (control[3] - control[2]) * (3.0 * t * t)
}

fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() <= 1.0e-9 {
        return None;
    }
    let inverse = determinant.recip();
    let offset = origin - a;
    let u = offset.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = offset.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(q) * inverse;
    (t > 1.0e-6 && t.is_finite()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loft() -> LoftSurface {
        let section = |z: f32, lift: f32| LoftSection {
            controls: [
                Vec3::new(-1.0, 0.0, z),
                Vec3::new(-0.35, lift + 0.5, z + 0.03),
                Vec3::new(0.35, lift - 0.25, z - 0.02),
                Vec3::new(1.0, lift, z),
            ],
        };
        LoftSurface {
            center: Vec3::new(0.6, -0.4, 1.2),
            rotation: Quat::from_rotation_y(0.35) * Quat::from_rotation_x(-0.2),
            sections: [
                section(-1.0, 0.0),
                section(0.0, 0.4),
                section(1.1, -0.2),
                section(0.0, 0.0),
                section(0.0, 0.0),
                section(0.0, 0.0),
                section(0.0, 0.0),
                section(0.0, 0.0),
            ],
            section_count: 3,
            tension: 0.35,
        }
    }

    #[test]
    fn loft_hit_coordinates_and_internal_section_seam_are_stable() {
        let loft = loft();
        assert!(loft.validate().is_ok());
        assert_eq!(loft.triangles(64, 16).unwrap().len(), 2048);
        let (u, v) = (0.37, 0.5);
        let expected = loft.center + loft.rotation * cubic(loft.sections[1].controls, u);
        assert!(loft.point(u, v).distance(expected) < 1e-5);

        let point = loft.point(0.42, 0.31);
        let (_, normal) = loft.frame(0.42, 0.31);
        let hit = loft.hit(point + normal * 3.0, -normal).unwrap();
        assert!(hit.distance(point) < 0.06);
        let coordinates = loft.coordinates(hit).unwrap();
        assert!((coordinates.0 - 0.42).abs() < 0.02);
        assert!((coordinates.1 - 0.31).abs() < 0.02);

        let mut stabilizer = crate::surface::SurfaceStabilizer::default();
        stabilizer.start(crate::surface::Surface::Loft(loft));
        let mut drawn = Vec::new();
        for index in 0..9 {
            let v = 0.44 + index as f32 * 0.015;
            if index > 0 {
                assert!(!stabilizer.discontinuity(loft.point(0.4, v)));
            }
            drawn.extend(stabilizer.push(
                crate::document::Point {
                    position: loft.point(0.4, v),
                    pressure: 0.3 + index as f32 * 0.05,
                    tilt: None,

                    time_ms: 0,
                },
                0.08,
            ));
        }
        drawn.extend(stabilizer.finish());
        assert!(drawn
            .iter()
            .all(|point| loft.coordinates(point.position).is_some()));

        let mut twisted = loft;
        twisted.sections[2].controls.reverse();
        assert!(twisted.validate().is_err());
        let mut folded = loft;
        folded.sections[2]
            .controls
            .iter_mut()
            .for_each(|point| point.z = -1.5);
        assert!(folded.validate().is_err());
    }
}
