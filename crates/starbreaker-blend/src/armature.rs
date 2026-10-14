//! Armatures as Blender 5.1 `.blend` blocks: `bArmature` + `Bone`
//! ([`ArmatureData`]), the armature `Object` ([`build_armature_object`]) and
//! `ArmatureModifierData` ([`build_armature_modifier`]).
//!
//! A bone is given by its rest pose in armature space ([`BoneRest`]: head,
//! orientation, length). Blender stores that redundantly — parent-relative
//! head/tail/roll, `bone_mat`, and armature-space `arm_head`/`arm_tail`/
//! `arm_mat`/`arm_roll` — and rebuilds the matrices from head/tail/roll
//! whenever the armature leaves edit mode. [`vec_roll_to_mat3`] and
//! [`mat3_vec_to_roll`] port Blender's own conversions so every stored field
//! agrees with what Blender would rebuild.
//!
//! Matrices use Blender's layout: `m[i]` is axis `i` (a column in math
//! notation); a bone points along its Y axis.

use crate::{PtrAlloc, OBJECT_SIZE, write_block, write_cstr_fixed, write_f32, write_i16, write_i32, write_id_name, write_ptr};

pub const SDNA_IDX_BONE: u32 = 62;
pub const SDNA_IDX_ARMATURE: u32 = 64;
pub const SDNA_IDX_ARMATURE_MODIFIER: u32 = 368;
pub const BONE_SIZE: usize = 488;
pub const ARMATURE_SIZE: usize = 616;
pub const ARMATURE_MODIFIER_SIZE: usize = 208;
pub const MODIFIER_TYPE_ARMATURE: i32 = 8;
/// `Object.type` for armatures (`OB_ARMATURE`).
pub const OB_ARMATURE: i16 = 25;

/// 3×3 matrix in Blender layout (`m[i]` = axis `i`).
pub type Mat3 = [[f32; 3]; 3];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `m · v`.
fn mul_v(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
}

/// `a · b` (Blender `mul_m3_m3m3`).
fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    [mul_v(a, b[0]), mul_v(a, b[1]), mul_v(a, b[2])]
}

/// Inverse of a rotation.
fn transpose(m: &Mat3) -> Mat3 {
    [0, 1, 2].map(|i| [m[0][i], m[1][i], m[2][i]])
}

/// Rotation by `angle` about the unit `axis` (Blender
/// `axis_angle_normalized_to_mat3`).
fn axis_angle(axis: [f32; 3], angle: f32) -> Mat3 {
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    let [x, y, z] = axis;
    [
        [x * x * t + c, x * y * t + z * s, x * z * t - y * s],
        [x * y * t - z * s, y * y * t + c, y * z * t + x * s],
        [x * z * t + y * s, y * z * t - x * s, z * z * t + c],
    ]
}

/// The orientation of a bone pointing along `vec` with `roll` (Blender
/// `vec_roll_to_mat3`): the shortest rotation taking Y onto `vec`, then
/// `roll` about it. `vec` need not be normalised; it must be non-zero.
pub fn vec_roll_to_mat3(vec: [f32; 3], roll: f32) -> Mat3 {
    const SAFE_THRESHOLD: f32 = 6.1e-3;
    const CRITICAL_THRESHOLD: f32 = 2.5e-4;
    let len = dot(vec, vec).sqrt();
    let [x, y, z] = scale(vec, 1.0 / len);
    let mut theta = 1.0 + y;
    let theta_alt = x * x + z * z;
    let mut b = [[0.0f32; 3]; 3];
    if theta > SAFE_THRESHOLD || theta_alt > CRITICAL_THRESHOLD * CRITICAL_THRESHOLD {
        b[0][1] = -x;
        b[1][0] = x;
        b[1][1] = y;
        b[1][2] = z;
        b[2][1] = -z;
        if theta <= SAFE_THRESHOLD {
            theta = theta_alt * 0.5 + theta_alt * theta_alt * 0.125;
        }
        b[0][0] = 1.0 - x * x / theta;
        b[2][2] = 1.0 - z * z / theta;
        b[2][0] = -x * z / theta;
        b[0][2] = -x * z / theta;
    } else {
        // Along -Y: symmetry about Z.
        b = [[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    mul(&axis_angle([x, y, z], roll), &b)
}

/// The roll of orientation `mat` about the bone direction `vec` (Blender
/// `mat3_vec_to_roll`): the twist about Y of `mat` relative to
/// `vec_roll_to_mat3(vec, 0)`.
pub fn mat3_vec_to_roll(mat: &Mat3, vec: [f32; 3]) -> f32 {
    let r = mul(&transpose(&vec_roll_to_mat3(vec, 0.0)), mat);
    // Twist about Y of the rotation r: 2·atan2(q.y, q.w) = atan2(4wy, 2(w² − y²)).
    (r[2][0] - r[0][2]).atan2(r[0][0] + r[2][2])
}

/// A bone's rest pose in armature space.
#[derive(Debug, Clone, PartialEq)]
pub struct BoneRest {
    pub name: String,
    /// Index of the parent bone; parents precede their children.
    pub parent: Option<usize>,
    pub head: [f32; 3],
    /// Orientation (Blender layout); the bone points along `rotation[1]`.
    pub rotation: Mat3,
    /// Display length along the bone's Y axis (> 0).
    pub length: f32,
}

/// The stored fields of one `Bone` block.
#[derive(Debug, Clone, PartialEq)]
struct BoneFields {
    head: [f32; 3],
    tail: [f32; 3],
    roll: f32,
    bone_mat: Mat3,
    arm_head: [f32; 3],
    arm_tail: [f32; 3],
    /// Canonical armature-space orientation (rebuilt from Y + roll).
    arm_rot: Mat3,
    arm_roll: f32,
    length: f32,
}

/// Every bone's stored fields, the way Blender's `ED_armature_from_edit` +
/// `BKE_armature_where_is` derive them from armature-space head/tail/roll.
fn bone_fields(bones: &[BoneRest]) -> Result<Vec<BoneFields>, String> {
    let mut out: Vec<BoneFields> = Vec::with_capacity(bones.len());
    for (index, bone) in bones.iter().enumerate() {
        if !(bone.length > 0.0) {
            return Err(format!("bone '{}' has length {}", bone.name, bone.length));
        }
        let axis = bone.rotation[1];
        let arm_roll = mat3_vec_to_roll(&bone.rotation, axis);
        let arm_rot = vec_roll_to_mat3(axis, arm_roll);
        let arm_tail = add(bone.head, scale(arm_rot[1], bone.length));
        let (head, bone_mat) = match bone.parent {
            Some(p) if p < index => {
                let parent = &out[p];
                let inverse = transpose(&parent.arm_rot);
                (mul_v(&inverse, sub(bone.head, parent.arm_tail)), mul(&inverse, &arm_rot))
            }
            Some(p) => return Err(format!("bone '{}' (#{index}) has parent #{p} after it", bone.name)),
            None => (bone.head, arm_rot),
        };
        let roll = mat3_vec_to_roll(&bone_mat, bone_mat[1]);
        out.push(BoneFields {
            head,
            tail: add(head, scale(bone_mat[1], bone.length)),
            roll,
            bone_mat,
            arm_head: bone.head,
            arm_tail,
            arm_rot,
            arm_roll,
            length: bone.length,
        });
    }
    Ok(out)
}

fn write_v3(buf: &mut [u8], off: usize, v: [f32; 3]) {
    for (k, x) in v.into_iter().enumerate() {
        write_f32(buf, off + 4 * k, x);
    }
}

fn write_m3(buf: &mut [u8], off: usize, m: &Mat3) {
    for (i, axis) in m.iter().enumerate() {
        write_v3(buf, off + 12 * i, *axis);
    }
}

/// `bArmature` + its `Bone` blocks, with their pointers.
pub struct ArmatureData {
    pub armature_ptr: u64,
    armature: Vec<u8>,
    bones: Vec<(u64, Vec<u8>)>,
}

impl ArmatureData {
    /// Lay out an armature named `name` with `bones` (rest pose).
    pub fn new(name: &str, bones: &[BoneRest], ptrs: &mut PtrAlloc) -> Result<Self, String> {
        let fields = bone_fields(bones)?;
        let armature_ptr = ptrs.alloc();
        let bone_ptrs: Vec<u64> = bones.iter().map(|_| ptrs.alloc()).collect();
        // Sibling lists: the roots (bonebase) and each bone's children.
        let siblings = |parent: Option<usize>| -> Vec<usize> {
            (0..bones.len()).filter(|&i| bones[i].parent == parent).collect()
        };
        let mut next_prev = vec![(0u64, 0u64); bones.len()];
        let mut link = |list: &[usize]| {
            for (k, &i) in list.iter().enumerate() {
                let prev = if k > 0 { bone_ptrs[list[k - 1]] } else { 0 };
                let next = list.get(k + 1).map_or(0, |&n| bone_ptrs[n]);
                next_prev[i] = (next, prev);
            }
        };
        let roots = siblings(None);
        link(&roots);
        let children: Vec<Vec<usize>> = (0..bones.len()).map(|i| siblings(Some(i))).collect();
        for list in &children {
            link(list);
        }

        let bone_blocks = bones
            .iter()
            .zip(&fields)
            .enumerate()
            .map(|(i, (bone, f))| {
                let mut d = vec![0u8; BONE_SIZE];
                write_ptr(&mut d, 0, next_prev[i].0);
                write_ptr(&mut d, 8, next_prev[i].1);
                write_ptr(&mut d, 40, bone.parent.map_or(0, |p| bone_ptrs[p]));
                write_ptr(&mut d, 48, children[i].first().map_or(0, |&c| bone_ptrs[c]));
                write_ptr(&mut d, 56, children[i].last().map_or(0, |&c| bone_ptrs[c]));
                write_cstr_fixed(&mut d, 64, 64, &bone.name);
                write_f32(&mut d, 128, f.roll);
                write_v3(&mut d, 132, f.head);
                write_v3(&mut d, 144, f.tail);
                write_m3(&mut d, 156, &f.bone_mat);
                d[196] = 0xff; // drawtype: armature-defined
                for k in 0..3 {
                    d[211 + 4 * k] = 0xff; // BoneColor.custom solid/select/active alpha
                }
                write_v3(&mut d, 228, f.arm_head);
                write_v3(&mut d, 240, f.arm_tail);
                for (k, axis) in f.arm_rot.iter().enumerate() {
                    write_v3(&mut d, 252 + 16 * k, *axis);
                }
                write_v3(&mut d, 252 + 48, f.arm_head);
                write_f32(&mut d, 252 + 60, 1.0);
                write_f32(&mut d, 316, f.arm_roll);
                write_f32(&mut d, 320, 0.25); // dist
                write_f32(&mut d, 324, 1.0); // weight
                write_f32(&mut d, 328, 0.1); // xwidth
                write_f32(&mut d, 332, f.length);
                write_f32(&mut d, 336, 0.1); // zwidth
                write_f32(&mut d, 340, 0.1); // rad_head
                write_f32(&mut d, 344, 0.05); // rad_tail
                write_f32(&mut d, 372, 1.0); // ease1
                write_f32(&mut d, 376, 1.0); // ease2
                write_v3(&mut d, 396, [1.0; 3]); // scale_in
                write_v3(&mut d, 408, [1.0; 3]); // scale_out
                write_v3(&mut d, 420, [1.0; 3]); // size
                write_i16(&mut d, 436, 1); // segments
                (bone_ptrs[i], d)
            })
            .collect();

        let mut armature = vec![0u8; ARMATURE_SIZE];
        write_id_name(&mut armature, "AR", name);
        write_ptr(&mut armature, 416, roots.first().map_or(0, |&r| bone_ptrs[r]));
        write_ptr(&mut armature, 424, roots.last().map_or(0, |&r| bone_ptrs[r]));
        write_i32(&mut armature, 476, 0x800); // flag: ARM_COL_CUSTOM (Blender's default)
        write_i16(&mut armature, 484, 3); // deformflag: vertex groups + envelope
        write_i32(&mut armature, 588, 1); // layer (legacy)
        Ok(Self { armature_ptr, armature, bones: bone_blocks })
    }

    /// Write the `AR` block and its `Bone` blocks.
    pub fn write(&self, out: &mut Vec<u8>) {
        write_block(out, b"AR\0\0", SDNA_IDX_ARMATURE, self.armature_ptr, 1, &self.armature);
        for (ptr, data) in &self.bones {
            write_block(out, b"DATA", SDNA_IDX_BONE, *ptr, 1, data);
        }
    }
}

/// An `OB_ARMATURE` Object for `armature_ptr`, parented to `parent_ptr`
/// (0 = none) with a local transform (`quat` is `[w, x, y, z]`).
pub fn build_armature_object(
    object_name: &str,
    armature_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    let mut data = crate::build_empty_object(object_name, loc, quat, scale, parent_ptr);
    debug_assert_eq!(data.len(), OBJECT_SIZE);
    write_i16(&mut data, 416, OB_ARMATURE);
    write_ptr(&mut data, 552, armature_ptr);
    data
}

/// An Armature modifier deforming by vertex groups with `armature_object_ptr`.
pub fn build_armature_modifier(name: &str, next_ptr: u64, prev_ptr: u64, armature_object_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; ARMATURE_MODIFIER_SIZE];
    write_ptr(&mut data, 0, next_ptr);
    write_ptr(&mut data, 8, prev_ptr);
    write_i32(&mut data, 16, MODIFIER_TYPE_ARMATURE);
    write_i32(&mut data, 20, crate::MODIFIER_MODE_DEFAULT);
    write_cstr_fixed(&mut data, 40, 64, name);
    write_i16(&mut data, 120, 1); // deformflag: ARM_DEF_VGROUP
    write_ptr(&mut data, 128, armature_object_ptr);
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| close(a[k], b[k]))
    }

    fn close_m3(a: &Mat3, b: &Mat3) -> bool {
        (0..3).all(|i| close3(a[i], b[i]))
    }

    /// Blender 5.1's own reference armature (`ref.blend`): root (0,0,0)→(0,0,1),
    /// jaw (0,0.5,1)→(0,1,1.5) with roll 0.3, parented to root.
    fn reference() -> Vec<BoneRest> {
        vec![
            BoneRest { name: "root".into(), parent: None, head: [0.0; 3], rotation: vec_roll_to_mat3([0.0, 0.0, 1.0], 0.0), length: 1.0 },
            BoneRest {
                name: "jaw".into(),
                parent: Some(0),
                head: [0.0, 0.5, 1.0],
                rotation: vec_roll_to_mat3([0.0, 0.5, 0.5], 0.3),
                length: 0.5f32.sqrt(),
            },
        ]
    }

    #[test]
    fn vec_roll_matches_blender_reference() {
        let jaw = vec_roll_to_mat3([0.0, 0.5, 0.5], 0.3);
        // arm_mat rotation Blender wrote for the jaw.
        let want = [[0.95533, 0.20896, -0.20896], [0.0, 0.70711, 0.70711], [0.29552, -0.67552, 0.67552]];
        assert!(close_m3(&jaw, &want), "{jaw:?}");
        let root = vec_roll_to_mat3([0.0, 0.0, 1.0], 0.0);
        assert!(close_m3(&root, &[[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]), "{root:?}");
    }

    #[test]
    fn roll_round_trips_any_orientation() {
        for (vec, roll) in [([0.3f32, -0.8, 0.2], 1.1f32), ([0.0, -1.0, 0.0], 0.4), ([0.0, 1.0, 0.0], -2.5), ([1.0, 0.0, 0.0], 3.0)] {
            let m = vec_roll_to_mat3(vec, roll);
            let back = mat3_vec_to_roll(&m, m[1]);
            assert!(close_m3(&vec_roll_to_mat3(m[1], back), &m), "vec {vec:?} roll {roll}: {back}");
        }
    }

    /// The stored fields equal what Blender wrote for the same bones.
    #[test]
    fn bone_fields_match_blender_reference() {
        let f = bone_fields(&reference()).unwrap();
        assert!(close3(f[0].head, [0.0; 3]) && close3(f[0].tail, [0.0, 0.0, 1.0]) && close(f[0].roll, 0.0));
        assert!(close3(f[1].head, [0.0, 0.0, -0.5]), "{:?}", f[1].head);
        assert!(close3(f[1].tail, [0.0, 0.5, -1.0]), "{:?}", f[1].tail);
        assert!(close(f[1].roll, 0.3) && close(f[1].arm_roll, 0.3));
        let local = [[0.95533, -0.20896, -0.20896], [0.0, 0.70711, -0.70711], [0.29552, 0.67552, 0.67552]];
        assert!(close_m3(&f[1].bone_mat, &local), "{:?}", f[1].bone_mat);
        assert!(close3(f[1].arm_tail, [0.0, 1.0, 1.5]));
        assert!(close(f[1].length, 0.70711));
    }

    #[test]
    fn bone_fields_reject_bad_input() {
        let mut bones = reference();
        bones[1].length = 0.0;
        assert!(bone_fields(&bones).is_err());
        let mut bones = reference();
        bones[0].parent = Some(1);
        assert!(bone_fields(&bones).is_err(), "parent after child");
    }

    #[test]
    fn armature_blocks_link_the_hierarchy() {
        let mut ptrs = PtrAlloc::new(0x1000);
        let data = ArmatureData::new("Rig", &reference(), &mut ptrs).unwrap();
        let ptr = |d: &[u8], off: usize| u64::from_le_bytes(d[off..off + 8].try_into().unwrap());
        let (root_ptr, root) = &data.bones[0];
        let (jaw_ptr, jaw) = &data.bones[1];
        assert_eq!(&data.armature[40..46], b"ARRig\0");
        assert_eq!((ptr(&data.armature, 416), ptr(&data.armature, 424)), (*root_ptr, *root_ptr), "bonebase = roots");
        assert_eq!((ptr(root, 48), ptr(root, 56)), (*jaw_ptr, *jaw_ptr), "root.childbase = jaw");
        assert_eq!(ptr(jaw, 40), *root_ptr, "jaw.parent");
        assert_eq!(&jaw[64..68], b"jaw\0");
        let mut out = Vec::new();
        data.write(&mut out);
        assert_eq!(out.len(), 3 * 32 + ARMATURE_SIZE + 2 * BONE_SIZE, "AR + two Bone blocks");
    }

    #[test]
    fn armature_modifier_targets_the_object() {
        let m = build_armature_modifier("Armature", 0, 0, 0xABCD);
        assert_eq!(i32::from_le_bytes(m[16..20].try_into().unwrap()), MODIFIER_TYPE_ARMATURE);
        assert_eq!(u64::from_le_bytes(m[128..136].try_into().unwrap()), 0xABCD);
        let o = build_armature_object("Rig", 0x10, [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0);
        assert_eq!(i16::from_le_bytes(o[416..418].try_into().unwrap()), OB_ARMATURE);
    }
}
