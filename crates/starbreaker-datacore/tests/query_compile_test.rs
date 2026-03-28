mod test_helpers;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::enums::DataType;
use starbreaker_datacore::error::QueryError;
use starbreaker_datacore::query::compile::compile_path;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, Pointer, PropertyDefinition, Record, StructDefinition,
};
use test_helpers::DcbBuilder;

// ─── Schema ──────────────────────────────────────────────────────────────────
//
// struct 0: Entity
//   name: String(Attribute)               prop 0
//   Components: StrongPointer[](ComplexArray) → Component(struct 1)  prop 1
//
// struct 1: Component   (no own fields, parent=-1)
//
// struct 2: SGeometryResourceParams  (parent = Component = 1)
//   Geometry: Class(Attribute) → GeometryOuter(struct 3)  prop 2
//
// struct 3: GeometryOuter  (parent=-1)
//   Geometry: Class(Attribute) → GeometryMiddle(struct 4)  prop 3
//
// struct 4: GeometryMiddle (parent=-1)
//   Geometry: Class(Attribute) → GeometryInner(struct 5)  prop 4
//   Material: Class(Attribute) → MaterialRef(struct 6)    prop 5
//
// struct 5: GeometryInner (parent=-1)
//   path: String(Attribute)  prop 6
//
// struct 6: MaterialRef  (parent=-1)
//   path: String(Attribute)  prop 7
//
// Struct sizes:
//   Entity: 12  (4 StringId + 8 array header [count u32 + first_index u32])
//   Component: 0
//   SGeometryResourceParams: 8  (GeometryInner.path 4 + MaterialRef.path 4)
//   GeometryOuter: 8  (GeometryInner.path 4 + MaterialRef.path 4)
//   GeometryMiddle: 8 (GeometryInner.path 4 + MaterialRef.path 4)
//   GeometryInner: 4  (path StringId)
//   MaterialRef: 4    (path StringId)

fn build_test_db() -> Vec<u8> {
    let mut b = DcbBuilder::new();

    // ── String table 1 (filenames) ───────────────────────────────────────────
    let file_name = b.add_string1("test.dcb");

    // ── String table 2 (names) ───────────────────────────────────────────────
    let s_entity = b.add_string2("Entity");
    let s_component = b.add_string2("Component");
    let s_sgeom = b.add_string2("SGeometryResourceParams");
    let s_geom_outer = b.add_string2("GeometryOuter");
    let s_geom_middle = b.add_string2("GeometryMiddle");
    let s_geom_inner = b.add_string2("GeometryInner");
    let s_matref = b.add_string2("MaterialRef");

    // Property names
    let p_name = b.add_string2("name");
    let p_components = b.add_string2("Components");
    let p_geometry = b.add_string2("Geometry");
    let p_material = b.add_string2("Material");
    let p_path = b.add_string2("path");

    // ── Struct definitions ───────────────────────────────────────────────────
    // struct 0: Entity — props 0..1 (2 own props, parent=-1)
    b.struct_defs.push(StructDefinition {
        name_offset: s_entity,
        parent_type_index: -1,
        attribute_count: 2,
        first_attribute_index: 0,
        struct_size: 12, // 4 (StringId) + 8 (array header: count u32 + first_index u32)
    });

    // struct 1: Component — 0 own props, parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_component,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 2, // no properties
        struct_size: 0,
    });

    // struct 2: SGeometryResourceParams — 1 own prop (prop 2), parent=1 (Component)
    b.struct_defs.push(StructDefinition {
        name_offset: s_sgeom,
        parent_type_index: 1,
        attribute_count: 1,
        first_attribute_index: 2,
        struct_size: 8, // 4 (geom_path StringId) + 4 (mtl_path StringId)
    });

    // struct 3: GeometryOuter — 1 own prop (prop 3), parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_geom_outer,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 3,
        struct_size: 8,
    });

    // struct 4: GeometryMiddle — 2 own props (props 4,5), parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_geom_middle,
        parent_type_index: -1,
        attribute_count: 2,
        first_attribute_index: 4,
        struct_size: 8,
    });

    // struct 5: GeometryInner — 1 own prop (prop 6), parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_geom_inner,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 6,
        struct_size: 4,
    });

    // struct 6: MaterialRef — 1 own prop (prop 7), parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_matref,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 7,
        struct_size: 4,
    });

    // ── Property definitions ─────────────────────────────────────────────────
    // prop 0: Entity.name — String(0x000A), Attribute(0x00), struct_index=0
    b.property_defs.push(PropertyDefinition {
        name_offset: p_name,
        struct_index: 0,
        data_type: 0x000A,     // String
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 1: Entity.Components — StrongPointer(0x0110), ComplexArray(0x01), struct_index=1
    b.property_defs.push(PropertyDefinition {
        name_offset: p_components,
        struct_index: 1,
        data_type: 0x0110,     // StrongPointer
        conversion_type: 0x01, // ComplexArray
        _padding: 0,
    });

    // prop 2: SGeometryResourceParams.Geometry — Class(0x0010), Attribute(0x00), struct_index=3
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 3,
        data_type: 0x0010,     // Class
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 3: GeometryOuter.Geometry — Class(0x0010), Attribute(0x00), struct_index=4
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 4,
        data_type: 0x0010,     // Class
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 4: GeometryMiddle.Geometry — Class(0x0010), Attribute(0x00), struct_index=5
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 5,
        data_type: 0x0010,     // Class
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 5: GeometryMiddle.Material — Class(0x0010), Attribute(0x00), struct_index=6
    b.property_defs.push(PropertyDefinition {
        name_offset: p_material,
        struct_index: 6,
        data_type: 0x0010,     // Class
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 6: GeometryInner.path — String(0x000A), Attribute(0x00), struct_index=0
    b.property_defs.push(PropertyDefinition {
        name_offset: p_path,
        struct_index: 0,
        data_type: 0x000A,     // String
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // prop 7: MaterialRef.path — String(0x000A), Attribute(0x00), struct_index=0
    b.property_defs.push(PropertyDefinition {
        name_offset: p_path,
        struct_index: 0,
        data_type: 0x000A,     // String
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // ── Data mappings ────────────────────────────────────────────────────────
    // 1 instance of Entity (struct 0)
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 0,
    });
    // 0 instances of Component (struct 1)
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 1,
    });
    // 1 instance of SGeometryResourceParams (struct 2)
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 2,
    });
    // 0 instances of GeometryOuter (struct 3) — inlined
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 3,
    });
    // 0 instances of GeometryMiddle (struct 4) — inlined
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 4,
    });
    // 0 instances of GeometryInner (struct 5) — inlined
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 5,
    });
    // 0 instances of MaterialRef (struct 6) — inlined
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 6,
    });

    // ── Record ───────────────────────────────────────────────────────────────
    let record_id = CigGuid::from_bytes([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.records.push(Record {
        name_offset: s_entity,
        file_name_offset: file_name,
        struct_index: 0, // Entity
        id: record_id,
        instance_index: 0,
        struct_size: 12,
    });

    // ── String values (table 1, via StringId) ───────────────────────────────
    // string_id_values[0] = "EntityName" (for Entity.name field)
    let entity_name_str = b.add_string1("EntityName");
    b.string_id_values.push(entity_name_str);

    // ── Strong pointer values ────────────────────────────────────────────────
    // strong_values[0] = Pointer { struct_index: 2, instance_index: 0 }
    //   (pointing to instance 0 of SGeometryResourceParams)
    b.strong_values.push(Pointer {
        struct_index: 2,
        instance_index: 0,
    });

    // ── Instance data ────────────────────────────────────────────────────────
    // Entity instance 0 (12 bytes):
    //   name: StringId = entity_name_str (i32)
    //   Components array header: count=1 (u32), first_index=0 (u32)
    b.instance_data
        .extend_from_slice(&entity_name_str.0.to_le_bytes()); // 4 bytes
    b.instance_data.extend_from_slice(&1u32.to_le_bytes()); // count=1
    b.instance_data.extend_from_slice(&0u32.to_le_bytes()); // first_index=0

    // SGeometryResourceParams instance 0 (8 bytes):
    //   Geometry is Class → inlined GeometryOuter (8 bytes)
    //     which contains GeometryMiddle (8 bytes), inlined:
    //       Geometry: GeometryInner (4 bytes): path StringId
    //       Material: MaterialRef (4 bytes): path StringId
    let geom_path_str = b.add_string1("data/geometry.cgf");
    let mtl_path_str = b.add_string1("data/material.mtl");
    b.instance_data
        .extend_from_slice(&geom_path_str.0.to_le_bytes()); // geom path (4 bytes)
    b.instance_data
        .extend_from_slice(&mtl_path_str.0.to_le_bytes()); // mtl path (4 bytes)

    b.build()
}

// ─── Test 1: compile_simple_field ────────────────────────────────────────────

#[test]
fn compile_simple_field() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    let compiled = compile_path::<String>(&db, 0, "name").unwrap();

    assert_eq!(compiled.segments().len(), 1);
    assert_eq!(compiled.root_struct_id(), db.struct_id("Entity").unwrap());
    assert_eq!(compiled.leaf_data_type(), DataType::String);

    let seg = &compiled.segments()[0];
    assert_eq!(seg.context_struct_index(), 0);
    assert_eq!(seg.property_position(), 0); // first property of Entity
    assert_eq!(seg.data_type(), DataType::String);
}

// ─── Test 2: compile_full_geometry_path ──────────────────────────────────────

#[test]
fn compile_full_geometry_path() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    let compiled = compile_path::<String>(
        &db,
        0,
        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    )
    .unwrap();

    assert_eq!(compiled.segments().len(), 5);
    assert_eq!(compiled.root_struct_id(), db.struct_id("Entity").unwrap());
    assert_eq!(compiled.leaf_data_type(), DataType::String);

    let segs = compiled.segments();

    // Segment 0: Entity.Components[SGeometryResourceParams]
    assert_eq!(segs[0].context_struct_index(), 0);
    assert_eq!(segs[0].data_type(), DataType::StrongPointer);
    assert_eq!(segs[0].type_filter_struct_index(), Some(2)); // SGeometryResourceParams
    assert_eq!(segs[0].target_struct_index(), Some(2));

    // Segment 1: SGeometryResourceParams.Geometry (→ GeometryOuter)
    assert_eq!(segs[1].context_struct_index(), 2);
    assert_eq!(segs[1].data_type(), DataType::Class);
    assert_eq!(segs[1].target_struct_index(), Some(3)); // GeometryOuter

    // Segment 2: GeometryOuter.Geometry (→ GeometryMiddle)
    assert_eq!(segs[2].context_struct_index(), 3);
    assert_eq!(segs[2].data_type(), DataType::Class);
    assert_eq!(segs[2].target_struct_index(), Some(4)); // GeometryMiddle

    // Segment 3: GeometryMiddle.Geometry (→ GeometryInner)
    assert_eq!(segs[3].context_struct_index(), 4);
    assert_eq!(segs[3].data_type(), DataType::Class);
    assert_eq!(segs[3].target_struct_index(), Some(5)); // GeometryInner

    // Segment 4: GeometryInner.path
    assert_eq!(segs[4].context_struct_index(), 5);
    assert_eq!(segs[4].data_type(), DataType::String);
    assert_eq!(segs[4].target_struct_index(), None);
}

// ─── Test 3: compile_material_path ───────────────────────────────────────────

#[test]
fn compile_material_path() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    let compiled = compile_path::<String>(
        &db,
        0,
        "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    )
    .unwrap();

    assert_eq!(compiled.segments().len(), 5);
    assert_eq!(compiled.leaf_data_type(), DataType::String);

    let segs = compiled.segments();

    // Last segment before path should be MaterialRef
    assert_eq!(segs[3].context_struct_index(), 4); // GeometryMiddle
    assert_eq!(segs[3].data_type(), DataType::Class);
    assert_eq!(segs[3].target_struct_index(), Some(6)); // MaterialRef

    assert_eq!(segs[4].context_struct_index(), 6); // MaterialRef
    assert_eq!(segs[4].data_type(), DataType::String);
}

// ─── Test 4: compile_property_not_found ──────────────────────────────────────

#[test]
fn compile_property_not_found() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    let result = compile_path::<String>(&db, 0, "nonexistent");

    match result {
        Err(QueryError::PropertyNotFound {
            property,
            struct_name,
        }) => {
            assert_eq!(property, "nonexistent");
            assert_eq!(struct_name, "Entity");
        }
        other => panic!("expected PropertyNotFound, got: {other:?}"),
    }
}

// ─── Test 5: compile_type_filter_required ────────────────────────────────────

#[test]
fn compile_type_filter_required() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    // Components is a polymorphic StrongPointer array — no type filter → error
    let result = compile_path::<String>(&db, 0, "Components[].Geometry");

    match result {
        Err(QueryError::TypeFilterRequired { property }) => {
            assert_eq!(property, "Components");
        }
        other => panic!("expected TypeFilterRequired, got: {other:?}"),
    }
}

// ─── Test 6: compile_type_filter_mismatch ────────────────────────────────────

#[test]
fn compile_type_filter_mismatch() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    // GeometryOuter does NOT inherit from Component (struct 1)
    let result = compile_path::<String>(&db, 0, "Components[GeometryOuter].Geometry.path");

    match result {
        Err(QueryError::TypeFilterMismatch { filter, expected }) => {
            assert_eq!(filter, "GeometryOuter");
            assert_eq!(expected, "Component");
        }
        other => panic!("expected TypeFilterMismatch, got: {other:?}"),
    }
}

// ─── Test 7: compile_type_filter_on_scalar ───────────────────────────────────

#[test]
fn compile_type_filter_on_scalar() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    // "name" is a scalar String, not an array — type filter not allowed
    let result = compile_path::<String>(&db, 0, "name[Foo]");

    match result {
        Err(QueryError::TypeFilterOnScalar { property }) => {
            assert_eq!(property, "name");
        }
        other => panic!("expected TypeFilterOnScalar, got: {other:?}"),
    }
}

// ─── Test 8: compile_leaf_type_mismatch ──────────────────────────────────────

#[test]
fn compile_leaf_type_mismatch() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();

    // "name" is a String, but we ask for i32 — should error with LeafTypeMismatch
    let result = compile_path::<i32>(&db, 0, "name");

    match result {
        Err(QueryError::LeafTypeMismatch {
            property,
            expected,
            actual,
        }) => {
            assert_eq!(property, "name");
            assert_eq!(actual, DataType::String);
            assert!(expected.contains(&DataType::Int32));
        }
        other => panic!("expected LeafTypeMismatch, got: {other:?}"),
    }
}
