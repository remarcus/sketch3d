use super::guide_resource_ui::PlanarTarget;
use super::*;

const CELL_PIXELS: f32 = 32.0;
const MAX_FEATURES: usize = 500_000;
const MAX_SEGMENTS: usize = 500_000;
const MAX_SEGMENT_CELL_REFERENCES: usize = 2_000_000;
const MAX_FEATURES_PER_CELL: usize = 128;
const MAX_SEGMENTS_PER_CELL: usize = 1024;
const MAX_SEGMENT_STEPS: usize = 512;
const MAX_INTERSECTION_PAIRS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapKind {
    Endpoint,
    Midpoint,
    Intersection,
    Grid,
}

impl SnapKind {
    pub(super) fn priority(self) -> u8 {
        match self {
            Self::Endpoint => 0,
            Self::Midpoint => 1,
            Self::Intersection => 2,
            Self::Grid => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Endpoint => "Endpoint",
            Self::Midpoint => "Midpoint",
            Self::Intersection => "Intersection",
            Self::Grid => "Grid",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SnapCue {
    pub screen: Pos2,
    pub kind: SnapKind,
    pub target_ids: [u64; 2],
}

#[derive(Clone, Copy)]
struct Feature {
    screen: Pos2,
    world: Vec3,
    kind: SnapKind,
    id: u64,
}

#[derive(Clone, Copy)]
struct Segment {
    a: Pos2,
    b: Pos2,
    id: u64,
    index: usize,
}

pub(super) struct SnapIndex {
    features: Vec<Feature>,
    feature_cells: HashMap<(i32, i32), Vec<usize>>,
    segments: Vec<Segment>,
    segment_cells: HashMap<(i32, i32), Vec<usize>>,
    coplanar_ids: HashSet<u64>,
    excluded_ids: HashSet<u64>,
    target: PlanarTarget,
    camera: Camera,
    rect: Rect,
    cell_references: usize,
    truncated: bool,
}

#[derive(Clone, Copy)]
struct Candidate {
    distance: f32,
    kind: SnapKind,
    id: u64,
    world: Vec3,
    screen: Pos2,
    ids: [u64; 2],
}

#[derive(Clone, Copy)]
struct SnapQuery {
    cursor: Pos2,
    tolerance: f32,
    angle: Option<(Vec3, Vec3)>,
    target: PlanarTarget,
}

#[derive(Clone, Copy)]
struct SnapCandidateInput {
    world: Vec3,
    screen: Pos2,
    kind: SnapKind,
    id: u64,
    ids: [u64; 2],
}

#[derive(Clone, Copy)]
pub(super) struct SnapOptions {
    pub enabled: bool,
    pub grid: bool,
    pub endpoints: bool,
    pub midpoints: bool,
    pub intersections: bool,
    pub spacing: f32,
    pub tolerance: f32,
    pub bypass: bool,
}

pub(super) struct SnapUi {
    pub options: SnapOptions,
    pub index: Option<SnapIndex>,
    pub cue: Option<SnapCue>,
}

impl Default for SnapUi {
    fn default() -> Self {
        Self {
            options: SnapOptions {
                enabled: true,
                grid: true,
                endpoints: true,
                midpoints: true,
                intersections: true,
                spacing: 0.25,
                tolerance: 12.0,
                bypass: false,
            },
            index: None,
            cue: None,
        }
    }
}

impl SnapUi {
    pub(super) fn controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Snapping", |ui| {
            let enabled = ui.checkbox(&mut self.options.enabled, "Planar snapping");
            if enabled.changed() && !self.options.enabled {
                self.clear();
            }
            if !self.options.enabled {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.options.grid, "Grid");
                ui.checkbox(&mut self.options.endpoints, "Endpoints");
                ui.checkbox(&mut self.options.midpoints, "Midpoints");
                ui.checkbox(&mut self.options.intersections, "Intersections");
            });
            if self.options.grid {
                ui_style::slider_row(
                    ui,
                    "Grid spacing",
                    "snap-grid-spacing",
                    &mut self.options.spacing,
                    0.01..=10.0,
                    100.0,
                    "wu",
                );
            }
            ui_style::slider_row(
                ui,
                "Acquire within",
                "snap-tolerance",
                &mut self.options.tolerance,
                4.0..=32.0,
                1.0,
                "px",
            );
            ui.small("Visible strokes on the active plane are targets, including locked layers. Hidden and off-plane strokes are excluded. Visible image and model references occlude candidates but are not snap targets. Nearest screen-space target wins; ties prefer endpoint, midpoint, intersection, then grid, then lower stroke ID. Hold Alt to bypass. Constrained lines only accept features within 1° of the constrained ray.");
        });
    }

    pub(super) fn prepare(
        &mut self,
        doc: &Document,
        camera: &Camera,
        rect: Rect,
        target: PlanarTarget,
        excluded_ids: &HashSet<u64>,
    ) {
        self.cue = None;
        self.index = Some(SnapIndex::build(doc, camera, rect, target, excluded_ids));
    }

    pub(super) fn clear(&mut self) {
        self.index = None;
        self.cue = None;
    }
}

impl SketchApp {
    pub(super) fn prepare_planar_snap(
        &mut self,
        target: PlanarTarget,
        rect: Rect,
        excluded_ids: &HashSet<u64>,
    ) {
        if !self.snapping.options.enabled {
            self.snapping.clear();
            return;
        }
        self.snapping
            .prepare(&self.doc, &self.camera, rect, target, excluded_ids);
        if self
            .snapping
            .index
            .as_ref()
            .is_some_and(|index| index.truncated)
        {
            self.status = "Snap search reached its per-gesture budget; some crowded scene features were skipped.".into();
        }
    }

    pub(super) fn snap_planar_point(
        &mut self,
        fallback: Vec3,
        cursor: Pos2,
        _target: PlanarTarget,
        rect: Rect,
        angle: Option<(Vec3, Vec3)>,
    ) -> Vec3 {
        let options = self.snapping.options;
        if !options.enabled || options.bypass {
            self.snapping.cue = None;
            return fallback;
        }
        if self.snapping.index.is_none() {
            self.prepare_planar_snap(_target, rect, &HashSet::new());
        }
        let result = self
            .snapping
            .index
            .as_ref()
            .and_then(|index| index.query(&self.doc, cursor, options, angle, true));
        if let Some((world, cue)) = result {
            if snap_occluded_by_scene_resources(
                &self.doc,
                &self.obj_reference_models,
                &mut self.obj_surface_indices,
                &self.camera,
                rect,
                cue.screen,
                world,
            ) {
                self.snapping.cue = None;
                fallback
            } else {
                self.snapping.cue = Some(cue);
                world
            }
        } else {
            self.snapping.cue = None;
            fallback
        }
    }

    pub(super) fn clear_planar_snap(&mut self) {
        self.snapping.clear();
    }

    pub(super) fn clear_planar_snap_cue(&mut self) {
        self.snapping.cue = None;
    }

    pub(super) fn planar_snap_index_stale(
        &self,
        target: PlanarTarget,
        rect: Rect,
        excluded_ids: &HashSet<u64>,
    ) -> bool {
        self.snapping
            .index
            .as_ref()
            .is_none_or(|index| !index.matches(&self.camera, rect, target, excluded_ids))
    }
}

impl SnapIndex {
    fn build(
        doc: &Document,
        camera: &Camera,
        rect: Rect,
        target: PlanarTarget,
        excluded_ids: &HashSet<u64>,
    ) -> Self {
        let mut index = Self {
            features: Vec::new(),
            feature_cells: HashMap::new(),
            segments: Vec::new(),
            segment_cells: HashMap::new(),
            coplanar_ids: HashSet::new(),
            excluded_ids: excluded_ids.clone(),
            target,
            camera: *camera,
            rect,
            cell_references: 0,
            truncated: false,
        };
        let plane_origin = target.plane.origin();
        let normal = target.plane.basis().2;
        let clip_rect = rect.expand(16.0);
        for stroke in &doc.strokes {
            if !doc.visible(stroke) {
                continue;
            }
            let on_plane = |point: Vec3| (point - plane_origin).dot(normal).abs() <= 0.005;
            if !stroke.points.iter().any(|point| on_plane(point.position)) {
                continue;
            }
            if stroke.points.iter().all(|point| on_plane(point.position)) {
                index.coplanar_ids.insert(stroke.id);
            }
            if !excluded_ids.contains(&stroke.id) {
                for endpoint in [stroke.points.first(), stroke.points.last()]
                    .into_iter()
                    .flatten()
                {
                    if on_plane(endpoint.position) {
                        if let Some((screen, _)) =
                            crate::gizmo::project(camera, rect, endpoint.position)
                        {
                            index.push_feature(
                                Feature {
                                    screen,
                                    world: endpoint.position,
                                    kind: SnapKind::Endpoint,
                                    id: stroke.id,
                                },
                                clip_rect,
                            );
                        }
                    }
                }
            }
            for (segment_index, pair) in stroke.points.windows(2).enumerate() {
                if !on_plane(pair[0].position) || !on_plane(pair[1].position) {
                    continue;
                }
                let (Some((a, _)), Some((b, _))) = (
                    crate::gizmo::project(camera, rect, pair[0].position),
                    crate::gizmo::project(camera, rect, pair[1].position),
                ) else {
                    continue;
                };
                let midpoint = pair[0].position.lerp(pair[1].position, 0.5);
                if !excluded_ids.contains(&stroke.id) {
                    if let Some((screen, _)) = crate::gizmo::project(camera, rect, midpoint) {
                        index.push_feature(
                            Feature {
                                screen,
                                world: midpoint,
                                kind: SnapKind::Midpoint,
                                id: stroke.id,
                            },
                            clip_rect,
                        );
                    }
                    index.push_segment(Segment {
                        a,
                        b,
                        id: stroke.id,
                        index: segment_index,
                    });
                }
            }
        }
        index
    }

    fn matches(
        &self,
        camera: &Camera,
        rect: Rect,
        target: PlanarTarget,
        excluded_ids: &HashSet<u64>,
    ) -> bool {
        self.rect == rect
            && self.target.plane == target.plane
            && self.target.half_extent() == target.half_extent()
            && self.excluded_ids == *excluded_ids
            && self.camera.projection == camera.projection
            && self.camera.yaw == camera.yaw
            && self.camera.pitch == camera.pitch
            && self.camera.roll == camera.roll
            && self.camera.distance == camera.distance
            && self.camera.target == camera.target
    }

    fn push_feature(&mut self, feature: Feature, clip_rect: Rect) {
        if self.features.len() >= MAX_FEATURES {
            self.truncated = true;
            return;
        }
        if !clip_rect.contains(feature.screen) {
            return;
        }
        let cell = cell(feature.screen);
        if let Some(indices) = self.feature_cells.get(&cell) {
            if indices.len() >= MAX_FEATURES_PER_CELL {
                self.truncated = true;
                let center = Pos2::new(
                    (cell.0 as f32 + 0.5) * CELL_PIXELS,
                    (cell.1 as f32 + 0.5) * CELL_PIXELS,
                );
                let worst = indices
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| {
                        self.features[**a]
                            .screen
                            .distance_sq(center)
                            .total_cmp(&self.features[**b].screen.distance_sq(center))
                    })
                    .map(|(position, index)| (position, *index));
                if let Some((position, old_index)) = worst {
                    let old = self.features[old_index];
                    let new_distance = feature.screen.distance_sq(center);
                    let old_distance = old.screen.distance_sq(center);
                    if new_distance < old_distance - 0.001
                        || ((new_distance - old_distance).abs() <= 0.001
                            && (feature.kind.priority(), feature.id)
                                < (old.kind.priority(), old.id))
                    {
                        self.features[old_index] = feature;
                        self.feature_cells.get_mut(&cell).unwrap()[position] = old_index;
                    }
                }
                return;
            }
        }
        let id = self.features.len();
        self.features.push(feature);
        self.feature_cells.entry(cell).or_default().push(id);
    }

    fn push_segment(&mut self, segment: Segment) {
        if self.segments.len() >= MAX_SEGMENTS {
            self.truncated = true;
            return;
        }
        let id = self.segments.len();
        self.segments.push(segment);
        let length = segment.a.distance(segment.b);
        let steps = (length / (CELL_PIXELS * 0.5)).ceil().max(1.0) as usize;
        let steps = steps.min(MAX_SEGMENT_STEPS);
        let mut previous = None;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let screen = segment.a + (segment.b - segment.a) * t;
            let cell = cell(screen);
            if previous != Some(cell) {
                if self.cell_references >= MAX_SEGMENT_CELL_REFERENCES {
                    self.truncated = true;
                    break;
                }
                let indices = self.segment_cells.entry(cell).or_default();
                if indices.len() < MAX_SEGMENTS_PER_CELL {
                    indices.push(id);
                    self.cell_references += 1;
                } else {
                    self.truncated = true;
                }
                previous = Some(cell);
            }
        }
    }

    pub(super) fn query(
        &self,
        doc: &Document,
        cursor: Pos2,
        options: SnapOptions,
        angle: Option<(Vec3, Vec3)>,
        check_occlusion: bool,
    ) -> Option<(Vec3, SnapCue)> {
        if !options.enabled || options.bypass || !cursor.is_finite() {
            return None;
        }
        let mut best: Option<Candidate> = None;
        let tolerance = options.tolerance.clamp(4.0, 32.0);
        let query = SnapQuery {
            cursor,
            tolerance,
            angle,
            target: self.target,
        };
        let cell_radius = (tolerance / CELL_PIXELS).ceil() as i32 + 1;
        let center = cell(cursor);
        let mut candidate_features = Vec::new();
        let mut candidate_segments = HashSet::new();
        for y in center.1 - cell_radius..=center.1 + cell_radius {
            for x in center.0 - cell_radius..=center.0 + cell_radius {
                if let Some(features) = self.feature_cells.get(&(x, y)) {
                    candidate_features.extend(features.iter().copied());
                }
                if let Some(segments) = self.segment_cells.get(&(x, y)) {
                    candidate_segments.extend(segments.iter().copied());
                }
            }
        }
        candidate_features.sort_unstable();
        for feature_index in candidate_features {
            let feature = self.features[feature_index];
            let enabled = match feature.kind {
                SnapKind::Endpoint => options.endpoints,
                SnapKind::Midpoint => options.midpoints,
                _ => false,
            };
            if !enabled {
                continue;
            }
            consider(
                &mut best,
                SnapCandidateInput {
                    world: feature.world,
                    screen: feature.screen,
                    kind: feature.kind,
                    id: feature.id,
                    ids: [feature.id, 0],
                },
                query,
            );
        }
        if options.intersections {
            let mut pair_count = 0;
            let mut segment_ids: Vec<_> = candidate_segments.into_iter().collect();
            segment_ids.sort_by_key(|index| {
                let segment = self.segments[*index];
                (segment.id, segment.index)
            });
            let mut groups: Vec<(u64, usize, usize)> = Vec::new();
            let mut first = 0;
            while first < segment_ids.len() {
                let id = self.segments[segment_ids[first]].id;
                let mut end = first + 1;
                while end < segment_ids.len() && self.segments[segment_ids[end]].id == id {
                    end += 1;
                }
                groups.push((id, first, end));
                first = end;
            }
            'cross_strokes: for first_group in 0..groups.len() {
                let (_, first_start, first_end) = groups[first_group];
                for &(_, second_start, second_end) in groups.iter().skip(first_group + 1) {
                    for first in first_start..first_end {
                        for second in second_start..second_end {
                            if !consider_intersection_pair(
                                self,
                                &segment_ids,
                                first,
                                second,
                                &mut pair_count,
                                &mut best,
                                query,
                            ) {
                                break 'cross_strokes;
                            }
                        }
                    }
                }
            }
            if pair_count < MAX_INTERSECTION_PAIRS {
                'self_crossings: for &(_, start, end) in &groups {
                    let mut nearby: Vec<_> = (start..end).collect();
                    nearby.sort_by(|a, b| {
                        let a_segment = self.segments[segment_ids[*a]];
                        let b_segment = self.segments[segment_ids[*b]];
                        point_segment_distance_squared(cursor, a_segment.a, a_segment.b).total_cmp(
                            &point_segment_distance_squared(cursor, b_segment.a, b_segment.b),
                        )
                    });
                    nearby.truncate(64);
                    nearby.sort_by_key(|position| self.segments[segment_ids[*position]].index);
                    for first_position in 0..nearby.len() {
                        for second_position in first_position + 1..nearby.len() {
                            if !consider_intersection_pair(
                                self,
                                &segment_ids,
                                nearby[first_position],
                                nearby[second_position],
                                &mut pair_count,
                                &mut best,
                                query,
                            ) {
                                break 'self_crossings;
                            }
                        }
                    }
                }
            }
        }
        if options.grid {
            let (u, v, _) = self.target.plane.basis();
            let origin = self.target.plane.origin();
            let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, self.rect, cursor);
            if let Some(point) = self.target.hit(ray_origin, ray_direction) {
                let offset = point - origin;
                let spacing = options.spacing.clamp(0.01, 10.0);
                let x = (offset.dot(u) / spacing).round() * spacing;
                let y = (offset.dot(v) / spacing).round() * spacing;
                let world = origin + u * x + v * y;
                if self.target.contains(world) {
                    if let Some((screen, _)) = crate::gizmo::project(&self.camera, self.rect, world)
                    {
                        consider(
                            &mut best,
                            SnapCandidateInput {
                                world,
                                screen,
                                kind: SnapKind::Grid,
                                id: u64::MAX,
                                ids: [0, 0],
                            },
                            query,
                        );
                    }
                }
            }
        }
        let candidate = best?;
        let cue = SnapCue {
            screen: candidate.screen,
            kind: candidate.kind,
            target_ids: candidate.ids,
        };
        if check_occlusion && !self.candidate_visible(doc, &self.camera, self.rect, cue) {
            return None;
        }
        Some((candidate.world, cue))
    }

    pub(super) fn candidate_visible(
        &self,
        doc: &Document,
        camera: &Camera,
        rect: Rect,
        cue: SnapCue,
    ) -> bool {
        let Some(picked) = crate::gizmo::pick_visible(doc, camera, rect, cue.screen) else {
            return true;
        };
        self.coplanar_ids.contains(&picked)
            || self.excluded_ids.contains(&picked)
            || cue.target_ids.contains(&picked)
    }
}

fn point_segment_distance_squared(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let delta = b - a;
    let t = ((point - a).dot(delta) / delta.length_sq().max(0.0001)).clamp(0.0, 1.0);
    point.distance_sq(a + delta * t)
}

fn consider_intersection_pair(
    index: &SnapIndex,
    segment_ids: &[usize],
    first: usize,
    second: usize,
    pair_count: &mut usize,
    best: &mut Option<Candidate>,
    query: SnapQuery,
) -> bool {
    if *pair_count >= MAX_INTERSECTION_PAIRS {
        return false;
    }
    *pair_count += 1;
    let a = index.segments[segment_ids[first]];
    let b = index.segments[segment_ids[second]];
    if a.id == b.id && a.index.abs_diff(b.index) <= 1 {
        return true;
    }
    let Some((ta, _)) = intersection(a.a, a.b, b.a, b.b) else {
        return true;
    };
    let screen = a.a + (a.b - a.a) * ta;
    let (origin, direction) = crate::gizmo::ray(&index.camera, index.rect, screen);
    let Some(world) = index.target.hit(origin, direction) else {
        return true;
    };
    let ids = if a.id <= b.id {
        [a.id, b.id]
    } else {
        [b.id, a.id]
    };
    consider(
        best,
        SnapCandidateInput {
            world,
            screen,
            kind: SnapKind::Intersection,
            id: a.id.min(b.id),
            ids,
        },
        query,
    );
    true
}

pub(super) fn snap_occluded_by_scene_resources(
    doc: &Document,
    models: &HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
    camera: &Camera,
    rect: Rect,
    screen: Pos2,
    target_world: Vec3,
) -> bool {
    let (origin, direction) = crate::gizmo::ray(camera, rect, screen);
    let direction_length = direction.length_squared();
    if direction_length <= f32::EPSILON {
        return false;
    }
    let target_t = (target_world - origin).dot(direction) / direction_length;
    if target_t <= 0.0 {
        return false;
    }
    let depth_tolerance = camera.world_per_pixel(target_world, rect.height()) * 0.25;
    for image in doc.image_planes.iter().filter(|image| image.visible) {
        let normal = image.rotation * Vec3::Z;
        let denominator = direction.dot(normal);
        if denominator.abs() <= 0.0001 {
            continue;
        }
        let t = (image.origin - origin).dot(normal) / denominator;
        if t <= 0.0 || t >= target_t - depth_tolerance {
            continue;
        }
        let hit = origin + direction * t;
        let local = image.rotation.inverse() * (hit - image.origin);
        if local.x.abs() <= image.size.x * 0.5 && local.y.abs() <= image.size.y * 0.5 {
            return true;
        }
    }
    super::obj_reference_ui::nearest_obj_reference_depth(
        origin,
        direction,
        &doc.model_references,
        models,
        indices,
    )
    .is_some_and(|depth| depth < target_t - depth_tolerance)
}

fn consider(best: &mut Option<Candidate>, input: SnapCandidateInput, query: SnapQuery) {
    let distance = query.cursor.distance(input.screen);
    if distance > query.tolerance || !query.target.contains(input.world) {
        return;
    }
    if let Some((start, constrained)) = query.angle {
        let expected = constrained - start;
        let candidate = input.world - start;
        if expected.length_squared() <= 1e-8 {
            if candidate.length_squared() > 1e-8 {
                return;
            }
        } else if candidate.length_squared() > 1e-8
            && expected.normalize().dot(candidate.normalize()) < 0.99985
        {
            return;
        }
    }
    let candidate = Candidate {
        distance,
        kind: input.kind,
        id: input.id,
        world: input.world,
        screen: input.screen,
        ids: input.ids,
    };
    let replace = best.is_none_or(|old| {
        if candidate.distance < old.distance - 0.001 {
            return true;
        }
        if (candidate.distance - old.distance).abs() > 0.001 {
            return false;
        }
        let candidate_key = (candidate.kind.priority(), candidate.id, candidate.ids);
        let old_key = (old.kind.priority(), old.id, old.ids);
        candidate_key < old_key
            || (candidate_key == old_key && world_lex_less(candidate.world, old.world))
    });
    if replace {
        *best = Some(candidate);
    }
}

fn world_lex_less(a: Vec3, b: Vec3) -> bool {
    for (left, right) in a.to_array().into_iter().zip(b.to_array()) {
        match left.total_cmp(&right) {
            std::cmp::Ordering::Less => return true,
            std::cmp::Ordering::Greater => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    false
}

fn intersection(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> Option<(f32, f32)> {
    let ab = b - a;
    let cd = d - c;
    let denominator = ab.x * cd.y - ab.y * cd.x;
    if denominator.abs() < 0.00001 {
        return None;
    }
    let offset = c - a;
    let ta = (offset.x * cd.y - offset.y * cd.x) / denominator;
    let tb = (offset.x * ab.y - offset.y * ab.x) / denominator;
    ((0.0..=1.0).contains(&ta) && (0.0..=1.0).contains(&tb)).then_some((ta, tb))
}

fn cell(point: Pos2) -> (i32, i32) {
    (
        (point.x / CELL_PIXELS).floor() as i32,
        (point.y / CELL_PIXELS).floor() as i32,
    )
}

pub(super) fn overlay(ui: &egui::Ui, cue: Option<SnapCue>, rect: Rect) {
    let Some(cue) = cue.filter(|cue| rect.contains(cue.screen)) else {
        return;
    };
    let painter = ui.painter().with_clip_rect(rect);
    let color = Color32::from_rgb(255, 210, 88);
    painter.circle_stroke(cue.screen, 7.0, egui::Stroke::new(1.6_f32, color));
    painter.circle_filled(cue.screen, 2.0, color);
    painter.text(
        cue.screen + Vec2::new(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        cue.kind.label(),
        egui::FontId::proportional(11.0),
        color,
    );
}

impl SketchApp {
    pub(super) fn verify_planar_snapping(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        target_stroke: &Stroke,
    ) -> Vec<&'static str> {
        let target = self
            .planar_drawing_target()
            .expect("verification image plane remains the active planar target");
        let target_endpoint = target_stroke
            .points
            .last()
            .expect("the target line has an endpoint")
            .position;
        assert!(target.contains(target_endpoint));
        assert!(self
            .doc
            .strokes
            .iter()
            .any(|stroke| stroke.id == target_stroke.id));

        let previous_options = self.snapping.options;
        let previous_constraint = self.line.constraint;
        let previous_line_enabled = self.line.enabled;
        let previous_alt = ctx.input(|input| input.modifiers.alt);
        self.snapping.options = SnapOptions {
            enabled: true,
            grid: false,
            endpoints: true,
            midpoints: false,
            intersections: false,
            spacing: previous_options.spacing,
            tolerance: 12.0,
            bypass: false,
        };
        self.line.constraint = super::line_tool::Constraint::Free;
        self.start_line_tool();

        let (u, v, _) = target.plane.basis();
        let start_world = target.plane.origin() - u * 1.2 + v * 0.9;
        let requested_end = target_endpoint + u * 0.035 + v * 0.01;
        let screen = |world| {
            crate::gizmo::project(&self.camera, rect, world)
                .expect("snap verification point projects into the canvas")
                .0
        };
        let start_screen = screen(start_world);
        let end_screen = screen(requested_end);
        assert!(rect.contains(start_screen) && rect.contains(end_screen));

        let stroke_count = self.doc.strokes.len();
        let undo_count = self.undo.len();
        self.line_input(start_screen, 1.0, None, rect, true);
        assert!(
            self.line.draft.is_some(),
            "line press creates a live preview"
        );
        assert_eq!(self.doc.strokes.len(), stroke_count);
        self.line_input(end_screen, 1.0, None, rect, false);
        let cue = self
            .snapping
            .cue
            .expect("line preview exposes its acquired snap cue");
        assert_eq!(cue.kind, SnapKind::Endpoint);
        assert_eq!(cue.target_ids, [target_stroke.id, 0]);
        let preview_endpoint = self.line.draft.as_ref().unwrap().mark.points[1].position;
        assert!(
            preview_endpoint.distance(target_endpoint) < 0.00001,
            "live line preview lands on the acquired endpoint"
        );
        self.release_line(end_screen, rect);
        assert_eq!(self.doc.strokes.len(), stroke_count + 1);
        assert_eq!(self.undo.len(), undo_count + 1, "line is one undo edit");
        let committed = self.doc.strokes.last().unwrap().clone();
        assert!(committed.points[1].position.distance(target_endpoint) < 0.00001);
        self.undo();
        assert_eq!(self.doc.strokes.len(), stroke_count);
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&committed));
        let mut checks = vec![
            "image-plane line preview displays an endpoint snap cue and commits the snapped target",
            "snapped image-plane line endpoint survives one undo and redo",
        ];

        // Drive Alt through the same canvas-input modifier path used by the
        // interactive app, then verify it suppresses the cue and snap.
        ctx.input_mut(|input| input.modifiers.alt = true);
        self.canvas_input(ctx, rect, Vec::new());
        assert!(self.snapping.options.bypass, "Alt enables snap bypass");
        self.start_line_tool();
        let bypass_stroke_count = self.doc.strokes.len();
        self.line_input(start_screen, 1.0, None, rect, true);
        self.line_input(end_screen, 1.0, None, rect, false);
        assert!(self.snapping.cue.is_none(), "Alt suppresses snap cues");
        let bypass_preview = self.line.draft.as_ref().unwrap().mark.points[1].position;
        assert!(
            bypass_preview.distance(target_endpoint) > 0.02,
            "Alt leaves the line preview at the unsnapped cursor position"
        );
        assert!(bypass_preview.distance(requested_end) < 0.001);
        ctx.input_mut(|input| {
            input.events = vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            input.modifiers.alt = false;
        });
        self.line_shortcuts(ctx);
        ctx.input_mut(|input| input.events.clear());
        assert!(
            self.line.draft.is_none(),
            "Escape cancels the bypass preview"
        );
        assert_eq!(self.doc.strokes.len(), bypass_stroke_count);
        assert_eq!(self.undo.len(), undo_count + 1);
        checks.push("Alt bypasses endpoint snapping and Escape cancels without a document edit");

        self.snapping.options = previous_options;
        self.clear_planar_snap();
        self.line.constraint = previous_constraint;
        if !previous_line_enabled {
            self.cancel_line_tool();
        }
        ctx.input_mut(|input| input.modifiers.alt = previous_alt);
        checks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Layer, Point, Stroke, StrokeMaterial};
    use crate::geometry::{DrawingPlane, Projection};
    use std::time::Instant;

    fn scene() -> (PlanarTarget, Camera, Rect, Vec3, Vec3, Vec3) {
        let plane = DrawingPlane {
            anchor: Vec3::new(1.25, -0.75, 2.5),
            rotation_degrees: Vec3::new(27.0, -38.0, 16.0),
            depth: 0.0,
        };
        let (u, v, normal) = plane.basis();
        let target = PlanarTarget::from_curve_frame(plane.origin(), normal, u, None)
            .expect("the rotated plane frame is valid");
        let mut camera = Camera {
            projection: Projection::Orthographic,
            ..Camera::default()
        };
        camera.align_guide(plane);
        camera.distance = 9.0;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::new(800.0, 600.0));
        (target, camera, rect, u, v, normal)
    }

    fn sample_point(position: Vec3) -> Point {
        Point {
            position,
            pressure: 0.7,
            tilt: None,
            time_ms: 0,
        }
    }

    fn stroke(id: u64, layer_id: u64, points: impl IntoIterator<Item = Vec3>) -> Stroke {
        Stroke {
            point_thickness: None,
            modifiers: None,
            fill: None,
            material: StrokeMaterial::Shaded,
            group_id: None,
            layer_id,
            id,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.02,
            brush: Brush::default(),
            points: points.into_iter().map(sample_point).collect(),
            curve: None,
        }
    }

    fn document(strokes: Vec<Stroke>) -> Document {
        Document {
            strokes,
            ..Document::default()
        }
    }

    fn project(camera: &Camera, rect: Rect, world: Vec3) -> Pos2 {
        crate::gizmo::project(camera, rect, world)
            .expect("test point is visible")
            .0
    }

    fn world_at_screen(target: PlanarTarget, camera: &Camera, rect: Rect, screen: Pos2) -> Vec3 {
        let (origin, direction) = crate::gizmo::ray(camera, rect, screen);
        target
            .hit(origin, direction)
            .expect("screen ray hits plane")
    }

    fn options() -> SnapOptions {
        SnapOptions {
            enabled: true,
            grid: false,
            endpoints: true,
            midpoints: true,
            intersections: true,
            spacing: 0.25,
            tolerance: 12.0,
            bypass: false,
        }
    }

    #[test]
    fn rotated_plane_candidates_follow_kind_priority_and_stroke_id_ties() {
        let (target, camera, rect, u, v, _) = scene();
        let center = target.plane.origin();
        // Deliberately insert the larger ID first. Equal-distance endpoint ties
        // resolve by ID, while a coincident endpoint outranks midpoint,
        // intersection, and grid candidates.
        let doc = document(vec![
            stroke(20, 1, [center, center + u * 0.25]),
            stroke(7, 1, [center - v * 0.25, center + v * 0.25]),
            stroke(11, 1, [center, center - u * 0.25]),
            stroke(3, 1, [center, center + v * 0.25]),
        ]);
        let index = SnapIndex::build(&doc, &camera, rect, target, &HashSet::new());
        assert!(!index.truncated);
        let cursor = project(&camera, rect, center);

        let mut query = options();
        query.grid = true;
        let (world, cue) = index
            .query(&doc, cursor, query, None, false)
            .expect("endpoint is within the acquisition tolerance");
        assert_eq!(cue.kind, SnapKind::Endpoint);
        assert_eq!(cue.target_ids, [3, 0]);
        assert!(world.distance(center) < 1.0e-5);

        query.endpoints = false;
        let (_, cue) = index
            .query(&doc, cursor, query, None, false)
            .expect("midpoint is within the acquisition tolerance");
        assert_eq!(cue.kind, SnapKind::Midpoint);

        query.midpoints = false;
        let (_, cue) = index
            .query(&doc, cursor, query, None, false)
            .expect("crossing segments form an intersection");
        assert_eq!(cue.kind, SnapKind::Intersection);

        query.intersections = false;
        let (_, cue) = index
            .query(&doc, cursor, query, None, false)
            .expect("the rotated-plane origin is the grid point");
        assert_eq!(cue.kind, SnapKind::Grid);
        assert_eq!(cue.target_ids, [0, 0]);
    }

    #[test]
    fn snap_index_keeps_visible_locked_targets_and_excludes_hidden_off_plane_or_selected_ink() {
        let (target, camera, rect, u, _, normal) = scene();
        let center = target.plane.origin();
        let mut doc = document(vec![
            stroke(1, 3, [center, center + u * 0.25]),
            stroke(8, 2, [center, center + u * 0.25]),
            // This stroke projects over the target but has no points on the
            // active plane, so it is not a feature source in SnapIndex.
            stroke(
                9,
                1,
                [center - normal * 0.2, center - normal * 0.2 + u * 0.25],
            ),
        ]);
        doc.layers.push(Layer {
            id: 2,
            name: "Locked target".into(),
            visible: true,
            locked: true,
            opacity: 1.0,
            blend_mode: Default::default(),
        });
        doc.layers.push(Layer {
            id: 3,
            name: "Hidden target".into(),
            visible: false,
            locked: false,
            opacity: 1.0,
            blend_mode: Default::default(),
        });
        assert!(!doc.editable(&doc.strokes[1]));
        assert!(doc.visible(&doc.strokes[1]));

        let index = SnapIndex::build(&doc, &camera, rect, target, &HashSet::new());
        assert!(!index.truncated);
        assert!(index.features.iter().any(|feature| feature.id == 8));
        assert!(!index.features.iter().any(|feature| feature.id == 1));
        assert!(!index.features.iter().any(|feature| feature.id == 9));
        assert!(index.coplanar_ids.contains(&8));
        assert!(!index.coplanar_ids.contains(&1));

        let cursor = project(&camera, rect, center);
        let (world, cue) = index
            .query(&doc, cursor, options(), None, false)
            .expect("visible locked ink remains a snap target");
        assert_eq!(cue.kind, SnapKind::Endpoint);
        assert_eq!(cue.target_ids, [8, 0]);
        assert!(world.distance(center) < 1.0e-5);
        assert!(index.candidate_visible(&doc, &camera, rect, cue));

        let excluded = HashSet::from([8]);
        let selected_index = SnapIndex::build(&doc, &camera, rect, target, &excluded);
        assert!(!selected_index
            .features
            .iter()
            .any(|feature| feature.id == 8));
        assert!(selected_index
            .query(&doc, cursor, options(), None, false)
            .is_none());
    }

    #[test]
    fn angle_constraint_filters_nearer_off_ray_snap_targets() {
        let (target, camera, rect, u, v, _) = scene();
        let start = target.plane.origin();
        let along_ray = start + u * 0.2;
        let off_ray = start + v * 0.08;
        let doc = document(vec![
            stroke(20, 1, [along_ray, along_ray + u * 0.08]),
            stroke(4, 1, [off_ray, off_ray + v * 0.08]),
        ]);
        let index = SnapIndex::build(&doc, &camera, rect, target, &HashSet::new());
        let cursor = project(&camera, rect, off_ray);
        let mut query = options();
        query.grid = false;
        query.midpoints = false;
        query.intersections = false;
        query.tolerance = 32.0;

        let (_, unconstrained) = index
            .query(&doc, cursor, query, None, false)
            .expect("the exact cursor target is acquired without a constraint");
        assert_eq!(unconstrained.target_ids, [4, 0]);

        let (_, constrained) = index
            .query(&doc, cursor, query, Some((start, start + u)), false)
            .expect("an on-ray endpoint remains within tolerance");
        assert_eq!(constrained.target_ids, [20, 0]);
    }

    fn nearby_index_counts(index: &SnapIndex, cursor: Pos2, tolerance: f32) -> (usize, usize) {
        let radius = (tolerance / CELL_PIXELS).ceil() as i32 + 1;
        let center = cell(cursor);
        let mut features = 0;
        let mut segments = HashSet::new();
        for y in center.1 - radius..=center.1 + radius {
            for x in center.0 - radius..=center.0 + radius {
                if let Some(items) = index.feature_cells.get(&(x, y)) {
                    features += items.len();
                }
                if let Some(items) = index.segment_cells.get(&(x, y)) {
                    segments.extend(items.iter().copied());
                }
            }
        }
        (features, segments.len())
    }

    #[test]
    fn large_scene_index_query_fixture_reports_bounded_local_work() {
        let (target, camera, _, _, _, _) = scene();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::new(1200.0, 800.0));
        let mut strokes = Vec::new();
        let mut query_screen = Pos2::ZERO;
        let columns = 36;
        let rows = 24;
        for row in 0..rows {
            for column in 0..columns {
                let x = 24.0 + column as f32 * 32.0;
                let y = 24.0 + row as f32 * 32.0;
                let first_screen = Pos2::new(x - 2.0, y);
                let last_screen = Pos2::new(x + 2.0, y);
                let first = world_at_screen(target, &camera, rect, first_screen);
                let last = world_at_screen(target, &camera, rect, last_screen);
                if row == 0 && column == 0 {
                    query_screen = first_screen;
                }
                strokes.push(stroke(
                    100 + (row * columns + column) as u64,
                    1,
                    [first, last],
                ));
            }
        }
        let fixture = document(strokes);
        let build_started = Instant::now();
        let index = SnapIndex::build(&fixture, &camera, rect, target, &HashSet::new());
        let build_elapsed = build_started.elapsed();
        assert!(!index.truncated, "fixture stays within index budgets");
        assert_eq!(index.features.len(), fixture.strokes.len() * 3);
        assert_eq!(index.segments.len(), fixture.strokes.len());

        let tolerance = 12.0;
        let (candidate_features, candidate_segments) =
            nearby_index_counts(&index, query_screen, tolerance);
        assert!(
            candidate_features <= 75,
            "{candidate_features} nearby features"
        );
        assert!(
            candidate_segments <= 25,
            "{candidate_segments} nearby segments"
        );

        let mut query = options();
        query.grid = false;
        query.midpoints = false;
        query.intersections = false;
        query.tolerance = tolerance;
        let query_started = Instant::now();
        const QUERY_COUNT: usize = 1_000;
        for _ in 0..QUERY_COUNT {
            let (_, cue) = index
                .query(&fixture, query_screen, query, None, false)
                .expect("fixture endpoint is queryable");
            assert_eq!(cue.target_ids[0], 100);
        }
        let query_elapsed = query_started.elapsed();
        eprintln!(
            "SNAP_FIXTURE strokes={} features={} segments={} nearby_features={} nearby_segments={} repeated_queries={} build_us={} query_total_us={}",
            fixture.strokes.len(),
            index.features.len(),
            index.segments.len(),
            candidate_features,
            candidate_segments,
            QUERY_COUNT,
            build_elapsed.as_micros(),
            query_elapsed.as_micros(),
        );
    }
}
