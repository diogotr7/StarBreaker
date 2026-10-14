//! The shared model's per-vertex normals (`Mesh.normals`) as Blender's
//! `custom_normal` attribute, for the flat and NMC-hierarchy mesh writers.
//!
//! Blender 5.1 stores custom split normals as a corner-domain `INT16_2D`
//! attribute named `custom_normal`, each `short2` relative to its corner fan's
//! default normal; no flag or `sharp_edge` / `sharp_face` layer is needed (the
//! writers emit smooth faces, so the fans are the ones the encoder assumes).
//! `starbreaker_blend::custom_normals` does the encoding; this module wraps it
//! in the writers' block pattern:
//!
//! - [`CustomNormalLayer::allocate`]: encode a mesh's normals (none → `None`)
//!   and reserve the name / `AttributeArray` / raw-data pointers.
//! - [`CustomNormalLayer::attribute`]: the `Attribute` entry for the mesh's
//!   attribute array.
//! - [`CustomNormalLayer::write`]: the name, array and data blocks.

use starbreaker_blend::custom_normals::{encode_vertex_normals, short2_data, CUSTOM_NORMAL_ATTRIBUTE};
use starbreaker_blend::{
    build_attribute, build_attribute_array, write_block, PtrAlloc, ATTR_DOMAIN_CORNER, ATTR_TYPE_INT16_2D,
    SDNA_IDX_ATTRIBUTE_ARRAY,
};

use crate::error::Error;
use crate::types::Mesh;

/// One mesh's encoded `custom_normal` attribute and its block pointers.
#[derive(Debug, Clone)]
pub(super) struct CustomNormalLayer {
    name_ptr: u64,
    array_ptr: u64,
    raw_ptr: u64,
    corners: usize,
    data: Vec<u8>,
}

impl CustomNormalLayer {
    /// Encode `mesh.normals` (`None` when the mesh has none). Errors when the
    /// normals do not match the mesh (one per vertex, triangle indices).
    pub fn allocate(ptrs: &mut PtrAlloc, mesh: &Mesh) -> Result<Option<Self>, Error> {
        let Some(normals) = &mesh.normals else {
            return Ok(None);
        };
        let encoded = encode_vertex_normals(&mesh.positions, &mesh.indices, normals).map_err(Error::Other)?;
        Ok(Some(Self {
            name_ptr: ptrs.alloc(),
            array_ptr: ptrs.alloc(),
            raw_ptr: ptrs.alloc(),
            corners: encoded.len(),
            data: short2_data(&encoded),
        }))
    }

    /// The `Attribute` struct for the mesh's attribute array.
    pub fn attribute(&self) -> Vec<u8> {
        build_attribute(self.name_ptr, ATTR_TYPE_INT16_2D, ATTR_DOMAIN_CORNER, self.array_ptr)
    }

    /// Write the attribute's name, `AttributeArray` and raw data blocks.
    pub fn write(&self, out: &mut Vec<u8>) {
        let name = format!("{CUSTOM_NORMAL_ATTRIBUTE}\0");
        write_block(out, b"DATA", 0, self.name_ptr, 1, name.as_bytes());
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, self.array_ptr, 1,
            &build_attribute_array(self.raw_ptr, self.corners as i64));
        write_block(out, b"DATA", 0, self.raw_ptr, 1, &self.data);
    }
}
