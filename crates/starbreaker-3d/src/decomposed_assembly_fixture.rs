//! Synthetic DataCore entity and triangle IVO for assembly texture-loading tests.
use starbreaker_datacore::dcb_builder::DcbBuilder;
use starbreaker_datacore::loadout::LoadoutNode;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, Pointer, PropertyDefinition, Record, StringId2, StructDefinition,
};

pub(super) const GEOMETRY: &str = "Test/synthetic.cgf";
pub(super) const MATERIAL: &str = "Test/synthetic.mtl";

pub(super) fn database_bytes() -> Vec<u8> {
    let mut b = DcbBuilder::new();
    let filename = b.add_string1("synthetic.xml");
    let geometry = b.add_string1(GEOMETRY);
    let material = b.add_string1(MATERIAL);
    // Entity.Components[] -> SGeometryResourceParams.Geometry.Geometry,
    // whose inline Geometry.path and Material.path fields contain the two strings.
    for (name, parent, count, first, size) in [
        ("EntityClassDefinition", -1, 1, 0, 8),
        ("Component", -1, 0, 1, 0),
        ("SGeometryResourceParams", 1, 1, 1, 8),
        ("GeometryOuter", -1, 1, 2, 8),
        ("GeometryMiddle", -1, 2, 3, 8),
        ("Path", -1, 1, 5, 4),
    ] {
        let name_offset = b.add_string2(name);
        b.struct_defs.push(StructDefinition {
            name_offset,
            parent_type_index: parent,
            attribute_count: count,
            first_attribute_index: first,
            struct_size: size,
        });
    }
    for (name, struct_index, data_type, conversion_type) in [
        ("Components", 1, 0x110, 1),
        ("Geometry", 3, 0x10, 0),
        ("Geometry", 4, 0x10, 0),
        ("Geometry", 5, 0x10, 0),
        ("Material", 5, 0x10, 0),
        ("path", 0, 0x0a, 0),
    ] {
        let name_offset = b.add_string2(name);
        b.property_defs.push(PropertyDefinition {
            name_offset,
            struct_index,
            data_type,
            conversion_type,
            _padding: 0,
        });
    }
    for struct_index in 0..6 {
        b.data_mappings.push(DataMapping {
            struct_count: if struct_index == 0 {
                2
            } else if struct_index == 2 {
                1
            } else {
                0
            },
            struct_index,
        });
    }
    for (i, name) in [
        "EntityClassDefinition.synthetic_fps_weapon",
        "EntityClassDefinition.synthetic_attachment",
    ]
    .into_iter()
    .enumerate()
    {
        let name_offset = b.add_string2(name);
        b.records.push(Record {
            name_offset,
            file_name_offset: filename,
            tag_offset: StringId2(-1),
            struct_index: 0,
            id: CigGuid::from_bytes([i as u8 + 1; 16]),
            instance_index: i as u16,
            struct_size: 8,
        });
        b.instance_data.extend_from_slice(&1_i32.to_le_bytes());
        b.instance_data.extend_from_slice(&0_i32.to_le_bytes());
    }
    b.strong_values.push(Pointer {
        struct_index: 2,
        instance_index: 0,
    });
    b.instance_data.extend_from_slice(&geometry.0.to_le_bytes());
    b.instance_data.extend_from_slice(&material.0.to_le_bytes());
    b.build()
}

pub(super) fn node(record: Record, name: &str) -> LoadoutNode {
    LoadoutNode {
        record,
        entity_name: name.into(),
        item_port_name: name.into(),
        helper_bone_name: None,
        no_rotation: false,
        offset_position: [0.0; 3],
        offset_rotation: [0.0; 3],
        detach_direction: [0.0; 3],
        port_flags: String::new(),
        port_tags: String::new(),
        geometry_path: Some(GEOMETRY.into()),
        material_path: Some(MATERIAL.into()),
        children: vec![],
    }
}

pub(super) fn mesh_bytes() -> Vec<u8> {
    fn put(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut skin = vec![0; 168 + 48];
    // MeshInfo: three vertices, three indices, one submesh.
    for (offset, value) in [(8, 3), (12, 3), (16, 1), (176, 3), (188, 3)] {
        put(&mut skin, offset, value);
    }
    skin.extend_from_slice(&crate::ivo::skin::stream_type::IVOVERTSUVS.to_le_bytes());
    skin.extend_from_slice(&20_u32.to_le_bytes());
    for position in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in position {
            skin.extend_from_slice(&value.to_le_bytes());
        }
        skin.extend_from_slice(&[255; 4]);
        skin.extend_from_slice(&[0; 4]);
    }
    skin.extend_from_slice(&[0; 4]); // vertex-stream alignment to eight bytes
    skin.extend_from_slice(&crate::ivo::skin::stream_type::IVOINDICES.to_le_bytes());
    skin.extend_from_slice(&4_u32.to_le_bytes());
    for index in [0_u32, 1, 2] {
        skin.extend_from_slice(&index.to_le_bytes());
    }
    skin.extend_from_slice(&[0; 4]);
    let mut bytes = vec![0; 32];
    bytes[..4].copy_from_slice(b"#ivo");
    for (offset, value) in [
        (4, 0x900),
        (8, 1),
        (12, 16),
        (16, 0xB8757777),
        (20, 0x900),
        (24, 32),
    ] {
        put(&mut bytes, offset, value);
    }
    bytes.extend(skin);
    bytes
}
