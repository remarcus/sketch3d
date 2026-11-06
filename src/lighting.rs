//! Presentation-only lighting; independent of native ink and export geometry.
use crate::{document::Document, geometry};
use glam::{Mat4, Vec3};
pub const SHADOW_SIZE: u32 = 2048;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    pub azimuth: f32,
    pub elevation: f32,
    pub intensity: f32,
    pub ambient: f32,
    pub shadows: bool,
    pub strength: f32,
    pub bias: f32,
    pub ground: bool,
    pub ground_height: f32,
    pub ground_center: [f32; 2],
    pub ground_size: f32,
    pub ground_color: [f32; 3],
}
impl Default for Lighting {
    fn default() -> Self {
        Self {
            azimuth: 21.801409,
            elevation: 33.02447,
            intensity: 0.45,
            ambient: 0.55,
            shadows: false,
            strength: 0.9,
            bias: 0.003,
            ground: false,
            ground_height: -0.15,
            ground_center: [0.0, 0.0],
            ground_size: 24.0,
            ground_color: [0.24, 0.30, 0.36],
        }
    }
}
impl Lighting {
    pub fn direction(self) -> Vec3 {
        let a = self.azimuth.to_radians();
        let e = self.elevation.clamp(10.0, 85.0).to_radians();
        Vec3::new(a.sin() * e.cos(), e.sin(), a.cos() * e.cos())
    }
    pub fn fit_ground(&mut self, doc: &Document) -> bool {
        let Some((min, max)) = document_bounds(doc) else {
            return false;
        };
        self.ground_height = min.y - 0.03;
        self.ground_center = [(min.x + max.x) * 0.5, (min.z + max.z) * 0.5];
        let reach = (max.y - self.ground_height) / self.direction().y;
        self.ground_size =
            ((max.x - min.x).max(max.z - min.z) + reach * 2.0 + 2.0).clamp(1.0, 40_000.0);
        self.ground = true;
        true
    }
}
pub fn document_bounds(doc: &Document) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for stroke in doc.strokes.iter().filter(|s| doc.visible(s)) {
        let extent = Vec3::splat(geometry::stroke_extent(stroke));
        for point in &stroke.points {
            let position = point.position
                - stroke
                    .fill
                    .as_ref()
                    .map_or(Vec3::ZERO, |fill| stroke.brush.normal * fill.offset);
            min = min.min(position - extent);
            max = max.max(position + extent);
        }
    }
    min.is_finite().then_some((min, max))
}
pub struct ShadowFit {
    pub matrix: Mat4,
    pub depth_span: f32,
}
/// World-fixed rotation and texel-snapped bounds: camera movement cannot change the fit.
pub fn shadow_fit(min: Vec3, max: Vec3, lighting: Lighting) -> ShadowFit {
    let light = lighting.direction();
    let up = if light.dot(Vec3::Y).abs() > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let view = Mat4::look_at_rh(Vec3::ZERO, -light, up);
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let p = Vec3::new(x, y, z);
                let q = view.transform_point3(p);
                lo = lo.min(q);
                hi = hi.max(q);
                if lighting.ground && y >= lighting.ground_height {
                    let ground = p - light * ((p.y - lighting.ground_height) / light.y);
                    let q = view.transform_point3(ground);
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
        }
    }
    let pad = ((hi - lo).max_element() * 0.02).max(0.05);
    lo -= Vec3::splat(pad);
    hi += Vec3::splat(pad);
    let span = (hi - lo).max(Vec3::splat(0.5));
    let texel = span.truncate() / (SHADOW_SIZE as f32 - 2.0);
    let center = ((lo + hi) * 0.5).truncate();
    let center = (center / texel).round() * texel;
    let half = span.truncate() * 0.5 + texel;
    let near = -hi.z;
    let far = near + span.z;
    ShadowFit {
        matrix: Mat4::orthographic_rh(
            center.x - half.x,
            center.x + half.x,
            center.y - half.y,
            center.y + half.y,
            near,
            far,
        ) * view,
        depth_span: span.z,
    }
}

pub fn local_shadow_fit(min: Vec3, max: Vec3, position: Vec3) -> ShadowFit {
    let center = (min + max) * 0.5;
    let mut direction = (center - position).normalize_or_zero();
    if direction.length_squared() < 1.0e-6 { direction = Vec3::Z; }
    let up = if direction.dot(Vec3::Y).abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_to_rh(position, direction, up);
    let radius = [min.x, max.x].into_iter().flat_map(|x| [min.y, max.y].into_iter().flat_map(move |y| [min.z, max.z].into_iter().map(move |z| Vec3::new(x,y,z))))
        .map(|point| point.distance(position)).fold(0.5_f32, f32::max);
    let far = (radius * 1.2).max(0.6);
    ShadowFit {
        matrix: Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.05, far) * view,
        depth_span: far - 0.05,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_shadow_fit_is_finite_at_scene_center() {
        let bounds = Vec3::splat(2.0);
        let fit = local_shadow_fit(-bounds, bounds, Vec3::ZERO);
        assert!(fit.matrix.is_finite());
    }
    #[test]
    fn fit_contains_casters_and_ground_projections_at_extreme_angles() {
        for (min, max) in [
            (Vec3::new(-2.0, 0.0, -1.0), Vec3::new(3.0, 5.0, 2.0)),
            (Vec3::splat(9000.0), Vec3::splat(9000.01)),
            (Vec3::ZERO, Vec3::ZERO),
        ] {
            for azimuth in [-180.0, 0.0, 90.0, 180.0] {
                for elevation in [10.0, 33.0, 85.0] {
                    let settings = Lighting {
                        azimuth,
                        elevation,
                        ground: true,
                        ground_height: min.y - 0.2,
                        ..Default::default()
                    };
                    let fit = shadow_fit(min, max, settings);
                    assert!(fit.matrix.is_finite() && fit.depth_span > 0.0);
                    for x in [min.x, max.x] {
                        for y in [min.y, max.y] {
                            for z in [min.z, max.z] {
                                let p = Vec3::new(x, y, z);
                                let projected = p - settings.direction()
                                    * ((p.y - settings.ground_height) / settings.direction().y);
                                for world in [p, projected] {
                                    let q = fit.matrix.project_point3(world);
                                    assert!(
                                        q.x.abs() <= 1.01
                                            && q.y.abs() <= 1.01
                                            && q.z >= -0.01
                                            && q.z <= 1.01,
                                        "{q:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn ground_fit_uses_visible_artwork_including_locked_layers() {
        let mut doc = crate::scenes::japanese_house();
        let mut settings = Lighting::default();
        assert!(settings.fit_ground(&doc));
        let fitted = settings;
        doc.layers[0].locked = true;
        assert!(settings.fit_ground(&doc));
        assert_eq!(settings, fitted);
        doc.layers[0].visible = false;
        assert!(!settings.fit_ground(&doc));
        assert_eq!(settings, fitted);
        assert!(!settings.fit_ground(&Document::default()));
    }
}
