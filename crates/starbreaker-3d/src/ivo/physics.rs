//! Decoder for the IVO `StatObjPhysics` chunk (0x58DE1772): the CryPhysics collision geometry a `.cga` / `.cgf`
//! carries per node. Layout measured on 40 094 ship geometry files (docs/decomposed-export-contract.md):
//! an 80-byte header with the entry count at 40, a table of (`u32` size, `u32` key) and the blobs back to back.
//! `key & 0xFFFF` is the NMC node (`0xFFFF` = the whole object), `key >> 16` the physics part (0: filtered /
//! raycast-only geometry, >= 1: the node's default colliders). Blobs starting `2` are CryPhysics
//! `phys_geometry` (box, trimesh, sphere, cylinder, capsule); `0xCAFEBABE` / `0xDEADBEB5` are Star Engine damage
//! physics (render-derived mesh, distance field), recorded by size only.
//!
//! Public: [`PhysicsEntry`], [`PhysicsGeometry`], [`parse_stat_obj_physics`], [`read_physics`].
use crate::error::Error;
use starbreaker_chunks::ChunkFile;

/// One table entry of the chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsEntry {
    /// NMC node index; `None` for whole-object physics (`0xFFFF`).
    pub node: Option<u16>,
    /// Physics part: 0 = filtered / raycast-only geometry, >= 1 = default colliders.
    pub part: u16,
    pub geometry: PhysicsGeometry,
}

/// Geometry in the node's own frame (CryEngine Z-up, metres).
#[derive(Debug, Clone, PartialEq)]
pub enum PhysicsGeometry {
    /// `axes` are the rows of CryPhysics' `Basis` (the box's axes in node space: it tests `Basis · (p − center)`).
    Box { axes: [[f32; 3]; 3], center: [f32; 3], half_size: [f32; 3] },
    Trimesh { vertices: Vec<[f32; 3]>, triangles: Vec<[u16; 3]>, material_ids: Vec<u8> },
    Sphere { center: [f32; 3], radius: f32 },
    /// `axis` unit, `half_height` half the axis length.
    Cylinder { center: [f32; 3], axis: [f32; 3], radius: f32, half_height: f32 },
    /// `half_height` = half the segment between the hemisphere centres.
    Capsule { center: [f32; 3], axis: [f32; 3], radius: f32, half_height: f32 },
    /// Star Engine damage physics (render-derived mesh), not decoded.
    DamageMesh { bytes: usize },
    /// Star Engine signed distance field, not decoded.
    Sdf { bytes: usize },
}

const PHYS_GEOM_VERSION: u32 = 2;
const DAMAGE_MESH_MAGIC: u32 = 0xCAFE_BABE;
const SDF_MAGIC: u32 = 0xDEAD_BEB5;
const GEOMETRY_OFFSET: usize = 72;

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
    what: &'static str,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(|| {
            Error::Other(format!("StatObjPhysics {}: needs {n} bytes at {}, blob has {}", self.what, self.at, self.data.len()))
        })?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, Error> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn i32(&mut self) -> Result<i32, Error> { Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn f32(&mut self) -> Result<f32, Error> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if v.is_finite() { Ok(v) } else { Err(Error::Other(format!("StatObjPhysics {}: non-finite float", self.what))) }
    }
    fn vec3(&mut self) -> Result<[f32; 3], Error> { Ok([self.f32()?, self.f32()?, self.f32()?]) }
    fn count(&mut self, what: &str) -> Result<usize, Error> {
        let v = self.i32()?;
        usize::try_from(v).map_err(|_| Error::Other(format!("StatObjPhysics {}: negative {what} {v}", self.what)))
    }
}

/// Decode the chunk body.
pub fn parse_stat_obj_physics(chunk: &[u8]) -> Result<Vec<PhysicsEntry>, Error> {
    let mut head = Cursor { data: chunk, at: 40, what: "header" };
    let count = head.count("entry count")?;
    head.at = 80;
    let mut table = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let size = u32::from_le_bytes(head.take(4)?.try_into().unwrap()) as usize;
        let key = u32::from_le_bytes(head.take(4)?.try_into().unwrap());
        table.push((size, key));
    }
    let mut blobs = Cursor { data: chunk, at: head.at, what: "table" };
    table
        .into_iter()
        .map(|(size, key)| {
            let blob = blobs.take(size)?;
            let node = (key & 0xFFFF) as u16;
            Ok(PhysicsEntry {
                node: (node != 0xFFFF).then_some(node),
                part: (key >> 16) as u16,
                geometry: parse_blob(blob)?,
            })
        })
        .collect()
}

fn parse_blob(blob: &[u8]) -> Result<PhysicsGeometry, Error> {
    let magic = blob.get(..4).map(|m| u32::from_le_bytes(m.try_into().unwrap()));
    match magic {
        Some(DAMAGE_MESH_MAGIC) => return Ok(PhysicsGeometry::DamageMesh { bytes: blob.len() }),
        Some(SDF_MAGIC) => return Ok(PhysicsGeometry::Sdf { bytes: blob.len() }),
        Some(PHYS_GEOM_VERSION) => {}
        _ => return Err(Error::Other(format!("StatObjPhysics: unknown blob {magic:08x?}"))),
    }
    let mut c = Cursor { data: blob, at: 68, what: "phys_geometry" };
    let kind = c.i32()?;
    debug_assert_eq!(c.at, GEOMETRY_OFFSET);
    Ok(match kind {
        0 => {
            let mut axes = [[0.0; 3]; 3];
            for row in &mut axes { *row = c.vec3()?; }
            let _oriented = c.i32()?;
            PhysicsGeometry::Box { axes, center: c.vec3()?, half_size: c.vec3()? }
        }
        1 => trimesh(&mut c)?,
        4 => PhysicsGeometry::Sphere { center: c.vec3()?, radius: c.f32()? },
        5 | 6 => {
            let (center, axis, radius, half_height) = (c.vec3()?, c.vec3()?, c.f32()?, c.f32()?);
            if kind == 5 { PhysicsGeometry::Cylinder { center, axis, radius, half_height } }
            else { PhysicsGeometry::Capsule { center, axis, radius, half_height } }
        }
        _ => return Err(Error::Other(format!("StatObjPhysics: unknown geometry type {kind}"))),
    })
}

/// Star Engine's CryPhysics trimesh: `nVerts, nTris, maxValency, flags, nA`; `u8 bVtxMap, u8 _`; `nA × u16`;
/// with a vertex map `nTris × u16` more; `nTris × u8` material ids; vertices; `nTris × 3 × u16` indices; tree tail.
fn trimesh(c: &mut Cursor) -> Result<PhysicsGeometry, Error> {
    c.what = "trimesh";
    let vertex_count = c.count("vertex count")?;
    let triangle_count = c.count("triangle count")?;
    let _max_valency = c.i32()?;
    let _flags = c.i32()?;
    let leading = c.count("leading array")?;
    let vertex_map = c.u8()? != 0;
    let _ = c.u8()?;
    c.take(2 * leading)?;
    if vertex_map { c.take(2 * triangle_count)?; }
    let material_ids = c.take(triangle_count)?.to_vec();
    let vertices = (0..vertex_count).map(|_| c.vec3()).collect::<Result<Vec<_>, _>>()?;
    let triangles = (0..triangle_count)
        .map(|_| {
            let t = [c.u16()?, c.u16()?, c.u16()?];
            if t.iter().any(|&i| i as usize >= vertex_count) {
                return Err(Error::Other(format!("StatObjPhysics trimesh: index {t:?} past {vertex_count} vertices")));
            }
            Ok(t)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PhysicsGeometry::Trimesh { vertices, triangles, material_ids })
}

/// The physics of a `.cga` / `.cgf` file: `None` when it has no `StatObjPhysics` chunk.
pub fn read_physics(file: &[u8]) -> Result<Option<Vec<PhysicsEntry>>, Error> {
    let chunk_file = ChunkFile::from_bytes(file)?;
    let ChunkFile::Ivo(ivo) = &chunk_file else { return Ok(None) };
    let Some(entry) = ivo.chunks().iter().find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::STATOBJ_PHYSICS) else {
        return Ok(None);
    };
    parse_stat_obj_physics(ivo.chunk_data(entry)).map(Some)
}

#[cfg(test)]
#[path = "physics_tests.rs"]
mod tests;
