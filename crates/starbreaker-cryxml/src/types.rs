use zerocopy::{FromBytes, Immutable, KnownLayout};

/// File header immediately following the 8-byte magic.
///
/// All offsets are relative to the start of the file (including the magic).
#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct CryXmlHeader {
    pub xml_size: u32,
    pub node_table_position: u32,
    pub node_count: u32,
    pub attribute_table_position: u32,
    pub attribute_count: u32,
    pub child_table_position: u32,
    pub child_count: u32,
    pub string_data_position: u32,
    pub string_data_size: u32,
}

/// A single node in the XML tree (28 bytes, packed).
#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct CryXmlNode {
    /// Byte offset into the string data section for this node's tag name.
    pub tag_string_offset: u32,
    /// Unused type discriminator.
    pub item_type: u32,
    /// Number of attributes on this node.
    pub attribute_count: u16,
    /// Number of direct children of this node.
    pub child_count: u16,
    /// Index of the parent node, or -1 for the root.
    pub parent_index: i32,
    /// Index into the attribute table where this node's attributes begin.
    pub first_attribute_index: i32,
    /// Index into the child-indices table where this node's children begin.
    pub first_child_index: i32,
    /// Reserved / unused.
    pub reserved: i32,
}

/// A key-value attribute pair (8 bytes, packed).
#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct CryXmlAttribute {
    /// Byte offset into the string data section for the attribute key.
    pub key_string_offset: u32,
    /// Byte offset into the string data section for the attribute value.
    pub value_string_offset: u32,
}
