//! Point containment for room-volume segments (`RoomVolume::contains`).
//!
//! Box, extruded polygon (even-odd rule) and closed triangle soup (parity of the
//! triangles a +Z ray crosses, with edges and vertices owned by exactly one
//! side). The Unreal importer repeats these tests on the exported volumes, so
//! both sides must agree on every case.

use super::room_volume::{RoomVolume, RoomVolumeShape};

impl RoomVolume {
    /// Whether a container-local point lies inside this segment.
    pub fn contains(&self, point: [f32; 3]) -> bool {
        let to_segment = glam::Mat4::from_cols_array_2d(&self.transform).inverse();
        let p = to_segment.transform_point3(glam::Vec3::from_array(point));
        if !p.is_finite() {
            return false;
        }
        match &self.shape {
            RoomVolumeShape::Box { min, max } => {
                (0..3).all(|axis| min[axis] <= p[axis] && p[axis] <= max[axis])
            }
            RoomVolumeShape::Prism { points, height } => {
                let base = points.iter().map(|q| q[2]).fold(f32::INFINITY, f32::min);
                if p.z < base || p.z > base + height {
                    return false;
                }
                // Even-odd rule on the XY polygon.
                let mut inside = false;
                for (index, a) in points.iter().enumerate() {
                    let b = points[(index + 1) % points.len()];
                    if (a[1] > p.y) != (b[1] > p.y)
                        && p.x < (b[0] - a[0]) * (p.y - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                inside
            }
            RoomVolumeShape::Trimesh { vertices } => {
                let origin = p.as_dvec3();
                let crossings = vertices
                    .chunks_exact(3)
                    .filter(|triangle| {
                        let corner = |index: usize| glam::Vec3::from_array(triangle[index]).as_dvec3() - origin;
                        upward_ray_crosses(corner(0), corner(1), corner(2))
                    })
                    .count();
                crossings % 2 == 1
            }
        }
    }
}

/// Whether the +Z ray from the origin crosses triangle `abc`.
///
/// Seen along the ray the triangle is a 2D triangle and the ray a point: it is
/// crossed when the point is on the same side of all three edges and the
/// surface lies ahead. Each edge is judged by [`edge_side`], which never
/// answers "on the edge", so a ray through an edge or a vertex of a closed soup
/// is counted for exactly one of the triangles that meet there on each side,
/// and a face seen edge-on is never crossed.
fn upward_ray_crosses(a: glam::DVec3, b: glam::DVec3, c: glam::DVec3) -> bool {
    let (Some(ab), Some(bc), Some(ca)) = (
        edge_side(a.truncate(), b.truncate()),
        edge_side(b.truncate(), c.truncate()),
        edge_side(c.truncate(), a.truncate()),
    ) else {
        return false;
    };
    if ab.1 != bc.1 || bc.1 != ca.1 {
        return false;
    }
    // Barycentric height of the surface above the origin: each edge weights the opposite corner.
    let area = ab.0 + bc.0 + ca.0;
    area != 0.0 && (bc.0 * a.z + ca.0 * b.z + ab.0 * c.z) / area > 0.0
}

/// Signed area of the directed edge `a → b` about the origin, and whether the
/// origin is to its left. `None` for an edge with no extent.
///
/// An origin exactly on the edge's line is placed as if it sat an infinitesimal
/// (ε, ε²) further along +X, +Y, which leaves the answer for every other point
/// untouched. The area is evaluated in one vertex order whichever way the edge
/// is traversed, so the two triangles sharing an edge always get opposite sides.
fn edge_side(a: glam::DVec2, b: glam::DVec2) -> Option<(f64, bool)> {
    let reversed = (b.x, b.y) < (a.x, a.y);
    let (first, second) = if reversed { (b, a) } else { (a, b) };
    let forward = first.x * second.y - first.y * second.x;
    let area = if reversed { -forward } else { forward };
    if area != 0.0 {
        Some((area, area > 0.0))
    } else if a.y != b.y {
        Some((0.0, a.y > b.y))
    } else if a.x != b.x {
        Some((0.0, b.x > a.x))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Triangle = [[f32; 3]; 3];

    fn soup(triangles: &[Triangle]) -> RoomVolume {
        RoomVolume {
            room_guid: "room".into(),
            room_name: None,
            source_entity: "soup".into(),
            transform: glam::Mat4::IDENTITY.to_cols_array_2d(),
            shape: RoomVolumeShape::Trimesh {
                vertices: triangles.iter().flatten().copied().collect(),
            },
        }
    }

    /// Unit cube without its top face: bottom split along (0,0,0)–(1,1,0).
    fn open_cube() -> Vec<Triangle> {
        let quad = |a, b, c, d| [[a, b, c], [a, c, d]];
        [
            quad([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
            quad([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [0.0, 0.0, 1.0]),
            quad([1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 1.0]),
            quad([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
            quad([0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]),
        ]
        .concat()
    }

    /// Top split along the diagonal (0,0,1)–(1,1,1): two triangles share that edge.
    fn cube_with_split_top() -> RoomVolume {
        let mut triangles = open_cube();
        triangles.push([[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]]);
        triangles.push([[0.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]]);
        soup(&triangles)
    }

    /// Top as a fan of four triangles around its centre vertex.
    fn cube_with_fan_top() -> RoomVolume {
        let mut triangles = open_cube();
        let centre = [0.5, 0.5, 1.0];
        let rim = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]];
        for index in 0..4 {
            triangles.push([rim[index], rim[(index + 1) % 4], centre]);
        }
        soup(&triangles)
    }

    /// Every lattice point with step 1/8 strictly inside the unit cube.
    fn interior_lattice() -> impl Iterator<Item = [f32; 3]> {
        let steps = || (1..8).map(|step| step as f32 / 8.0);
        steps().flat_map(move |x| steps().flat_map(move |y| steps().map(move |z| [x, y, z])))
    }

    #[test]
    fn interior_points_are_inside_whichever_edge_or_vertex_a_ray_meets() {
        // The lattice puts crossing rays exactly on shared edges, on the fan
        // centre, and on the bottom diagonal: none may be counted twice or not at all.
        for volume in [cube_with_split_top(), cube_with_fan_top()] {
            for point in interior_lattice() {
                assert!(volume.contains(point), "{point:?} is inside the cube");
            }
        }
    }

    #[test]
    fn a_point_whose_ray_meets_a_shared_edge_is_counted_once() {
        // An oblique ray (2,3,4) from here meets the top exactly on its diagonal.
        assert!(cube_with_split_top().contains([0.5, 0.375, 0.5]));
    }

    #[test]
    fn exterior_points_are_outside_along_walls_edges_and_corners() {
        for volume in [cube_with_split_top(), cube_with_fan_top()] {
            for x in [-0.5, 0.0, 0.25, 0.5, 1.0, 1.5] {
                for y in [-0.5, 0.0, 0.25, 0.5, 1.0, 1.5] {
                    for z in [-1.0, -0.125, 1.125, 2.0] {
                        let point = [x, y, z];
                        assert!(!volume.contains(point), "{point:?} is outside the cube");
                    }
                }
            }
        }
    }
}
