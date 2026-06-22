//! Per-screen UI render aspect derived from the cockpit screen mesh.
//!
//! Engine-faithful sizing: a render-to-texture UI screen is displayed on a mesh
//! quad whose material is the render-target material (`RTT_Screen*`). The
//! physical display aspect is that quad's in-plane proportions — NOT the shared
//! `M_Physical_Screen` canvas size (1920×1080) every physical screen otherwise
//! inherits. Square gauges (g-force, velocity ball) must render square, the
//! annunciator as its wide strip, the compass wider still; the only place that
//! shape exists is the geometry.
//!
//! [`ui_screen_aspect`] finds the UI-render-target faces on a named mesh node and
//! returns their ORIENTED in-plane aspect width / height via principal-axis
//! analysis ([`planar_aspect_w_over_h`]): the two in-plane axes are classified
//! against world-up (`+Z`), so a landscape screen reports `> 1.0` and a PORTRAIT
//! screen (the LR-indicator's tall column display) reports `< 1.0` instead of the
//! orientation-blind long/short ratio. Computed from the exported world-space
//! vertices, so it survives the cockpit screen's tilt. Curved screens (e.g.
//! radar) yield the chord aspect, a close lower bound on the true arc aspect.
//! Validated against the in-game references: g-force/velocity 1.0, compass 3.27,
//! annunciator 5.58, the curved 4:3 MFDs 1.333, the LR-indicator 0.64 (portrait).

use crate::mtl::MtlFile;
use crate::nmc::NodeMeshCombo;
use crate::types::Mesh;

/// True when any material in `materials` is a UI render-target (`RTT_Screen*`).
/// Used by the export loader to keep real vertex positions for screen-bearing
/// geometry (whose display aspect must be measured) instead of the empty-mesh
/// decomposed cache shortcut.
pub(crate) fn materials_contain_ui_render_target(materials: Option<&MtlFile>) -> bool {
    materials.is_some_and(|file| file.materials.iter().any(|m| is_render_target_name(&m.name)))
}

/// Lower-cased substrings identifying the UI render-target faces of a screen
/// quad. Screens display the engine-pushed UI on faces whose material is a
/// render-to-texture display surface (`RTT_Screen` / `RTT_Hud`); only those faces
/// define the display aspect, so the rest of a multi-material housing mesh is
/// ignored. (`RTT_Text_To_Decal` and `Glass_*_RTO` are deliberately excluded —
/// they are text decals and glass overlays, not the display surface.)
const UI_RENDER_TARGET_MATERIAL_MARKERS: [&str; 2] = ["rtt_screen", "rtt_hud"];

/// True when `name` (a material name) marks a UI render-target display surface.
fn is_render_target_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    UI_RENDER_TARGET_MATERIAL_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// World-space "up" in the cockpit (CryEngine is Z-up): the axis used to
/// classify a screen quad's in-plane principal axes into horizontal (width) and
/// vertical (height) for the oriented aspect.
const COCKPIT_WORLD_UP: [f64; 3] = [0.0, 0.0, 1.0];

/// ORIENTED aspect width / height of the UI-render-target faces on the mesh node
/// named `node_name` — `> 1.0` landscape, `< 1.0` PORTRAIT (the LR-indicator's
/// tall column display) — or `None` when the screen quad cannot be located (no
/// matching material/node) or is degenerate.
///
/// The screen quad is tilted in the cockpit; its in-plane principal axes are
/// classified against [`COCKPIT_WORLD_UP`] so a portrait screen reports `< 1.0`
/// rather than the long/short ratio (the engine sizes the render target to the
/// real physical orientation). `node_name` is the binding's `helper_name` (the
/// cockpit screen hardpoint). When `nmc` is absent the node cannot be
/// disambiguated, so this returns `None` rather than risk measuring an unrelated
/// screen on the same mesh.
pub(crate) fn ui_screen_aspect(
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
    materials: Option<&MtlFile>,
    node_name: &str,
) -> Option<f32> {
    let points = collect_render_target_vertices(mesh, nmc, materials, node_name);
    if std::env::var_os("SB_SCREEN_ASPECT_ORIENT_PROBE").is_some()
        && let Some((axes, extents)) = planar_axes_extents(&points)
    {
        eprintln!(
            "SCREEN_ORIENT node={node_name} major_ax={:?} major_ext={:.3} minor_ax={:?} minor_ext={:.3} |major.z|={:.3} |minor.z|={:.3} long/short={:.3} w/h={:?}",
            axes[0], extents[0], axes[1], extents[1],
            axes[0][2].abs(), axes[1][2].abs(),
            extents[0] / extents[1].max(1e-9),
            planar_aspect_w_over_h(&points, COCKPIT_WORLD_UP),
        );
    }
    planar_aspect_w_over_h(&points, COCKPIT_WORLD_UP)
}

/// Gather the model-space vertex positions of every UI-render-target submesh that
/// belongs to the mesh node named `node_name`.
///
/// A submesh is a UI render-target when its `material_name` is set and marked
/// (the in-memory test path) OR — the real export case, where `material_name` is
/// empty and only `material_id` is populated — when the material it indexes in
/// `materials` is marked. The node gate isolates this screen from siblings on the
/// same mesh.
fn collect_render_target_vertices(
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
    materials: Option<&MtlFile>,
    node_name: &str,
) -> Vec<[f32; 3]> {
    // Dedup to UNIQUE vertices: the index list repeats shared quad corners
    // (two triangles share an edge), and duplicate points would bias the
    // principal-axis fit away from the quad's true edges.
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for submesh in &mesh.submeshes {
        let name_marks = submesh
            .material_name
            .as_deref()
            .is_some_and(is_render_target_name);
        let id_marks = materials
            .and_then(|file| file.materials.get(submesh.material_id as usize))
            .is_some_and(|m| is_render_target_name(&m.name));
        if !name_marks && !id_marks {
            continue;
        }
        let node_matches = nmc
            .and_then(|combo| combo.nodes.get(submesh.node_parent_index as usize))
            .is_some_and(|node| node.name.eq_ignore_ascii_case(node_name));
        if !node_matches {
            continue;
        }
        let start = (submesh.first_index as usize).min(mesh.indices.len());
        let end = start.saturating_add(submesh.num_indices as usize).min(mesh.indices.len());
        seen.extend(mesh.indices[start..end].iter().copied());
    }
    seen.into_iter()
        .filter_map(|index| mesh.positions.get(index as usize).copied())
        .collect()
}

/// Principal in-plane axes (world-space unit vectors) and their extents for a
/// point set, ranked by extent descending — `[major, minor, normal]`. `None`
/// when fewer than 3 points. Principal-axis analysis (eigenvectors of the
/// covariance) aligns the measurement axes to the quad's own edges, so the
/// result is correct regardless of how the screen is tilted in the cockpit.
fn planar_axes_extents(points: &[[f32; 3]]) -> Option<([[f64; 3]; 3], [f64; 3])> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mut centroid = [0.0f64; 3];
    for p in points {
        for k in 0..3 {
            centroid[k] += p[k] as f64;
        }
    }
    for k in 0..3 {
        centroid[k] /= n;
    }

    let mut cov = [[0.0f64; 3]; 3];
    for p in points {
        let d = [
            p[0] as f64 - centroid[0],
            p[1] as f64 - centroid[1],
            p[2] as f64 - centroid[2],
        ];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }

    let eig = jacobi_eigenvectors_3x3(cov);
    // Measure the spread along each principal axis (max - min projection); the
    // covariance eigenvalue orders by variance, but the extent is what the
    // render target must match, so measure extents directly and rank those.
    let mut pairs: Vec<([f64; 3], f64)> = eig
        .iter()
        .map(|&axis| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for p in points {
                let d = [
                    p[0] as f64 - centroid[0],
                    p[1] as f64 - centroid[1],
                    p[2] as f64 - centroid[2],
                ];
                let proj = d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2];
                lo = lo.min(proj);
                hi = hi.max(proj);
            }
            (axis, hi - lo)
        })
        .collect();
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let axes = [pairs[0].0, pairs[1].0, pairs[2].0];
    let extents = [pairs[0].1, pairs[1].1, pairs[2].1];
    Some((axes, extents))
}

/// In-plane aspect (largest / second-largest principal extent, ≥ 1.0) of a point
/// set, or `None` when fewer than 3 points or the set is effectively 1-D.
/// Orientation-AGNOSTIC: always the long/short ratio. Production sizing uses the
/// oriented [`planar_aspect_w_over_h`]; this remains as the long/short reference
/// the PCA-extent tests assert against.
#[cfg(test)]
fn planar_aspect(points: &[[f32; 3]]) -> Option<f32> {
    let (_, extents) = planar_axes_extents(points)?;
    let (major, minor) = (extents[0], extents[1]);
    if minor <= 1e-9 || !major.is_finite() {
        return None;
    }
    Some((major / minor) as f32)
}

/// ORIENTED in-plane aspect width / height (can be < 1.0 for a PORTRAIT screen),
/// where "up" in the cockpit is `world_up`. The two in-plane principal axes are
/// classified by their alignment with `world_up`: the axis more parallel to it
/// is the VERTICAL (height) axis, the other is HORIZONTAL (width); aspect =
/// horizontal_extent / vertical_extent.
///
/// A landscape screen (the annunciator, compass, MFDs) has its long axis
/// horizontal, so width = major, height = minor and the result matches
/// [`planar_aspect`]. A PORTRAIT screen (the LR-indicator's tall column display)
/// has its long axis vertical, so width = minor, height = major and the result
/// is < 1.0. `None` when degenerate or the plane is parallel to `world_up`
/// (no resolvable horizontal/vertical split).
pub(crate) fn planar_aspect_w_over_h(points: &[[f32; 3]], world_up: [f64; 3]) -> Option<f32> {
    let (axes, extents) = planar_axes_extents(points)?;
    // The two in-plane axes are the two largest extents (the third is the
    // near-zero plane normal).
    let up_align = |axis: &[f64; 3]| {
        (axis[0] * world_up[0] + axis[1] * world_up[1] + axis[2] * world_up[2]).abs()
    };
    let (a0, e0) = (axes[0], extents[0]);
    let (a1, e1) = (axes[1], extents[1]);
    // Classify the more up-aligned in-plane axis as vertical.
    let (width_extent, height_extent) = if up_align(&a0) >= up_align(&a1) {
        (e1, e0) // axis 0 is vertical (height), axis 1 horizontal (width)
    } else {
        (e0, e1) // axis 0 is horizontal (width), axis 1 vertical (height)
    };
    if height_extent <= 1e-9 || !width_extent.is_finite() {
        return None;
    }
    Some((width_extent / height_extent) as f32)
}

/// Eigenvectors (as `[x, y, z]` rows) of a symmetric 3×3 matrix via cyclic
/// Jacobi rotations. The covariance matrices here are tiny and well-conditioned,
/// so a fixed sweep count converges comfortably.
fn jacobi_eigenvectors_3x3(mut a: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const OFFS: [(usize, usize); 3] = [(0, 1), (0, 2), (1, 2)];
    for _ in 0..100 {
        // Annihilate the largest off-diagonal element.
        let mut pq = OFFS[0];
        let mut max = a[pq.0][pq.1].abs();
        for &(p, q) in &OFFS[1..] {
            if a[p][q].abs() > max {
                max = a[p][q].abs();
                pq = (p, q);
            }
        }
        if max < 1e-18 {
            break;
        }
        let (p, q) = pq;
        let apq = a[p][q];
        let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;

        let mut next = a;
        for k in 0..3 {
            if k != p && k != q {
                let akp = a[k][p];
                let akq = a[k][q];
                next[k][p] = c * akp - s * akq;
                next[p][k] = next[k][p];
                next[k][q] = s * akp + c * akq;
                next[q][k] = next[k][q];
            }
        }
        next[p][p] = a[p][p] - t * apq;
        next[q][q] = a[q][q] + t * apq;
        next[p][q] = 0.0;
        next[q][p] = 0.0;
        a = next;

        for k in 0..3 {
            let vkp = v[k][p];
            let vkq = v[k][q];
            v[k][p] = c * vkp - s * vkq;
            v[k][q] = s * vkp + c * vkq;
        }
    }
    // Return axes as rows [x,y,z]: axis j is column j of V.
    [
        [v[0][0], v[1][0], v[2][0]],
        [v[0][1], v[1][1], v[2][1]],
        [v[0][2], v[1][2], v[2][2]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmc::NmcNode;
    use crate::types::{Mesh, SubMesh};

    /// Rotate a point by `ax` (rad, about X) then `ay` (rad, about Y) so a planar
    /// quad becomes tilted in 3-space — the aspect must survive the tilt.
    fn tilt(p: [f32; 3], ax: f32, ay: f32) -> [f32; 3] {
        let (sa, ca) = (ax.sin(), ax.cos());
        let (x, y, z) = (p[0], ca * p[1] - sa * p[2], sa * p[1] + ca * p[2]);
        let (sy, cy) = (ay.sin(), ay.cos());
        [cy * x + sy * z, y, -sy * x + cy * z]
    }

    fn rect_points(w: f32, h: f32, ax: f32, ay: f32) -> Vec<[f32; 3]> {
        [[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, h, 0.0], [0.0, h, 0.0]]
            .into_iter()
            .map(|p| tilt(p, ax, ay))
            .collect()
    }

    #[test]
    fn planar_aspect_square_and_rectangles_flat() {
        assert!((planar_aspect(&rect_points(1.0, 1.0, 0.0, 0.0)).unwrap() - 1.0).abs() < 1e-4);
        assert!((planar_aspect(&rect_points(5.0, 1.0, 0.0, 0.0)).unwrap() - 5.0).abs() < 1e-4);
        // 4:3 regardless of which side is "width".
        assert!((planar_aspect(&rect_points(4.0, 3.0, 0.0, 0.0)).unwrap() - 4.0 / 3.0).abs() < 1e-4);
        assert!((planar_aspect(&rect_points(3.0, 4.0, 0.0, 0.0)).unwrap() - 4.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn planar_aspect_survives_tilt() {
        // The annunciator (5.58) is tilted in the cockpit; an AABB would collapse
        // it. Principal-axis analysis must recover the true aspect when tilted.
        let a = planar_aspect(&rect_points(5.58, 1.0, 0.6, -0.4)).unwrap();
        assert!((a - 5.58).abs() < 1e-3, "tilted aspect was {a}");
        let sq = planar_aspect(&rect_points(1.0, 1.0, 0.7, 0.3)).unwrap();
        assert!((sq - 1.0).abs() < 1e-3, "tilted square was {sq}");
    }

    /// Upright quad in the X-Z plane: width along X, height along world-up +Z.
    fn upright_rect_points(w: f32, h: f32, ax: f32, ay: f32) -> Vec<[f32; 3]> {
        [[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, 0.0, h], [0.0, 0.0, h]]
            .into_iter()
            .map(|p| tilt(p, ax, ay))
            .collect()
    }

    #[test]
    fn planar_aspect_w_over_h_orients_landscape_and_portrait() {
        let up = [0.0, 0.0, 1.0];
        // Landscape: width 2 (X) > height 1 (Z) → w/h = 2.0.
        let land = planar_aspect_w_over_h(&upright_rect_points(2.0, 1.0, 0.0, 0.0), up).unwrap();
        assert!((land - 2.0).abs() < 1e-4, "landscape w/h was {land}");
        // Portrait: width 1 (X) < height 2 (Z) → w/h = 0.5 (the LR-indicator case
        // long/short would report 2.0, hiding the portrait orientation).
        let port = planar_aspect_w_over_h(&upright_rect_points(1.0, 2.0, 0.0, 0.0), up).unwrap();
        assert!((port - 0.5).abs() < 1e-4, "portrait w/h was {port}");
        // long/short is orientation-agnostic (always >= 1) for both.
        assert!((planar_aspect(&upright_rect_points(1.0, 2.0, 0.0, 0.0)).unwrap() - 2.0).abs() < 1e-4);
    }

    #[test]
    fn planar_aspect_w_over_h_portrait_survives_tilt() {
        // A portrait screen tilted back in the cockpit must still read < 1.0: the
        // vertical edge (height, +Z-ward) stays the more up-aligned axis.
        let up = [0.0, 0.0, 1.0];
        let port = planar_aspect_w_over_h(&upright_rect_points(1.0, 1.56, 0.5, 0.0), up).unwrap();
        assert!((port - 1.0 / 1.56).abs() < 1e-2, "tilted portrait w/h was {port}");
    }

    #[test]
    fn planar_aspect_rejects_degenerate() {
        assert_eq!(planar_aspect(&[]), None);
        assert_eq!(planar_aspect(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]), None);
        // Collinear points have no second in-plane extent.
        assert_eq!(
            planar_aspect(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]),
            None
        );
    }

    fn empty_mesh() -> Mesh {
        Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: Vec::new(),
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
        }
    }

    fn node(name: &str) -> NmcNode {
        NmcNode {
            name: name.to_string(),
            parent_index: None,
            world_to_bone: [[0.0; 4]; 3],
            bone_to_world: [[0.0; 4]; 3],
            scale: [1.0; 3],
            geometry_type: 0,
            properties: Default::default(),
        }
    }

    fn submesh(material: &str, node_index: u16, num_indices: u32) -> SubMesh {
        SubMesh {
            material_name: Some(material.to_string()),
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices,
            first_vertex: 0,
            num_vertices: 0,
            node_parent_index: node_index,
        }
    }

    /// An UPRIGHT 2:1 (wide) quad on node index 1, material `RTT_Screen`, plus a
    /// decoy 1:1 quad on node index 2 with a non-UI material to prove selection
    /// is specific. The quad stands in the X-Z plane (width along X, height along
    /// world-up +Z) like a real cockpit screen, so the oriented w/h aspect is
    /// well-defined.
    fn screen_mesh() -> (Mesh, NodeMeshCombo) {
        let mut mesh = empty_mesh();
        // node-1 quad: width X=2.0, height Z=1.0 (landscape, upright in X-Z).
        mesh.positions = vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            // node-2 decoy quad: 1.0 x 1.0, offset in Y.
            [0.0, 5.0, 0.0],
            [1.0, 5.0, 0.0],
            [1.0, 5.0, 1.0],
            [0.0, 5.0, 1.0],
        ];
        mesh.indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let mut ui = submesh("drak_int_master_01_mtl_RTT_Screen_02", 1, 6);
        ui.first_index = 0;
        let mut decoy = submesh("drak_int_master_01_mtl_Panel_Metal", 2, 6);
        decoy.first_index = 6;
        mesh.submeshes = vec![ui, decoy];
        let nmc = NodeMeshCombo {
            nodes: vec![node("root"), node("Screen_Test"), node("Other_Geo")],
            material_indices: Vec::new(),
        };
        (mesh, nmc)
    }

    #[test]
    fn ui_screen_aspect_selects_ui_material_on_named_node() {
        let (mesh, nmc) = screen_mesh();
        let a = ui_screen_aspect(&mesh, Some(&nmc), None, "Screen_Test").unwrap();
        assert!((a - 2.0).abs() < 1e-4, "expected 2.0, got {a}");
        // case-insensitive node match
        assert!(ui_screen_aspect(&mesh, Some(&nmc), None, "screen_test").is_some());
    }

    #[test]
    fn ui_screen_aspect_none_for_wrong_node_or_no_nmc() {
        let (mesh, nmc) = screen_mesh();
        // node carries the decoy (non-UI) material only.
        assert_eq!(ui_screen_aspect(&mesh, Some(&nmc), None, "Other_Geo"), None);
        assert_eq!(ui_screen_aspect(&mesh, Some(&nmc), None, "Nonexistent"), None);
        // Without an NMC the node can't be disambiguated.
        assert_eq!(ui_screen_aspect(&mesh, None, None, "Screen_Test"), None);
    }
}
