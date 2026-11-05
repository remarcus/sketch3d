//! Original procedural artwork made entirely from editable document strokes.
use crate::{
    document::{Document, Point, Stroke},
    geometry::Camera,
};
use glam::Vec3;
use std::{
    f32::consts::{PI, TAU},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

type Color = [f32; 3];
const WOOD: Color = [0.36, 0.17, 0.085];
const TIMBER: Color = [0.65, 0.36, 0.16];
const PAPER: Color = [0.92, 0.80, 0.57];
const ROOF: Color = [0.12, 0.27, 0.31];
const EDGE: Color = [0.31, 0.48, 0.48];
struct Artist {
    doc: Document,
}
impl Artist {
    fn new() -> Self {
        Self {
            doc: Document::default(),
        }
    }
    fn path(&mut self, positions: Vec<Vec3>, color: Color, radius: f32) {
        if positions.is_empty() {
            return;
        }
        let count = positions.len();
        self.doc.strokes.push(Stroke {
            model_surface: None,
            curve: None,
            material: Default::default(),
            brush: Default::default(),
            id: self.doc.strokes.len() as u64 + 1,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            gradient: None,
            radius,
            points: positions
                .into_iter()
                .enumerate()
                .map(|(i, position)| Point {
                    tilt: None,
                    position,
                    pressure: 0.88 + 0.08 * (i as f32 / count as f32 * PI).sin(),
                    time_ms: 0,
                })
                .collect(),
        });
    }
    fn line(&mut self, a: Vec3, b: Vec3, color: Color, radius: f32) {
        self.path(vec![a, b], color, radius);
    }
    fn loop_xz(&mut self, center: Vec3, rx: f32, rz: f32, color: Color, radius: f32) {
        self.path(
            (0..=48)
                .map(|i| {
                    let t = i as f32 / 48.0 * TAU;
                    center + Vec3::new(rx * t.cos(), 0.0, rz * t.sin())
                })
                .collect(),
            color,
            radius,
        );
    }
    fn box_outline(&mut self, min: Vec3, max: Vec3, color: Color, radius: f32) {
        let corners: [Vec3; 8] = std::array::from_fn(|i| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        });
        for i in 0..8 {
            for bit in [1, 2, 4] {
                let j = i ^ bit;
                if j > i {
                    self.line(corners[i], corners[j], color, radius);
                }
            }
        }
    }
    fn cloud(&mut self, center: Vec3, extent: Vec3, color: Color, radius: f32) {
        self.path(
            (0..=100)
                .map(|i| {
                    let t = i as f32 / 100.0;
                    let phi = t * PI;
                    let angle = t * TAU * 7.0;
                    center
                        + Vec3::new(phi.sin() * angle.cos(), phi.cos(), phi.sin() * angle.sin())
                            * extent
                })
                .collect(),
            color,
            radius,
        );
    }
}
fn roof_y(x: f32) -> f32 {
    let t = (x.abs() / 3.55).clamp(0.0, 1.0);
    4.1 - 1.55 * t.powf(0.72) + 0.28 * t.powi(5)
}

pub fn house_camera() -> Camera {
    Camera {
        yaw: 0.62,
        pitch: 0.34,
        roll: 0.0,
        projection: crate::geometry::Projection::Perspective,
        distance: 15.2,
        target: Vec3::new(0.0, 1.55, 0.6),
    }
}

pub fn pencil_house() -> Document {
    use crate::document::guide_resources::{GuideGeometry, GuideResource};
    use glam::Quat;

    let mut a = Artist::new();
    a.doc.layers[0].name = "Pastel color".into();
    let mut linework = a.doc.layers[0].clone();
    linework.id = 2;
    linework.name = "Graphite linework".into();
    a.doc.layers.push(linework);
    a.doc.active_layer = 2;
    let palette = crate::presets::Library::default();
    let pastel = palette.get(4).expect("built-in Chalk").brush;
    let graphite = crate::presets::startup_pencil();
    let ink = graphite.color;
    let wall = [0.85, 0.48, 0.29];
    let roof = [0.29, 0.48, 0.61];
    let glass = [0.50, 0.72, 0.73];
    let roof_height = |z: f32| 4.65 - z.abs() * 0.6;

    for i in 0..49 {
        let y = 0.06 + i as f32 * 0.074;
        let wobble = (i as f32 * 1.73).sin() * 0.11;
        a.line(
            Vec3::new(-2.2 + wobble, y, 1.57),
            Vec3::new(2.18, y + wobble, 1.57),
            wall,
            0.063,
        );
        a.line(
            Vec3::new(2.23, y, 1.52),
            Vec3::new(2.23, y + wobble, -1.52),
            [0.64, 0.31, 0.21],
            0.064,
        );
    }
    for i in 0..44 {
        let z = -1.8 + i as f32 * 0.084;
        let y = roof_height(z);
        let offset = (i as f32 * 2.11).cos() * 0.14;
        a.line(
            Vec3::new(-2.47 + offset, y, z),
            Vec3::new(2.49 + offset, y + 0.02, z),
            roof,
            0.071,
        );
    }
    for floor in [0.78, 2.44] {
        for x in [-1.35, 0.0, 1.35] {
            for i in 0..10 {
                let y = floor + i as f32 * 0.071;
                a.line(
                    Vec3::new(x - 0.37, y, 1.61),
                    Vec3::new(x + 0.36, y + 0.04, 1.61),
                    glass,
                    0.052,
                );
            }
        }
    }
    for (x, z, height) in [(-2.8, 1.2, 1.5), (2.8, -0.9, 1.8), (1.8, 2.1, 0.45)] {
        for i in 0..17 {
            let t = i as f32 / 16.0;
            let spread = (t * PI).sin() * 0.66;
            let center = Vec3::new(x, 0.12 + t * height, z + t * 0.13);
            a.line(
                center - Vec3::new(spread, 0.07, 0.0),
                center + Vec3::new(spread, 0.05, 0.0),
                [0.32, 0.53, 0.22],
                0.075,
            );
        }
    }
    for stroke in &mut a.doc.strokes {
        stroke.brush = pastel;
    }
    let first_ink = a.doc.strokes.len();
    a.box_outline(
        Vec3::new(-2.22, 0.0, -1.55),
        Vec3::new(2.22, 3.72, 1.55),
        ink,
        0.033,
    );
    for x in [-2.48, 2.48] {
        a.path(
            vec![
                Vec3::new(x, 3.56, -1.82),
                Vec3::new(x, 4.65, 0.0),
                Vec3::new(x, 3.56, 1.82),
            ],
            ink,
            0.038,
        );
    }
    for z in [-1.82, 0.0, 1.82] {
        let y = roof_height(z);
        a.line(Vec3::new(-2.48, y, z), Vec3::new(2.48, y, z), ink, 0.038);
    }
    for i in 0..21 {
        let x = -2.44 + i as f32 * 0.244;
        a.line(
            Vec3::new(x, 4.67, 0.0),
            Vec3::new(x + 0.07, 3.57, 1.8),
            ink,
            0.017,
        );
    }
    for y in [1.89, 2.04, 3.68] {
        a.path(
            vec![
                Vec3::new(-2.24, y, 1.60),
                Vec3::new(2.26, y, 1.60),
                Vec3::new(2.26, y, -1.57),
            ],
            ink,
            0.026,
        );
    }
    for floor in [0.76, 2.42] {
        for x in [-1.35, 0.0, 1.35] {
            let z = 1.65;
            let lo = floor;
            let hi = floor + 0.74;
            a.path(
                vec![
                    Vec3::new(x - 0.4, lo, z),
                    Vec3::new(x + 0.4, lo, z),
                    Vec3::new(x + 0.39, hi, z),
                    Vec3::new(x - 0.41, hi, z),
                    Vec3::new(x - 0.4, lo, z),
                ],
                ink,
                0.027,
            );
            a.line(Vec3::new(x, lo, z), Vec3::new(x, hi, z), ink, 0.019);
            a.line(
                Vec3::new(x - 0.42, lo + 0.35, z),
                Vec3::new(x + 0.43, lo + 0.35, z),
                ink,
                0.018,
            );
        }
    }
    let chimney = Vec3::new(1.37, 4.59, -0.53);
    for y in [4.38, 4.96, 5.03] {
        a.loop_xz(chimney.with_y(y), 0.25, 0.25, ink, 0.029);
    }
    for i in 0..12 {
        let angle = i as f32 * TAU / 12.0;
        let offset = Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.25;
        a.line(
            chimney.with_y(4.38) + offset,
            chimney.with_y(5.03) + offset,
            ink,
            0.019,
        );
    }
    for i in 0..23 {
        let z = 1.49 - i as f32 * 0.135;
        a.line(
            Vec3::new(2.26, 0.06, z),
            Vec3::new(2.26, 0.40 + (i as f32).sin() * 0.14, z - 0.18),
            ink,
            0.017,
        );
    }
    for stroke in &mut a.doc.strokes[first_ink..] {
        stroke.layer_id = 2;
        stroke.brush = graphite.brush;
    }
    a.doc.guide_resources = [
        (
            "Front wall",
            GuideGeometry::Plane {
                origin: Vec3::new(0.0, 1.9, 1.67),
                rotation: Quat::IDENTITY,
            },
        ),
        (
            "Front roof",
            GuideGeometry::Plane {
                origin: Vec3::new(0.0, 4.68, 0.0),
                rotation: Quat::from_rotation_x(0.6_f32.atan()) * Quat::from_rotation_x(-PI * 0.5),
            },
        ),
        (
            "Round chimney",
            GuideGeometry::Cylinder {
                center: chimney,
                rotation: Quat::IDENTITY,
                radius: 0.27,
                height: 0.88,
            },
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, geometry))| GuideResource {
        id: index as u64 + 1,
        name: name.into(),
        geometry,
        visible: true,
        locked: false,
    })
    .collect();
    a.doc.active_guide = Some(1);
    a.doc
}

pub fn japanese_house() -> Document {
    let mut a = Artist::new();
    // A soft brush-painted island under the house and garden.
    for i in 0..48 {
        let z = -3.7 + i as f32 * 0.19;
        let t = (z - 0.7) / 4.5;
        let half = 5.9 * (1.0 - t * t).max(0.0).sqrt();
        let green = if i % 3 == 0 {
            [0.105, 0.245, 0.16]
        } else {
            [0.085, 0.215, 0.145]
        };
        a.line(
            Vec3::new(-half, -0.04, z),
            Vec3::new(half, -0.04, z),
            green,
            0.12,
        );
    }
    a.loop_xz(
        Vec3::new(0.0, -0.01, 0.7),
        5.9,
        4.5,
        [0.21, 0.35, 0.20],
        0.035,
    );
    // Raised timber veranda: planks, edge beam and short support posts.
    for i in 0..38 {
        let z = -2.05 + i as f32 * 0.12;
        let color = if i % 2 == 0 {
            [0.53, 0.29, 0.13]
        } else {
            [0.62, 0.36, 0.17]
        };
        a.line(
            Vec3::new(-3.0, 0.46, z),
            Vec3::new(3.0, 0.46, z),
            color,
            0.074,
        );
    }
    for z in [-2.1, 2.43] {
        a.line(
            Vec3::new(-3.04, 0.36, z),
            Vec3::new(3.04, 0.36, z),
            WOOD,
            0.11,
        );
        for x in [-2.7, 0.0, 2.7] {
            a.line(Vec3::new(x, 0.02, z), Vec3::new(x, 0.42, z), WOOD, 0.10);
        }
    }
    for x in [-3.0, 3.0] {
        a.line(
            Vec3::new(x, 0.36, -2.1),
            Vec3::new(x, 0.36, 2.43),
            WOOD,
            0.10,
        );
    }
    // Opaque paper/plaster walls are overlapping ink strokes, not separate meshes.
    for i in 0..21 {
        let y = 0.84 + i as f32 * 0.085;
        for z in [-1.68, 1.68] {
            a.line(Vec3::new(-2.66, y, z), Vec3::new(2.66, y, z), PAPER, 0.055);
        }
        for x in [-2.66, 2.66] {
            a.line(
                Vec3::new(x, y, -1.68),
                Vec3::new(x, y, 1.68),
                [0.83, 0.72, 0.51],
                0.055,
            );
        }
    }
    for i in 0..4 {
        let y = 0.59 + i as f32 * 0.09;
        for z in [-1.72, 1.72] {
            a.line(Vec3::new(-2.7, y, z), Vec3::new(2.7, y, z), TIMBER, 0.06);
        }
        for x in [-2.7, 2.7] {
            a.line(Vec3::new(x, y, -1.7), Vec3::new(x, y, 1.7), TIMBER, 0.06);
        }
    }
    a.box_outline(
        Vec3::new(-2.72, 0.54, -1.74),
        Vec3::new(2.72, 2.64, 1.74),
        WOOD,
        0.07,
    );
    // Shoji-style front and back screens, with structural posts and fine lattice.
    for z in [-1.75, 1.75] {
        for x in [-2.68, -0.9, 0.9, 2.68] {
            a.line(Vec3::new(x, 0.54, z), Vec3::new(x, 2.7, z), WOOD, 0.065);
        }
        for center in [-1.8, 0.0, 1.8] {
            for offset in [-0.6, -0.3, 0.0, 0.3, 0.6] {
                a.line(
                    Vec3::new(center + offset, 0.97, z),
                    Vec3::new(center + offset, 2.54, z),
                    TIMBER,
                    0.015,
                );
            }
            for y in [1.0, 1.39, 1.78, 2.17, 2.56] {
                a.line(
                    Vec3::new(center - 0.83, y, z),
                    Vec3::new(center + 0.83, y, z),
                    TIMBER,
                    0.018,
                );
            }
        }
        a.line(
            Vec3::new(0.0, 0.9, z + z.signum() * 0.025),
            Vec3::new(0.0, 2.6, z + z.signum() * 0.025),
            WOOD,
            0.028,
        );
    }
    // Side-screen lattice, visible when orbiting around either side.
    for x in [-2.74, 2.74] {
        for z in [-1.2, -0.8, -0.4, 0.0, 0.4, 0.8, 1.2] {
            a.line(Vec3::new(x, 1.03, z), Vec3::new(x, 2.5, z), TIMBER, 0.017);
        }
        for y in [1.03, 1.4, 1.77, 2.14, 2.51] {
            a.line(Vec3::new(x, y, -1.6), Vec3::new(x, y, 1.6), TIMBER, 0.02);
        }
    }
    // Gable framing beneath the generous overhang.
    for z in [-1.79, 1.79] {
        a.line(
            Vec3::new(-2.75, 2.67, z),
            Vec3::new(2.75, 2.67, z),
            TIMBER,
            0.085,
        );
        for x in [-2.0, -1.0, 0.0, 1.0, 2.0] {
            a.line(
                Vec3::new(x, 2.68, z),
                Vec3::new(x, roof_y(x) - 0.10, z),
                WOOD,
                0.045,
            );
        }
    }
    // Curved ceramic tile runs. The eave tips lift gently upward.
    for side in [-1.0, 1.0] {
        for i in 0..34 {
            let z = -2.34 + i as f32 * 0.142;
            let color = if i % 3 == 0 { [0.16, 0.31, 0.35] } else { ROOF };
            a.path(
                (0..=36)
                    .map(|j| {
                        let x = side * j as f32 / 36.0 * 3.55;
                        Vec3::new(x, roof_y(x), z)
                    })
                    .collect(),
                color,
                0.09,
            );
        }
        for x in [0.65, 1.25, 1.9, 2.55, 3.15] {
            let x = x * side;
            a.line(
                Vec3::new(x, roof_y(x) + 0.07, -2.38),
                Vec3::new(x, roof_y(x) + 0.07, 2.4),
                EDGE,
                0.019,
            );
        }
        a.line(
            Vec3::new(side * 3.55, roof_y(3.55) - 0.04, -2.42),
            Vec3::new(side * 3.55, roof_y(3.55) - 0.04, 2.42),
            WOOD,
            0.075,
        );
    }
    for z in [-2.44, 2.44] {
        a.path(
            (0..=80)
                .map(|i| {
                    let x = -3.55 + i as f32 / 80.0 * 7.1;
                    Vec3::new(x, roof_y(x) - 0.045, z)
                })
                .collect(),
            WOOD,
            0.082,
        );
        a.path(
            (0..=80)
                .map(|i| {
                    let x = -3.55 + i as f32 / 80.0 * 7.1;
                    Vec3::new(x, roof_y(x) + 0.01, z + 0.015)
                })
                .collect(),
            TIMBER,
            0.026,
        );
    }
    a.line(
        Vec3::new(0.0, 4.17, -2.5),
        Vec3::new(0.0, 4.17, 2.5),
        EDGE,
        0.105,
    );
    // Veranda steps and a wandering stone path.
    for (i, (z, y, width)) in [(2.62, 0.29, 1.2), (2.92, 0.15, 1.4)]
        .into_iter()
        .enumerate()
    {
        for j in 0..4 {
            a.line(
                Vec3::new(-width, y, z + j as f32 * 0.07),
                Vec3::new(width, y, z + j as f32 * 0.07),
                if i == 0 { TIMBER } else { [0.36, 0.39, 0.34] },
                0.06,
            );
        }
    }
    for i in 0..5 {
        let z = 3.4 + i as f32 * 0.43;
        let x = (i as f32 * 1.6).sin() * 0.19;
        for r in [0.10, 0.22, 0.34] {
            a.loop_xz(
                Vec3::new(x, 0.045, z),
                r,
                r * 0.55,
                [0.43, 0.48, 0.42],
                0.074,
            );
        }
    }
    // An original flowering cherry tree: curved branches and pink ink canopies.
    let root = Vec3::new(-4.5, 0.0, 1.0);
    a.path(
        vec![
            root,
            root + Vec3::new(-0.18, 0.9, 0.0),
            root + Vec3::new(0.03, 1.8, 0.12),
            root + Vec3::new(0.3, 2.8, 0.05),
            root + Vec3::new(0.08, 3.5, 0.0),
        ],
        [0.30, 0.17, 0.11],
        0.12,
    );
    let blooms = [
        (-0.85, 2.95, 0.0),
        (-0.55, 3.58, -0.15),
        (0.05, 3.95, 0.0),
        (0.62, 3.56, 0.08),
        (0.95, 2.99, 0.12),
        (-0.25, 3.1, 0.55),
        (0.42, 3.4, 0.61),
        (-0.68, 3.37, 0.48),
        (0.40, 3.34, -0.55),
        (-0.25, 3.6, -0.6),
        (0.0, 2.95, -0.35),
    ];
    for (i, (x, y, z)) in blooms.into_iter().enumerate() {
        let end = root + Vec3::new(x, y, z);
        a.path(
            vec![
                root + Vec3::new(0.03, 1.65, 0.1),
                root + Vec3::new(x * 0.4, y * 0.72, z * 0.6),
                end,
            ],
            [0.39, 0.23, 0.14],
            0.045,
        );
        let color = if i % 3 == 0 {
            [0.97, 0.60, 0.62]
        } else if i % 3 == 1 {
            [0.93, 0.43, 0.52]
        } else {
            [1.0, 0.73, 0.70]
        };
        a.cloud(end, Vec3::new(0.65, 0.47, 0.57), color, 0.075);
    }
    // Low rounded shrubs and small grass gestures keep the garden grounded.
    for (center, extent) in [
        (Vec3::new(4.05, 0.48, -1.4), Vec3::new(0.95, 0.5, 0.8)),
        (Vec3::new(4.5, 0.35, -0.4), Vec3::new(0.65, 0.38, 0.65)),
        (Vec3::new(-3.7, 0.28, 2.6), Vec3::new(0.7, 0.3, 0.55)),
    ] {
        a.cloud(center, extent, [0.19, 0.37, 0.20], 0.085);
    }
    for x in [-2.3, 2.3, 3.5] {
        for i in 0..7 {
            let t = i as f32 - 3.0;
            let base = Vec3::new(x, 0.03, 3.5);
            a.path(
                vec![
                    base,
                    base + Vec3::new(t * 0.045, 0.24, 0.04),
                    base + Vec3::new(t * 0.095, 0.35 - t.abs() * 0.035, 0.12),
                ],
                [0.39, 0.51, 0.24],
                0.016,
            );
        }
    }
    // A small stone garden lantern to the right of the entrance.
    let lantern = Vec3::new(4.15, 0.0, 2.0);
    for r in [0.15, 0.28, 0.40] {
        a.loop_xz(
            lantern + Vec3::Y * 0.09,
            r,
            r * 0.8,
            [0.38, 0.43, 0.39],
            0.08,
        );
    }
    a.line(
        lantern + Vec3::Y * 0.12,
        lantern + Vec3::Y * 1.0,
        [0.42, 0.47, 0.42],
        0.115,
    );
    a.box_outline(
        lantern + Vec3::new(-0.23, 1.0, -0.23),
        lantern + Vec3::new(0.23, 1.4, 0.23),
        [0.57, 0.61, 0.52],
        0.045,
    );
    a.cloud(
        lantern + Vec3::Y * 1.2,
        Vec3::new(0.14, 0.15, 0.14),
        [1.0, 0.69, 0.25],
        0.045,
    );
    for z in [-0.4, -0.2, 0.0, 0.2, 0.4] {
        a.path(
            vec![
                lantern + Vec3::new(-0.43, 1.46, z),
                lantern + Vec3::new(0.0, 1.69, z),
                lantern + Vec3::new(0.43, 1.46, z),
            ],
            [0.35, 0.43, 0.41],
            0.10,
        );
    }
    a.line(
        lantern + Vec3::Y * 1.7,
        lantern + Vec3::Y * 1.82,
        [0.52, 0.57, 0.49],
        0.09,
    );
    // A hanging red paper lantern at the veranda.
    let red = Vec3::new(2.0, 2.02, 2.1);
    a.line(red + Vec3::Y * 0.28, red + Vec3::Y * 0.7, WOOD, 0.012);
    a.cloud(red, Vec3::new(0.23, 0.29, 0.23), [0.88, 0.19, 0.12], 0.04);
    for y in [-0.26, 0.26] {
        a.loop_xz(red + Vec3::Y * y, 0.10, 0.10, WOOD, 0.025);
    }
    a.doc
}

pub fn lantern_cylinder() -> crate::surface::Cylinder {
    crate::surface::Cylinder {
        center: Vec3::new(0.0, 1.75, 0.0),
        rotation_degrees: Vec3::ZERO,
        radius: 1.07,
        height: 2.6,
    }
}
pub fn lantern_camera() -> Camera {
    Camera {
        projection: crate::geometry::Projection::Perspective,
        yaw: 0.45,
        pitch: 0.18,
        roll: 0.0,
        distance: 7.5,
        target: Vec3::new(0.0, 1.8, 0.0),
    }
}
pub fn cylindrical_lantern() -> Document {
    let mut a = Artist::new();
    let guide = lantern_cylinder();
    for i in 0..21 {
        let y = 0.58 + i as f32 * 0.116;
        let color = if i % 2 == 0 {
            [0.89, 0.22, 0.11]
        } else {
            [0.97, 0.32, 0.15]
        };
        a.loop_xz(Vec3::new(0.0, y, 0.0), 1.0, 1.0, color, 0.07);
    }
    for y in [0.48, 3.02] {
        for i in 0..9 {
            let r = 0.06 + i as f32 * 0.13;
            a.loop_xz(Vec3::new(0.0, y, 0.0), r, r, [0.29, 0.19, 0.12], 0.075);
        }
    }
    for i in 0..16 {
        let t = i as f32 / 16.0 * TAU;
        a.line(
            Vec3::new(1.08 * t.sin(), 0.55, 1.08 * t.cos()),
            Vec3::new(1.08 * t.sin(), 2.95, 1.08 * t.cos()),
            [0.78, 0.47, 0.22],
            0.013,
        );
    }
    for y in [-0.62, 0.0, 0.62] {
        a.path(
            (0..=80)
                .map(|i| {
                    let theta = -0.75 + i as f32 / 80.0 * 2.0;
                    guide.point(theta, y + 0.13 * (theta * 4.0).sin())
                })
                .collect(),
            [1.0, 0.76, 0.29],
            0.035,
        );
    }
    a.path(
        (0..=72)
            .map(|i| {
                let t = i as f32 / 72.0 * TAU;
                Vec3::new(0.42 * t.cos(), 3.38 + 0.42 * t.sin(), 0.0)
            })
            .collect(),
        [0.67, 0.43, 0.20],
        0.045,
    );
    for r in [0.1, 0.23, 0.36, 0.49, 0.62] {
        a.loop_xz(Vec3::new(0.0, 0.12, 0.0), r, r, [0.27, 0.36, 0.34], 0.08);
    }
    a.line(
        Vec3::new(0.0, 0.14, 0.0),
        Vec3::new(0.0, 0.48, 0.0),
        [0.42, 0.26, 0.14],
        0.11,
    );
    a.doc
}

/// Write the built-in original scenes as editable, no-overwrite demo fixtures.
pub fn write_capability_fixtures(directory: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let fixtures = [
        ("garden-house", japanese_house()),
        ("cylindrical-lantern", cylindrical_lantern()),
    ];
    let mut created = Vec::with_capacity(fixtures.len());
    let result = (|| -> Result<(), String> {
        for (name, document) in fixtures {
            let destination = directory.join(format!("{name}.sketch.json"));
            let bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|error| {
                    format!("Cannot create fixture {}: {error}", destination.display())
                })?;
            created.push(destination);
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in &created {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pencil_house_round_trips_as_editable_artwork() {
        let house = pencil_house();
        let loaded = Document::from_json(&serde_json::to_string(&house).unwrap()).unwrap();
        assert_eq!(loaded.strokes, house.strokes);
        assert_eq!(loaded.guide_resources, house.guide_resources);
        for layer in &house.layers {
            assert!(loaded
                .strokes
                .iter()
                .any(|stroke| stroke.layer_id == layer.id));
        }
    }
    #[test]
    fn house_is_a_valid_editable_document() {
        let house = japanese_house();
        assert!(house.strokes.len() > 300);
        assert!(house.strokes.len() < 800);
        let loaded = Document::from_json(&serde_json::to_string(&house).unwrap()).unwrap();
        assert_eq!(loaded.strokes, house.strokes);
        assert!(house
            .strokes
            .iter()
            .any(|s| s.points.iter().any(|p| p.position.y > 4.0)));
    }
    #[test]
    fn starter_geometry_is_deterministic() {
        assert_eq!(japanese_house().strokes, japanese_house().strokes);
    }
    #[test]
    fn lantern_is_a_valid_editable_document() {
        let lantern = cylindrical_lantern();
        assert!(lantern.strokes.len() > 50);
        let loaded = Document::from_json(&serde_json::to_string(&lantern).unwrap()).unwrap();
        assert_eq!(loaded.strokes, lantern.strokes);
        assert!(lantern_cylinder().valid());
    }

    #[test]
    fn capability_fixture_export_is_editable_and_never_overwrites_existing_files() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let directory = root.path().join("fixtures");
        fs::create_dir(&directory).unwrap();
        let lantern_path = directory.join("cylindrical-lantern.sketch.json");
        fs::write(&lantern_path, b"preserve existing fixture").unwrap();

        assert!(write_capability_fixtures(&directory).is_err());
        assert!(!directory.join("garden-house.sketch.json").exists());
        assert_eq!(
            fs::read(&lantern_path).unwrap(),
            b"preserve existing fixture"
        );

        fs::remove_file(&lantern_path).unwrap();
        let paths = write_capability_fixtures(&directory).unwrap();
        assert_eq!(paths.len(), 2);
        for path in &paths {
            let text = fs::read_to_string(path).unwrap();
            let loaded = Document::from_json(&text).unwrap();
            assert!(!loaded.strokes.is_empty());
        }
        let original: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
        assert!(write_capability_fixtures(&directory).is_err());
        assert_eq!(
            paths
                .iter()
                .map(|path| fs::read(path).unwrap())
                .collect::<Vec<_>>(),
            original
        );
    }
}
