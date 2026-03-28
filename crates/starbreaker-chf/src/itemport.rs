use serde::{Deserialize, Serialize};
use starbreaker_common::{CigGuid, NameHash, ParseError, SpanReader, SpanWriter};

use crate::read_helpers::{read_guid, read_name_hash};

/// A recursive item port tree node representing equipped items.
///
/// Game field names: `"portID"` (name hash), `"itemGUID"`, `"childCount"`.
/// The binary format stores these in a recursive tree structure. The `tail_count` field
/// is part of the binary serializer's bookkeeping — it's not exposed through the game's
/// serialization API but is present in the on-disk format and must be preserved for round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemPort {
    /// CRC32 hash of the port name. Game field: `"portID"`.
    pub name: NameHash,
    /// Item GUID. Game field: `"itemGUID"`.
    pub id: CigGuid,
    /// Binary format bookkeeping field. Zero on all nodes except the last sibling
    /// at each level, where it equals the total material count in the file.
    /// Not part of the game's serialization API but present in the on-disk format.
    pub tail_count: u32,
    pub children: Vec<ItemPort>,
}

impl ItemPort {
    /// Read an ItemPort (and its children recursively) from the reader.
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let name = read_name_hash(reader)?;
        let id = read_guid(reader)?;
        let child_count = reader.read_u32()?;
        let tail_count = reader.read_u32()?;

        let mut children = Vec::with_capacity(child_count as usize);
        for _ in 0..child_count {
            children.push(ItemPort::read(reader)?);
        }

        Ok(ItemPort {
            name,
            id,
            tail_count,
            children,
        })
    }

    /// Write this ItemPort (and its children recursively) to the writer.
    pub fn write(&self, writer: &mut SpanWriter) {
        writer.write_val(&self.name);
        writer.write_val(&self.id);
        writer.write_u32(self.children.len() as u32);
        writer.write_u32(self.tail_count);

        for child in &self.children {
            child.write(writer);
        }
    }

    /// Total number of item ports in the tree (this node + all descendants).
    pub fn total_count(&self) -> u64 {
        1 + self.children.iter().map(|c| c.total_count()).sum::<u64>()
    }
}
