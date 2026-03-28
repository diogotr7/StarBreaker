mod test_helpers;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::error::QueryError;
use starbreaker_datacore::query::compile::compile_path;
use starbreaker_datacore::query::execute::{query_all, query_one};
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, Pointer, PropertyDefinition, Record, StructDefinition,
};
use test_helpers::DcbBuilder;

// ─── Schema ──────────────────────────────────────────────────────────────────
//
// Same 7-struct entity→geometry schema as in query_compile_test.rs:
//
// struct 0: Entity
//   name: String(Attribute)                              prop 0
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
// Instance data layout:
//   Entity (12 bytes): name StringId(4) + Components array header (count i32 + first_index i32 = 8)
//   SGeometryResourceParams (8 bytes): GeometryInner.path StringId(4) + MaterialRef.path StringId(4)

fn build_test_db() -> Vec<u8> {
    let mut b = DcbBuilder::new();

    // ── String table 1 (filenames / string values) ──────────────────────────
    // Must create string entries BEFORE writing their StringIds into instance_data.
    let file_name = b.add_string1("test.dcb"); // offset 0
    let entity_name_sid = b.add_string1("test_entity"); // offset 9  (after "test.dcb\0")
    let geom_path_sid = b.add_string1("objects/test.skin"); // offset 21 (after "test_entity\0")
    let mtl_path_sid = b.add_string1("objects/test.mtl"); // offset 39 (after "objects/test.skin\0")

    // ── String table 2 (names) ──────────────────────────────────────────────
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

    // ── Struct definitions ──────────────────────────────────────────────────
    // struct 0: Entity — props 0..1 (2 own props, parent=-1)
    b.struct_defs.push(StructDefinition {
        name_offset: s_entity,
        parent_type_index: -1,
        attribute_count: 2,
        first_attribute_index: 0,
        struct_size: 12,
    });

    // struct 1: Component — 0 own props, parent=-1
    b.struct_defs.push(StructDefinition {
        name_offset: s_component,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 2,
        struct_size: 0,
    });

    // struct 2: SGeometryResourceParams — 1 own prop (prop 2), parent=1 (Component)
    b.struct_defs.push(StructDefinition {
        name_offset: s_sgeom,
        parent_type_index: 1,
        attribute_count: 1,
        first_attribute_index: 2,
        struct_size: 8,
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

    // ── Property definitions ────────────────────────────────────────────────
    // prop 0: Entity.name — String(0x000A), Attribute(0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: p_name,
        struct_index: 0,
        data_type: 0x000A,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 1: Entity.Components — StrongPointer(0x0110), ComplexArray(0x01), struct_index=1
    b.property_defs.push(PropertyDefinition {
        name_offset: p_components,
        struct_index: 1,
        data_type: 0x0110,
        conversion_type: 0x01,
        _padding: 0,
    });

    // prop 2: SGeometryResourceParams.Geometry — Class(0x0010), Attribute(0x00), struct_index=3
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 3,
        data_type: 0x0010,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 3: GeometryOuter.Geometry — Class(0x0010), Attribute(0x00), struct_index=4
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 4,
        data_type: 0x0010,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 4: GeometryMiddle.Geometry — Class(0x0010), Attribute(0x00), struct_index=5
    b.property_defs.push(PropertyDefinition {
        name_offset: p_geometry,
        struct_index: 5,
        data_type: 0x0010,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 5: GeometryMiddle.Material — Class(0x0010), Attribute(0x00), struct_index=6
    b.property_defs.push(PropertyDefinition {
        name_offset: p_material,
        struct_index: 6,
        data_type: 0x0010,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 6: GeometryInner.path — String(0x000A), Attribute(0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: p_path,
        struct_index: 0,
        data_type: 0x000A,
        conversion_type: 0x00,
        _padding: 0,
    });

    // prop 7: MaterialRef.path — String(0x000A), Attribute(0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: p_path,
        struct_index: 0,
        data_type: 0x000A,
        conversion_type: 0x00,
        _padding: 0,
    });

    // ── Data mappings ───────────────────────────────────────────────────────
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 0,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 1,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 2,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 3,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 4,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 5,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 0,
        struct_index: 6,
    });

    // ── Record ──────────────────────────────────────────────────────────────
    let record_id = CigGuid::from_bytes([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.records.push(Record {
        name_offset: s_entity,
        file_name_offset: file_name,
        struct_index: 0,
        id: record_id,
        instance_index: 0,
        struct_size: 12,
    });

    // ── Strong pointer values ───────────────────────────────────────────────
    // strong_values[0] = Pointer { struct_index: 2, instance_index: 0 }
    b.strong_values.push(Pointer {
        struct_index: 2,
        instance_index: 0,
    });

    // ── Instance data ───────────────────────────────────────────────────────
    // Entity instance 0 (12 bytes):
    //   name: StringId pointing to "test_entity"
    //   Components array header: count=1, first_index=0
    b.instance_data
        .extend_from_slice(&entity_name_sid.0.to_le_bytes()); // 4 bytes: StringId
    b.instance_data.extend_from_slice(&1i32.to_le_bytes()); // 4 bytes: count=1
    b.instance_data.extend_from_slice(&0i32.to_le_bytes()); // 4 bytes: first_index=0

    // SGeometryResourceParams instance 0 (8 bytes):
    //   Geometry is Class → inlined GeometryOuter (8 bytes)
    //     which contains GeometryMiddle (8 bytes), inlined:
    //       Geometry: GeometryInner (4 bytes): path StringId
    //       Material: MaterialRef (4 bytes): path StringId
    b.instance_data
        .extend_from_slice(&geom_path_sid.0.to_le_bytes()); // 4 bytes: geometry path
    b.instance_data
        .extend_from_slice(&mtl_path_sid.0.to_le_bytes()); // 4 bytes: material path

    b.build()
}

// ─── Test 1: query_simple_string_field ──────────────────────────────────────

#[test]
fn query_simple_string_field() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];

    let path = compile_path::<String>(&db, 0, "name").unwrap();
    let result = query_one::<String>(&db, &path, record).unwrap();
    assert_eq!(result, "test_entity");
}

// ─── Test 2: query_geometry_path ────────────────────────────────────────────

#[test]
fn query_geometry_path() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];

    let path = compile_path::<String>(
        &db,
        0,
        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    )
    .unwrap();
    let result = query_one::<String>(&db, &path, record).unwrap();
    assert_eq!(result, "objects/test.skin");
}

// ─── Test 3: query_material_path ────────────────────────────────────────────

#[test]
fn query_material_path() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];

    let path = compile_path::<String>(
        &db,
        0,
        "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    )
    .unwrap();
    let result = query_one::<String>(&db, &path, record).unwrap();
    assert_eq!(result, "objects/test.mtl");
}

// ─── Test 4: query_all_returns_vec ──────────────────────────────────────────

#[test]
fn query_all_returns_vec() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];

    let path = compile_path::<String>(
        &db,
        0,
        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    )
    .unwrap();
    let results = query_all::<String>(&db, &path, record).unwrap();
    assert_eq!(results, vec!["objects/test.skin"]);
}

// ─── Test 5: query_one_struct_mismatch ──────────────────────────────────────

#[test]
fn query_one_struct_mismatch() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0]; // struct_index=0 (Entity)

    // Compile for struct 2 (SGeometryResourceParams), query against Entity record
    let path = compile_path::<String>(&db, 2, "Geometry.Geometry.Geometry.path").unwrap();

    match query_one::<String>(&db, &path, record) {
        Err(QueryError::StructMismatch { expected, actual }) => {
            assert_eq!(expected, 2);
            assert_eq!(actual, 0);
        }
        other => panic!("expected StructMismatch, got: {other:?}"),
    }
}

// ─── Test 6: query_value_sub_tree ──────────────────────────────────────────

#[test]
fn query_value_sub_tree() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];

    let path = db
        .compile_path::<Value>(db.struct_id("Entity").unwrap(), "Components[SGeometryResourceParams]")
        .unwrap();
    let val: Value = db.query_single(&path, record).unwrap().unwrap();

    match &val {
        Value::Object { type_name, fields, .. } => {
            assert_eq!(*type_name, "SGeometryResourceParams");
            assert!(fields.iter().any(|(name, _)| *name == "Geometry"));
        }
        other => panic!("expected Object, got {:?}", other),
    }
}

// ─── Test 7: query_via_database_methods ────────────────────────────────────

#[test]
fn query_via_database_methods() {
    let data = build_test_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[0];
    let path = db.compile_path::<String>(db.struct_id("Entity").unwrap(), "name").unwrap();
    let val: String = db.query_single(&path, record).unwrap().unwrap();
    assert_eq!(val, "test_entity");
}
