//! A spatial trailing stabilizer. Radius is in world units, derived from pixels.
//! Pressure is retained immediately; only the position is filtered.
use crate::document::Point;

#[derive(Default)]
pub struct Stabilizer {
    filtered: Option<Point>,
    target: Option<Point>,
}
impl Stabilizer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn push(&mut self, point: Point, radius: f32) -> Point {
        self.target = Some(point);
        let mut result = point;
        if let Some(previous) = self.filtered {
            let delta = point.position - previous.position;
            let distance = delta.length();
            if radius > 0.0 && distance > 0.0 {
                result.position = previous.position + delta * (1.0 - radius / distance).max(0.0);
            }
        }
        self.filtered = Some(result);
        result
    }
    /// Catch up to the last contact sample, without treating pen-up pressure as ink.
    pub fn finish(&mut self) -> Vec<Point> {
        let tail = match (self.filtered, self.target) {
            (Some(start), Some(target)) if start.position != target.position => (1..=8)
                .map(|i| {
                    if i == 8 {
                        target
                    } else {
                        Point {
                            position: start.position.lerp(target.position, i as f32 / 8.0),
                            pressure: start.pressure
                                + (target.pressure - start.pressure) * i as f32 / 8.0,
                        }
                    }
                })
                .collect(),
            (_, Some(target)) => vec![target],
            _ => Vec::new(),
        };
        self.reset();
        tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    #[test]
    fn zero_strength_preserves_samples_and_pressure() {
        let mut filter = Stabilizer::default();
        for i in 0..20 {
            let p = Point {
                position: Vec3::new(i as f32 * 0.1, (i as f32).sin(), 0.0),
                pressure: i as f32 / 20.0,
            };
            assert_eq!(filter.push(p, 0.0), p);
        }
        assert_eq!(filter.finish().len(), 1);
    }
    #[test]
    fn reduces_jitter_and_catches_up_to_exact_endpoint() {
        let mut filter = Stabilizer::default();
        let mut raw_y = 0.0;
        let mut smooth_y = 0.0;
        let mut raw_variation = 0.0;
        let mut smooth_variation = 0.0;
        let mut last = Point {
            position: Vec3::ZERO,
            pressure: 0.0,
        };
        for i in 0..100 {
            let p = Point {
                position: Vec3::new(
                    i as f32 * 0.01,
                    if i % 2 == 0 { 0.008 } else { -0.008 },
                    0.0,
                ),
                pressure: i as f32 / 100.0,
            };
            let smoothed = filter.push(p, 0.04);
            if i > 0 {
                raw_variation += (p.position.y - raw_y).abs();
                smooth_variation += (smoothed.position.y - smooth_y).abs();
            }
            raw_y = p.position.y;
            smooth_y = smoothed.position.y;
            assert_eq!(smoothed.pressure, p.pressure);
            last = p;
        }
        assert!(smooth_variation < raw_variation * 0.4);
        let tail = filter.finish();
        assert_eq!(tail.last(), Some(&last));
        let next = Point {
            position: Vec3::splat(5.0),
            pressure: 0.0,
        };
        assert_eq!(
            filter.push(next, 0.04),
            next,
            "Next stroke must have independent state"
        );
    }
    #[test]
    fn cancellation_does_not_leak_a_tail_into_the_next_stroke() {
        let mut filter = Stabilizer::default();
        filter.push(
            Point {
                position: Vec3::ZERO,
                pressure: 1.0,
            },
            0.1,
        );
        filter.push(
            Point {
                position: Vec3::X,
                pressure: 0.2,
            },
            0.1,
        );
        filter.reset();
        assert!(filter.finish().is_empty());
    }
}
