use super::*;
use crate::{
    document::guide_resources::{
        FreeformSurface, GuideGeometry, GuideResource, LoftSection, LoftSurface,
        ProfilePoint as SavedProfilePoint,
    },
    revolved::{ProfilePoint, Revolved},
    surface::{ConeSurface, Cylinder, GuidePreview, SphereSurface, Surface},
};

type GuideEditState = (
    Vec<GuideResource>,
    Option<u64>,
    Vec<crate::image_planes::ImagePlane>,
    Option<u64>,
);

impl SketchApp {
    pub(super) fn resource_target_active(&self) -> bool {
        self.doc.active_guide.is_some() || self.doc.active_image_plane.is_some()
    }

    fn guide_edit_state(&self) -> GuideEditState {
        (
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
        )
    }

    fn commit_guide_edit(&mut self, before: GuideEditState) {
        let after = (
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
        );
        if before == after {
            return;
        }
        if self.doc.active_guide.is_some() {
            self.finish();
            self.cancel_line_tool();
            self.cancel_curve_tool();
            self.cancel_shape_draft();
            self.shape_tool.mode = None;
            self.cancel_fill_tool();
        }
        self.undo.push(Edit::Guides { before, after });
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
    }

    pub(super) fn saved_guide_geometry(&self) -> Option<&GuideGeometry> {
        self.doc
            .active_guide
            .and_then(|id| {
                self.doc
                    .guide_resources
                    .iter()
                    .find(|guide| guide.id == id && guide.visible && !guide.locked)
            })
            .map(|guide| &guide.geometry)
    }

    pub(super) fn saved_guide_surface(&self) -> Option<Surface> {
        geometry_surface(self.saved_guide_geometry()?)
    }

    pub(super) fn saved_guide_frame(&self, world: Vec3) -> Option<(Vec3, Vec3)> {
        geometry_frame(self.saved_guide_geometry()?, world)
    }

    pub(super) fn saved_guide_preview(&self) -> Option<GuidePreview> {
        self.loft_preview
            .map(GuidePreview::Loft)
            .or_else(|| self.saved_guide_geometry().and_then(geometry_preview))
    }

    pub(super) fn saved_guide_camera_frame(&self) -> Option<(Vec3, glam::Quat, f32)> {
        let guide = self.doc.guide_resources.iter().find(|guide| {
            Some(guide.id) == self.doc.active_guide && guide.visible && !guide.locked
        })?;
        Some(match &guide.geometry {
            GuideGeometry::Plane { origin, rotation } => (*origin, *rotation, 2.0),
            GuideGeometry::Cylinder {
                center,
                rotation,
                radius,
                height,
            }
            | GuideGeometry::Cone {
                center,
                rotation,
                radius,
                height,
            } => (*center, *rotation, (radius * 3.0).max(height * 0.7)),
            GuideGeometry::Sphere {
                center,
                rotation,
                radius,
            } => (*center, *rotation, radius * 3.0),
            GuideGeometry::Freeform { surface } => (
                surface.center,
                surface.rotation,
                (surface.depth * 0.8)
                    .max((surface.directrix[3].x - surface.directrix[0].x).abs() * 1.5),
            ),
            GuideGeometry::Loft { surface } => {
                let bounds = surface.bounds().unwrap_or([Vec3::ZERO, Vec3::splat(2.0)]);
                (
                    surface.center + surface.rotation * (bounds[0] + bounds[1]) * 0.5,
                    surface.rotation,
                    (bounds[1] - bounds[0]).length().max(2.0),
                )
            }
            GuideGeometry::Revolved {
                center,
                rotation,
                profile,
            } => {
                let max_radius = profile.iter().map(|p| p.radius).fold(0.0_f32, f32::max);
                let height = profile
                    .last()
                    .zip(profile.first())
                    .map_or(2.0, |(last, first)| last.height - first.height);
                (*center, *rotation, (max_radius * 3.0).max(height * 0.7))
            }
        })
    }

    pub(super) fn saved_guide_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Saved guides", |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.guide_name);
                if ui.button("Save current").clicked() { self.save_current_guide(); }
            });
            ui.horizontal(|ui| {
                if ui.button("Add sphere").clicked() {
                    self.add_saved_guide("Sphere", GuideGeometry::sphere(self.camera.target, 1.0));
                }
                if ui.button("Add cone").clicked() {
                    self.add_saved_guide("Cone", GuideGeometry::cone(self.camera.target, 1.0, 2.0));
                }
                if ui.button("Add freeform").clicked() {
                    let surface = FreeformSurface {
                        center: self.camera.target,
                        rotation: glam::Quat::IDENTITY,
                        directrix: [
                            glam::Vec2::new(-1.5, 0.0),
                            glam::Vec2::new(-0.5, 0.0),
                            glam::Vec2::new(0.5, 0.0),
                            glam::Vec2::new(1.5, 0.0),
                        ],
                        depth: 2.0,
                        bend: 0.0,
                    };
                    self.add_saved_guide("Freeform", GuideGeometry::Freeform { surface });
                }
                let eligible = self.selected_stroke_count();
                if ui
                    .add_enabled(
                        (2..=8).contains(&eligible),
                        egui::Button::new("Preview loft from selected strokes"),
                    )
                    .clicked()
                {
                    self.add_loft_from_selection();
                }
            });
            ui.small("For a loft, select 2–8 open cross-section strokes drawn in the same endpoint direction.");
            if let Some(surface) = self.loft_preview {
                ui.group(|ui| {
                    ui.strong("Loft preview");
                    ui.small(format!(
                        "{} sections, ordered along +Z; each stroke keeps its original endpoint direction.",
                        surface.section_count
                    ));
                    for section in 0..surface.section_count {
                        ui.small(format!("Section {}", section + 1));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply loft").clicked() {
                            self.loft_preview = None;
                            self.add_saved_guide("Loft", GuideGeometry::Loft { surface });
                            self.selected.clear();
                            self.status = "Loft guide added as an independent snapshot.".into();
                        }
                        if ui.button("Cancel preview").clicked() {
                            self.loft_preview = None;
                            self.renderer.invalidate();
                        }
                    });
                });
            }
            if self.doc.guide_resources.is_empty() {
                ui.small("Save a plane, cylinder, or revolved guide, or add a sphere/cone/freeform.");
            }
            let mut remove = None;
            let before = self.guide_edit_state();
            let mut changed = false;
            let mut activate = None;
            for index in 0..self.doc.guide_resources.len() {
                let id = self.doc.guide_resources[index].id;
                let mut changed_guide = false;
                ui.group(|ui| {
                    let guide = &mut self.doc.guide_resources[index];
                    let original_geometry = guide.geometry.clone();
                    ui.horizontal(|ui| {
                        ui.strong(&guide.name);
                        let mut active = self.doc.active_guide == Some(id);
                        if ui.checkbox(&mut active, "Use").changed() { activate = Some((id, active)); }
                        changed_guide |= ui.checkbox(&mut guide.visible, "Visible").changed();
                        changed_guide |= ui.checkbox(&mut guide.locked, "Locked").changed();
                        if ui.small_button("Remove").clicked() { remove = Some(index); }
                    });
                    ui.add_enabled_ui(!guide.locked, |ui| {
                        changed_guide |= edit_geometry(ui, &mut guide.geometry);
                    });
                    if changed_guide && guide.validate().is_err() {
                        guide.geometry = original_geometry;
                        changed_guide = false;
                        ui.colored_label(Color32::LIGHT_RED, "Invalid guide geometry; the last valid values were restored.");
                    }
                });
                changed |= changed_guide;
            }
            if let Some(index) = remove {
                let guide = self.doc.guide_resources.remove(index);
                if self.doc.active_guide == Some(guide.id) { self.doc.active_guide = None; }
                changed = true;
            }
            if let Some((id, true)) = activate {
                if self.doc.guide_resources.iter().any(|guide| guide.id == id && guide.visible && !guide.locked) {
                    self.doc.active_guide = Some(id);
                    self.doc.active_image_plane = None;
                    for plane in &mut self.doc.image_planes { plane.drawable = false; }
                    changed = true;
                }
            } else if let Some((id, false)) = activate {
                if self.doc.active_guide == Some(id) { self.doc.active_guide = None; changed = true; }
            }
            if let Some(id) = self.doc.active_guide {
                if self.doc.guide_resources.iter().any(|guide| guide.id == id && (!guide.visible || guide.locked)) {
                    self.doc.active_guide = None;
                    changed = true;
                }
            }
            if changed { self.commit_guide_edit(before); }
            ui.small("Saved guide transforms live in the drawing. Existing strokes never move. Ring/torus guides are not included.");
        });
        self.obj_reference_controls(ui);
    }

    fn save_current_guide(&mut self) {
        let name = if self.guide_name.trim().is_empty() {
            match (self.use_revolved, self.use_cylinder) {
                (true, _) => "Revolved",
                (_, true) => "Cylinder",
                _ => "Plane",
            }
            .to_owned()
        } else {
            self.guide_name.trim().to_owned()
        };
        let geometry = if self.use_revolved {
            revolved_geometry(self.revolved)
        } else if self.use_cylinder {
            GuideGeometry::Cylinder {
                center: self.cylinder.center,
                rotation: self.cylinder.rotation(),
                radius: self.cylinder.radius,
                height: self.cylinder.height,
            }
        } else {
            GuideGeometry::Plane {
                origin: self.guide.origin(),
                rotation: self.guide.rotation(),
            }
        };
        self.add_saved_guide(name, geometry);
    }

    fn add_saved_guide(&mut self, name: impl Into<String>, geometry: GuideGeometry) {
        if self.doc.guide_resources.len() >= crate::document::guide_resources::MAX_GUIDES {
            self.status = "This drawing already has the maximum number of saved guides.".into();
            return;
        }
        let id = self.next_id;
        if id == 0 || id == u64::MAX {
            self.status = "No more guide IDs are available.".into();
            return;
        }
        let guide = GuideResource {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            geometry,
        };
        if let Err(error) = guide.validate() {
            self.status = error;
            return;
        }
        let before = self.guide_edit_state();
        self.doc.guide_resources.push(guide);
        self.doc.active_guide = Some(id);
        self.doc.active_image_plane = None;
        for plane in &mut self.doc.image_planes {
            plane.drawable = false;
        }
        self.next_id += 1;
        self.guide_name.clear();
        self.commit_guide_edit(before);
        self.status = "Saved guide added and activated.".into();
    }

    fn selected_stroke_count(&self) -> usize {
        self.doc
            .strokes
            .iter()
            .filter(|stroke| self.selected.contains(&stroke.id) && stroke.fill.is_none())
            .count()
    }

    fn add_loft_from_selection(&mut self) {
        let mut strokes: Vec<_> = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| self.selected.contains(&stroke.id) && stroke.fill.is_none())
            .collect();
        if !(2..=8).contains(&strokes.len()) {
            self.status = "Select between two and eight open strokes for a loft.".into();
            return;
        }
        if strokes.iter().any(|stroke| stroke.points.len() < 2) {
            self.status = "Each loft section needs at least two points.".into();
            return;
        }
        let centroids: Vec<Vec3> = strokes
            .iter()
            .map(|stroke| {
                stroke
                    .points
                    .iter()
                    .map(|point| point.position)
                    .sum::<Vec3>()
                    / stroke.points.len() as f32
            })
            .collect();
        let Some(mut x_axis) = (strokes[0].points.last().unwrap().position
            - strokes[0].points[0].position)
            .try_normalize()
        else {
            self.status = "The first loft section has no usable direction.".into();
            return;
        };
        let center = centroids.iter().copied().sum::<Vec3>() / centroids.len() as f32;
        let progression = centroids.last().unwrap() - centroids[0];
        let Some(z_axis) = (progression - x_axis * progression.dot(x_axis)).try_normalize() else {
            self.status = "Loft sections need a distinct progression direction.".into();
            return;
        };
        let Some(y_axis) = z_axis.cross(x_axis).try_normalize() else {
            self.status = "Loft section and progression directions are parallel.".into();
            return;
        };
        x_axis = y_axis.cross(z_axis).normalize_or_zero();
        let rotation =
            glam::Quat::from_mat3(&glam::Mat3::from_cols(x_axis, y_axis, z_axis)).normalize();
        let inverse = rotation.inverse();
        let mut ordered: Vec<_> = strokes
            .drain(..)
            .map(|stroke| {
                let points: Vec<Vec3> = stroke
                    .points
                    .iter()
                    .map(|point| inverse * (point.position - center))
                    .collect();
                let z = points.iter().map(|point| point.z).sum::<f32>() / points.len() as f32;
                (z, points)
            })
            .collect();
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
        let sections: Vec<LoftSection> = ordered
            .iter()
            .map(|(_, points)| fit_loft_section(points))
            .collect();
        let section_count = sections.len();
        let mut section_storage = [LoftSection {
            controls: [Vec3::ZERO; 4],
        }; 8];
        section_storage[..section_count].copy_from_slice(&sections);
        let surface = LoftSurface {
            center,
            rotation,
            sections: section_storage,
            section_count: section_count as u8,
            tension: 0.0,
        };
        if let Err(error) = surface.validate() {
            self.status = format!("Cannot create loft: {error}");
            return;
        }
        self.loft_preview = Some(surface);
        self.renderer.invalidate();
        self.status =
            "Loft preview ready. Apply to save it, or cancel to leave the drawing unchanged."
                .into();
    }
}

fn fit_loft_section(points: &[Vec3]) -> LoftSection {
    let sample = |fraction: f32| {
        let mut lengths = Vec::with_capacity(points.len());
        lengths.push(0.0_f32);
        for pair in points.windows(2) {
            lengths.push(lengths.last().copied().unwrap() + pair[0].distance(pair[1]));
        }
        let total = *lengths.last().unwrap();
        let target = total * fraction;
        let index = lengths
            .partition_point(|length| *length < target)
            .clamp(1, points.len() - 1);
        let span = lengths[index] - lengths[index - 1];
        let t = if span <= 1e-8 {
            0.0
        } else {
            (target - lengths[index - 1]) / span
        };
        points[index - 1].lerp(points[index], t)
    };
    let [p0, q1, q2, p3] = [
        sample(0.0),
        sample(1.0 / 3.0),
        sample(2.0 / 3.0),
        sample(1.0),
    ];
    let a = q1 * 27.0 - p0 * 8.0 - p3;
    let b = q2 * 27.0 - p0 - p3 * 8.0;
    LoftSection {
        controls: [p0, a / 9.0 - b / 18.0, -a / 18.0 + b / 9.0, p3],
    }
}

pub(super) fn rotation_degrees(rotation: glam::Quat) -> Vec3 {
    let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}
fn rotation_from_degrees(degrees: Vec3) -> glam::Quat {
    let r = Vec3::new(
        degrees.x.to_radians(),
        degrees.y.to_radians(),
        degrees.z.to_radians(),
    );
    glam::Quat::from_euler(glam::EulerRot::ZYX, r.z, r.y, r.x).normalize()
}
fn revolved_geometry(value: Revolved) -> GuideGeometry {
    GuideGeometry::Revolved {
        center: value.center,
        rotation: value.rotation(),
        profile: value.profile[..value.count]
            .iter()
            .map(|p| SavedProfilePoint {
                height: p.height,
                radius: p.radius,
            })
            .collect(),
    }
}
fn to_revolved(center: Vec3, rotation: glam::Quat, profile: &[SavedProfilePoint]) -> Revolved {
    let mut result = Revolved::default();
    result.center = center;
    result.rotation_degrees = rotation_degrees(rotation);
    result.count = profile.len().min(8);
    for (dest, source) in result.profile.iter_mut().zip(profile.iter()).take(8) {
        *dest = ProfilePoint {
            height: source.height,
            radius: source.radius,
        };
    }
    result
}
fn to_cylinder(center: Vec3, rotation: glam::Quat, radius: f32, height: f32) -> Cylinder {
    Cylinder {
        center,
        rotation_degrees: rotation_degrees(rotation),
        radius,
        height,
    }
}

fn geometry_surface(geometry: &GuideGeometry) -> Option<Surface> {
    Some(match geometry {
        GuideGeometry::Plane { .. } => Surface::Plane,
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => Surface::Cylinder(to_cylinder(*center, *rotation, *radius, *height)),
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => Surface::Revolved(to_revolved(*center, *rotation, profile)),
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => Surface::Sphere(SphereSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
        }),
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => Surface::Cone(ConeSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        }),
        GuideGeometry::Freeform { surface } => Surface::Freeform(*surface),
        GuideGeometry::Loft { surface } => Surface::Loft(surface.clone()),
    })
}
fn geometry_preview(geometry: &GuideGeometry) -> Option<GuidePreview> {
    Some(match geometry {
        GuideGeometry::Plane { origin, rotation } => GuidePreview::Plane(DrawingPlane {
            anchor: *origin,
            rotation_degrees: rotation_degrees(*rotation),
            depth: 0.0,
        }),
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => GuidePreview::Cylinder(to_cylinder(*center, *rotation, *radius, *height)),
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => GuidePreview::Revolved(to_revolved(*center, *rotation, profile)),
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => GuidePreview::Sphere(SphereSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
        }),
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => GuidePreview::Cone(ConeSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        }),
        GuideGeometry::Freeform { surface } => GuidePreview::Freeform(*surface),
        GuideGeometry::Loft { surface } => GuidePreview::Loft(surface.clone()),
    })
}
fn geometry_frame(geometry: &GuideGeometry, world: Vec3) -> Option<(Vec3, Vec3)> {
    Some(match geometry {
        GuideGeometry::Plane { rotation, .. } => (*rotation * Vec3::Z, *rotation * Vec3::X),
        GuideGeometry::Cylinder {
            center, rotation, ..
        } => {
            let local = rotation.inverse() * (world - *center);
            (
                *rotation * Vec3::new(local.x, 0.0, local.z).normalize_or_zero(),
                *rotation * Vec3::new(local.z, 0.0, -local.x).normalize_or_zero(),
            )
        }
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => {
            let surface = to_revolved(*center, *rotation, profile);
            (surface.normal(world), surface.tangent(world))
        }
        GuideGeometry::Sphere {
            center, rotation, ..
        } => {
            let local = rotation.inverse() * (world - *center);
            let tangent = Vec3::new(local.z, 0.0, -local.x).normalize_or_zero();
            (
                *rotation * local.normalize_or_zero(),
                *rotation
                    * if tangent.length_squared() > 0.5 {
                        tangent
                    } else {
                        Vec3::X
                    },
            )
        }
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => {
            let local = rotation.inverse() * (world - *center);
            let k = *radius / *height;
            let tangent = Vec3::new(local.z, 0.0, -local.x).normalize_or_zero();
            (
                *rotation
                    * Vec3::new(local.x, k * k * (*height - local.y), local.z).normalize_or_zero(),
                *rotation
                    * if tangent.length_squared() > 0.5 {
                        tangent
                    } else {
                        Vec3::X
                    },
            )
        }
        GuideGeometry::Freeform { surface } => {
            let (u, v) = surface.coordinates(world).unwrap_or((0.5, 0.5));
            let (tangent, normal) = surface.frame(u, v);
            (normal, tangent)
        }
        GuideGeometry::Loft { surface } => {
            let (u, v) = surface.coordinates(world).unwrap_or((0.5, 0.5));
            let (tangent, normal) = surface.frame(u, v);
            (normal, tangent)
        }
    })
}

fn edit_geometry(ui: &mut egui::Ui, geometry: &mut GuideGeometry) -> bool {
    let mut changed = false;
    match geometry {
        GuideGeometry::Plane { origin, rotation } => {
            changed |= edit_transform(ui, origin, rotation);
        }
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        }
        | GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => {
            changed |= edit_transform(ui, center, rotation);
            changed |= ui
                .add(egui::Slider::new(radius, 0.001..=10000.0).text("Radius (wu)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(height, 0.001..=10000.0).text("Height (wu)"))
                .changed();
        }
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => {
            changed |= edit_transform(ui, center, rotation);
            changed |= ui
                .add(egui::Slider::new(radius, 0.001..=10000.0).text("Radius (wu)"))
                .changed();
        }
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => {
            changed |= edit_transform(ui, center, rotation);
            let mut remove = None;
            let can_remove = profile.len() > 2;
            for (i, point) in profile.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("P{}", i + 1));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.height)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.radius)
                                .speed(0.01)
                                .range(0.001..=10000.0)
                                .prefix("R "),
                        )
                        .changed();
                    if can_remove && ui.small_button("−").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(index) = remove {
                profile.remove(index);
                changed = true;
            }
            if profile.len() < 8 && ui.small_button("Add profile point").clicked() {
                let last = *profile.last().unwrap();
                profile.push(SavedProfilePoint {
                    height: last.height + 1.0,
                    radius: last.radius,
                });
                changed = true;
            }
        }
        GuideGeometry::Freeform { surface } => {
            changed |= edit_transform(ui, &mut surface.center, &mut surface.rotation);
            changed |= ui
                .add(egui::Slider::new(&mut surface.depth, 0.02..=10000.0).text("Sweep depth (wu)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut surface.bend, -0.9..=0.9).text("Sweep bend"))
                .changed();
            ui.small("Edit the four monotone directrix control points in local XY.");
            for (index, point) in surface.directrix.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("C{}", index + 1));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.x)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("X "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.y)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        )
                        .changed();
                });
            }
        }
        GuideGeometry::Loft { surface } => {
            changed |= edit_transform(ui, &mut surface.center, &mut surface.rotation);
            changed |= ui
                .add(egui::Slider::new(&mut surface.tension, -1.0..=1.0).text("Loft tension"))
                .changed();
            ui.small(format!(
                "Independent snapshot with {} ordered sections",
                surface.section_count
            ));
        }
    }
    changed
}
fn edit_transform(ui: &mut egui::Ui, center: &mut Vec3, rotation: &mut glam::Quat) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Center");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut center[axis])
                        .speed(0.01)
                        .range(-10000.0..=10000.0),
                )
                .changed();
        }
    });
    let mut degrees = rotation_degrees(*rotation);
    ui.horizontal(|ui| {
        ui.label("Rotation°");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut degrees[axis])
                        .speed(0.5)
                        .range(-3600.0..=3600.0),
                )
                .changed();
        }
    });
    if changed {
        *rotation = rotation_from_degrees(degrees);
    }
    changed
}
