//! Bounded acceleration and adjacency data for OBJ surface picking.
//!
//! The index owns a BVH and per-triangle edge neighbors; query vertices and
//! normals are resolved from the unchanged `ObjModel`. Adjacency uses OBJ
//! position indices, so separately indexed but coincident vertices are treated
//! as a seam.

use crate::obj_reference::ObjModel;
use glam::Vec3;

const LEAF_TRIANGLES: usize = 8;
const NO_TRIANGLE: u32 = u32::MAX;
pub const MAX_STROKE_CONTACT_GAP_PIXELS: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjMeshHit {
    pub triangle_index: u32,
    /// Weights corresponding to the triangle's three OBJ corners.
    pub barycentric: Vec3,
    pub position: Vec3,
    /// Interpolated vertex normal, or the source face normal if interpolation
    /// is invalid. Orientation follows the OBJ normals and face winding.
    pub normal: Vec3,
    /// Ray parameter `t`; this is world distance only when direction is unit.
    pub ray_t: f32,
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    fn include(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    fn extent(self) -> Vec3 {
        self.max - self.min
    }

    fn hit(self, origin: Vec3, direction: Vec3, max_t: f32) -> Option<f32> {
        let mut near = 0.0_f32;
        let mut far = max_t;
        for axis in 0..3 {
            let o = origin[axis];
            let d = direction[axis];
            if d == 0.0 {
                if o < self.min[axis] || o > self.max[axis] {
                    return None;
                }
                continue;
            }
            let inverse = d.recip();
            let mut first = (self.min[axis] - o) * inverse;
            let mut last = (self.max[axis] - o) * inverse;
            if first > last {
                std::mem::swap(&mut first, &mut last);
            }
            near = near.max(first);
            far = far.min(last);
            if far < near {
                return None;
            }
        }
        (far > 0.0).then_some(near.max(0.0))
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Bounds,
    left: u32,
    right: u32,
    start: u32,
    count: u32,
}

impl Node {
    fn leaf(bounds: Bounds, start: usize, count: usize) -> Self {
        Self {
            bounds,
            left: NO_TRIANGLE,
            right: NO_TRIANGLE,
            start: start as u32,
            count: count as u32,
        }
    }

    fn branch(bounds: Bounds, left: usize, right: usize) -> Self {
        Self {
            bounds,
            left: left as u32,
            right: right as u32,
            start: 0,
            count: 0,
        }
    }

    fn is_leaf(self) -> bool {
        self.count > 0
    }
}

#[derive(Clone, Copy)]
struct EdgeRecord {
    low: u32,
    high: u32,
    owner: u32,
}

/// Immutable BVH and triangle-edge neighbors for one `ObjModel` topology.
#[derive(Clone, Debug)]
pub struct ObjMeshIndex {
    nodes: Vec<Node>,
    triangle_order: Vec<u32>,
    adjacency: Vec<[u32; 3]>,
}

impl ObjMeshIndex {
    /// Build the index in O(n log n) time with bounded storage. Models with no
    /// triangles, more than one million triangles, invalid indices, or nonfinite
    /// vertex data are rejected.
    pub fn new(model: &ObjModel) -> Result<Self, String> {
        let triangle_count = model.triangles.len();
        if triangle_count == 0 {
            return Err("Cannot index an OBJ with no triangles".into());
        }
        if triangle_count > crate::obj_reference::MAX_TRIANGLES {
            return Err("OBJ surface index exceeds one million triangles".into());
        }
        if model.positions.iter().any(|point| !point.is_finite()) {
            return Err("OBJ position data contains a nonfinite value".into());
        }
        if model.normals.iter().any(|normal| {
            !normal.is_finite()
                || !normal.length_squared().is_finite()
                || normal.length_squared() <= 1e-12
        }) {
            return Err("OBJ normal data contains an invalid vector".into());
        }
        if model.texcoords.iter().any(|texcoord| !texcoord.is_finite()) {
            return Err("OBJ UV data contains a nonfinite value".into());
        }

        let mut centroids = Vec::with_capacity(triangle_count);
        let mut edges = Vec::with_capacity(triangle_count.saturating_mul(3));
        let mut adjacency = vec![[NO_TRIANGLE; 3]; triangle_count];
        for (triangle_index, triangle) in model.triangles.iter().enumerate() {
            if !triangle.face_normal.is_finite()
                || !triangle.face_normal.length_squared().is_finite()
                || triangle.face_normal.length_squared() <= 1e-12
            {
                return Err("OBJ triangle has an invalid face normal".into());
            }
            for corner in triangle.corners {
                let Some(_) = model.positions.get(corner.position as usize) else {
                    return Err("OBJ triangle references an invalid position".into());
                };
                if corner
                    .normal
                    .is_some_and(|normal| normal as usize >= model.normals.len())
                    || corner
                        .texcoord
                        .is_some_and(|uv| uv as usize >= model.texcoords.len())
                {
                    return Err("OBJ triangle references an invalid normal or UV".into());
                }
            }
            let vertices = triangle_vertices(model, triangle)
                .ok_or("OBJ triangle references an invalid position")?;
            let area = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
            let area_squared = area.length_squared();
            if !area.is_finite() || !area_squared.is_finite() || area_squared <= 0.0 {
                return Err("OBJ triangle has zero area".into());
            }
            let centroid = vertices[0] / 3.0 + vertices[1] / 3.0 + vertices[2] / 3.0;
            if !centroid.is_finite() {
                return Err("OBJ triangle has invalid bounds".into());
            }
            centroids.push(centroid);
            for edge in 0..3 {
                let first = triangle.corners[edge].position;
                let second = triangle.corners[(edge + 1) % 3].position;
                if first == second {
                    return Err("OBJ triangle contains a collapsed edge".into());
                }
                let (low, high) = if first < second {
                    (first, second)
                } else {
                    (second, first)
                };
                let owner = triangle_index
                    .checked_mul(3)
                    .and_then(|owner| owner.checked_add(edge))
                    .ok_or("OBJ adjacency index overflow")?;
                edges.push(EdgeRecord {
                    low,
                    high,
                    owner: owner as u32,
                });
            }
        }

        edges.sort_unstable_by(|a, b| a.low.cmp(&b.low).then_with(|| a.high.cmp(&b.high)));
        let mut begin = 0;
        while begin < edges.len() {
            let mut end = begin + 1;
            while end < edges.len()
                && edges[end].low == edges[begin].low
                && edges[end].high == edges[begin].high
            {
                end += 1;
            }
            // Only manifold edges connect strokes. Non-manifold edges remain
            // boundaries so a contact cannot jump into an ambiguous branch.
            if end - begin == 2 {
                let first = edges[begin].owner;
                let second = edges[begin + 1].owner;
                let first_triangle = (first / 3) as usize;
                let first_edge = (first % 3) as usize;
                let second_triangle = (second / 3) as usize;
                let second_edge = (second % 3) as usize;
                adjacency[first_triangle][first_edge] = second_triangle as u32;
                adjacency[second_triangle][second_edge] = first_triangle as u32;
            }
            begin = end;
        }

        let mut index = Self {
            nodes: Vec::with_capacity(triangle_count.div_ceil(LEAF_TRIANGLES) * 2),
            triangle_order: (0..triangle_count as u32).collect(),
            adjacency,
        };
        index.build_node(model, &centroids, 0, triangle_count);
        Ok(index)
    }

    /// True for the same triangle or when two triangles share a manifold OBJ
    /// position-index edge. Coincident but separately indexed seams do not join.
    pub fn triangles_adjacent(&self, first: u32, second: u32) -> bool {
        if first == second {
            return (first as usize) < self.adjacency.len();
        }
        self.adjacency
            .get(first as usize)
            .is_some_and(|edges| edges.contains(&second))
    }

    /// Return the triangle across each oriented local edge, when the edge is
    /// manifold. The order matches `(corner[i], corner[(i + 1) % 3])`.
    pub fn triangle_neighbors(&self, triangle: u32) -> Option<[Option<u32>; 3]> {
        self.adjacency.get(triangle as usize).map(|neighbors| {
            neighbors.map(|neighbor| (neighbor != NO_TRIANGLE).then_some(neighbor))
        })
    }

    pub fn surface_contact_patch(
        &self,
        model: &ObjModel,
        triangle: u32,
        center: Vec3,
        rotation: glam::Quat,
        scale: f32,
    ) -> Result<Vec<crate::document::ModelSurfaceTriangle>, String> {
        if !center.is_finite()
            || !rotation.is_finite()
            || (rotation.length_squared() - 1.0).abs() > 1.0e-3
            || !scale.is_finite()
            || scale <= 0.0
        {
            return Err("Model surface transform is invalid".into());
        }
        let mut ids = vec![triangle];
        if let Some(neighbors) = self.triangle_neighbors(triangle) {
            ids.extend(neighbors.into_iter().flatten());
        }
        ids.sort_unstable();
        ids.dedup();
        let mut output = Vec::with_capacity(ids.len());
        for id in ids {
            let source = model
                .triangles
                .get(id as usize)
                .ok_or("Model contact refers to a missing triangle")?;
            let vertices = model.render_vertices(source)?;
            let positions = vertices.map(|vertex| center + rotation * (vertex.position * scale));
            let normal = (rotation * source.face_normal)
                .try_normalize()
                .ok_or("Model contact has an invalid surface normal")?;
            output.push(crate::document::ModelSurfaceTriangle {
                source_id: id,
                positions,
                normal,
            });
        }
        Ok(output)
    }

    /// Whether two pointer contacts may continue one model-surface stroke.
    /// A large screen-space gap or disconnected mesh edge starts a new stroke.
    pub fn stroke_contacts_connect(
        &self,
        previous_triangle: u32,
        current_triangle: u32,
        screen_gap_pixels: f32,
    ) -> bool {
        screen_gap_pixels.is_finite()
            && (0.0..=MAX_STROKE_CONTACT_GAP_PIXELS).contains(&screen_gap_pixels)
            && self.triangles_adjacent(previous_triangle, current_triangle)
    }

    /// Find the nearest positive, double-sided triangle hit in local OBJ space.
    /// Direction need not be normalized; `ray_t` is its corresponding parameter.
    pub fn hit(&self, model: &ObjModel, origin: Vec3, direction: Vec3) -> Option<ObjMeshHit> {
        if !origin.is_finite()
            || !direction.is_finite()
            || direction.length_squared() <= 1e-16
            || model.triangles.len() != self.adjacency.len()
        {
            return None;
        }
        let mut stack = Vec::with_capacity(32);
        stack.push(0_u32);
        let mut nearest_t = f32::INFINITY;
        let mut nearest: Option<(u32, Vec3)> = None;

        while let Some(node_index) = stack.pop() {
            let node = *self.nodes.get(node_index as usize)?;
            if node.bounds.hit(origin, direction, nearest_t).is_none() {
                continue;
            }
            if node.is_leaf() {
                let start = node.start as usize;
                let end = start.saturating_add(node.count as usize);
                for &triangle_index in self.triangle_order.get(start..end)? {
                    let triangle = model.triangles.get(triangle_index as usize)?;
                    let vertices = triangle_vertices(model, triangle)?;
                    if let Some((t, barycentric)) =
                        ray_triangle(origin, direction, vertices[0], vertices[1], vertices[2])
                    {
                        if t < nearest_t {
                            nearest_t = t;
                            nearest = Some((triangle_index, barycentric));
                        }
                    }
                }
            } else {
                let left = node.left as usize;
                let right = node.right as usize;
                let left_near = self
                    .nodes
                    .get(left)?
                    .bounds
                    .hit(origin, direction, nearest_t);
                let right_near = self
                    .nodes
                    .get(right)?
                    .bounds
                    .hit(origin, direction, nearest_t);
                match (left_near, right_near) {
                    (Some(a), Some(b)) if a <= b => {
                        stack.push(node.right);
                        stack.push(node.left);
                    }
                    (Some(_), Some(_)) => {
                        stack.push(node.left);
                        stack.push(node.right);
                    }
                    (Some(_), None) => stack.push(node.left),
                    (None, Some(_)) => stack.push(node.right),
                    (None, None) => {}
                }
            }
        }

        let (triangle_index, barycentric) = nearest?;
        let triangle = model.triangles.get(triangle_index as usize)?;
        let vertices = model.render_vertices(triangle).ok()?;
        let interpolated = vertices[0].normal * barycentric.x
            + vertices[1].normal * barycentric.y
            + vertices[2].normal * barycentric.z;
        let normal = interpolated
            .try_normalize()
            .unwrap_or_else(|| triangle.face_normal.normalize());
        Some(ObjMeshHit {
            triangle_index,
            barycentric,
            position: origin + direction * nearest_t,
            normal,
            ray_t: nearest_t,
        })
    }

    fn build_node(
        &mut self,
        model: &ObjModel,
        centroids: &[Vec3],
        start: usize,
        end: usize,
    ) -> usize {
        let mut bounds = Bounds::empty();
        let mut centroid_bounds = Bounds::empty();
        for &triangle_index in &self.triangle_order[start..end] {
            let triangle = &model.triangles[triangle_index as usize];
            for corner in triangle.corners {
                bounds.include(model.positions[corner.position as usize]);
            }
            centroid_bounds.include(centroids[triangle_index as usize]);
        }

        let node_index = self.nodes.len();
        self.nodes.push(Node::leaf(bounds, start, end - start));
        if end - start <= LEAF_TRIANGLES {
            return node_index;
        }

        let extent = centroid_bounds.extent();
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let middle = start + (end - start) / 2;
        let order = &mut self.triangle_order;
        order[start..end].select_nth_unstable_by(middle - start, |a, b| {
            centroids[*a as usize][axis]
                .total_cmp(&centroids[*b as usize][axis])
                .then_with(|| a.cmp(b))
        });
        let left = self.build_node(model, centroids, start, middle);
        let right = self.build_node(model, centroids, middle, end);
        self.nodes[node_index] = Node::branch(bounds, left, right);
        node_index
    }
}

fn triangle_vertices(
    model: &ObjModel,
    triangle: &crate::obj_reference::ObjTriangle,
) -> Option<[Vec3; 3]> {
    Some([
        *model.positions.get(triangle.corners[0].position as usize)?,
        *model.positions.get(triangle.corners[1].position as usize)?,
        *model.positions.get(triangle.corners[2].position as usize)?,
    ])
}

fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, Vec3)> {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    let scale = edge1.length() * edge2.length() * direction.length();
    if scale <= 1e-30 || determinant.abs() <= scale * 1e-8 {
        return None;
    }
    let inverse = determinant.recip();
    let offset = origin - a;
    let u = offset.dot(p) * inverse;
    if !(-1e-6..=1.000001).contains(&u) {
        return None;
    }
    let q = offset.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < -1e-6 || u + v > 1.000001 {
        return None;
    }
    let t = edge2.dot(q) * inverse;
    if t <= 0.0 || !t.is_finite() {
        return None;
    }
    Some((t, Vec3::new(1.0 - u - v, u, v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_obj(contents: &str) -> crate::obj_reference::ObjModel {
        let directory = tempfile::tempdir().unwrap();
        crate::obj_reference::parse(contents.as_bytes(), &directory.path().join("mesh.obj"))
            .unwrap()
    }

    #[test]
    fn double_sided_hits_and_manifold_contact_continuity_are_bounded() {
        let model = parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 1 3 4\n");
        let index = ObjMeshIndex::new(&model).unwrap();
        assert!(index.triangles_adjacent(0, 1));
        assert!(index.stroke_contacts_connect(0, 1, 24.0));
        assert!(!index.stroke_contacts_connect(0, 1, 24.01));
        assert!(!index.stroke_contacts_connect(0, 1, f32::NAN));

        let above = index
            .hit(&model, Vec3::new(0.7, 0.2, 1.0), -Vec3::Z)
            .unwrap();
        let below = index
            .hit(&model, Vec3::new(0.7, 0.2, -1.0), Vec3::Z)
            .unwrap();
        assert!(above.position.distance(Vec3::new(0.7, 0.2, 0.0)) < 1e-5);
        assert!(below.position.distance(above.position) < 1e-5);

        let stacked = parse_obj(
            "v -1 -1 0\nv 1 -1 0\nv 0 1 0\nv -1 -1 1\nv 1 -1 1\nv 0 1 1\nf 1 2 3\nf 4 5 6\n",
        );
        let stacked_index = ObjMeshIndex::new(&stacked).unwrap();
        let nearest = stacked_index
            .hit(&stacked, Vec3::new(0.0, 0.0, 2.0), -Vec3::Z)
            .unwrap();
        assert!((nearest.position.z - 1.0).abs() < 1e-5);
        assert!(stacked_index
            .hit(&stacked, Vec3::new(3.0, 0.0, 2.0), -Vec3::Z)
            .is_none());
    }

    #[test]
    fn duplicated_vertices_and_nonmanifold_edges_do_not_join_surface_strokes() {
        let separate = parse_obj(
            "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\nf 1 3 4\nf 5 6 7\n",
        );
        let separate_index = ObjMeshIndex::new(&separate).unwrap();
        assert!(!separate_index.triangles_adjacent(0, 2));
        assert!(!separate_index.stroke_contacts_connect(0, 2, 2.0));

        let nonmanifold = parse_obj(
            "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0.5 0.5 1\nf 1 2 3\nf 1 3 4\nf 1 3 5\n",
        );
        let nonmanifold_index = ObjMeshIndex::new(&nonmanifold).unwrap();
        assert!(!nonmanifold_index.triangles_adjacent(0, 1));
        assert!(!nonmanifold_index.triangles_adjacent(0, 2));
    }
}
