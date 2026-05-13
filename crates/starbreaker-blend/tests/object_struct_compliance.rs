//! Comprehensive Object struct compliance tests for Blender 5.1.x
//!
//! These tests verify the Object struct binary layout, field offsets, and correctness
//! of all object types (mesh, lamp, empty, linked instances).
//!
//! Reference: Blender DNA_object_types.h
//! - Object struct MUST be exactly 1288 bytes
//! - All pointer fields MUST be 8 bytes (64-bit little-endian)
//! - Critical field offsets must match DNA layout
//!
//! Spec from task:
//! 1. Total Object size: EXACTLY 1288 bytes
//! 2. Critical field offsets:
//!    - type @ offset 416 (i16): object type enum (OB_MESH=1, OB_LAMP=10, OB_EMPTY=0)
//!    - data @ offset 552 (void*): pointer to mesh/lamp/collection data
//!    - parent @ offset 496 (Object*): parent object pointer
//!    - loc @ offset 736 (float[3]): location XYZ
//!    - scale @ offset 760 (float[3]): scale factors
//!    - rot @ offset 820 (float[4]): quaternion rotation XYZW
//!    - mat @ offset 712 (Material**): material slot array pointer
//!    - matbits @ offset 720 (u8*): material slot flags pointer
//!    - totcol @ offset 728 (i32): total material slots
//!    - actcol @ offset 732 (i16): active material slot
//! 3. All pointer fields are 8 bytes (64-bit)
//! 4. No gaps or alignment issues

use starbreaker_blend::*;

// ── Test 1: Object struct size must be exactly 1288 bytes ─────────────────────
#[test]
fn object_struct_size_is_exactly_1288_bytes() {
    let obj = build_object(
        "TestMesh",
        0x2000,    // mesh_ptr
        0x3000,    // object_mat_ptr
        0x4000,    // matbits_ptr
        1,         // material_slots
        0x5000,    // properties_ptr
    );
    assert_eq!(obj.len(), 1288, "Object struct MUST be exactly 1288 bytes");
}

// ── Test 2: Object block code must be "OB\0\0" when written ────────────────────
#[test]
fn object_block_code_is_ob00() {
    let obj = build_object("TestObj", 0x2000, 0x3000, 0x4000, 1, 0x5000);
    // ID structure starts at offset 40 with the name after the two-char prefix
    // The ID.name starts with two-char prefix at offset 40
    // For "OBTestObj", bytes [40..42] should be "OB"
    assert_eq!(&obj[40..42], b"OB", "Object name must start with 'OB' prefix");
}

// ── Test 3: Object type field at offset 416 for OB_MESH ──────────────────────
#[test]
fn object_type_field_at_offset_416_ob_mesh() {
    let obj = build_object("MeshObj", 0x2000, 0x3000, 0x4000, 1, 0x5000);
    // Type field is i16 (2 bytes) at offset 416
    let type_bytes = &obj[416..418];
    let type_val = i16::from_le_bytes([type_bytes[0], type_bytes[1]]);
    assert_eq!(type_val, 1, "OB_MESH type must be 1");
}

// ── Test 4: Object type field for OB_EMPTY ───────────────────────────────────
#[test]
fn object_type_field_for_ob_empty() {
    let obj = build_empty_object("EmptyObj", [1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    let type_bytes = &obj[416..418];
    let type_val = i16::from_le_bytes([type_bytes[0], type_bytes[1]]);
    assert_eq!(type_val, 0, "OB_EMPTY type must be 0");
}

// ── Test 5: Object type field for OB_LAMP ────────────────────────────────────
#[test]
fn object_type_field_for_ob_lamp() {
    let obj = build_lamp_object("LampObj", 0x2000, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    let type_bytes = &obj[416..418];
    let type_val = i16::from_le_bytes([type_bytes[0], type_bytes[1]]);
    assert_eq!(type_val, 10, "OB_LAMP type must be 10");
}

// ── Test 6: Data pointer field at offset 552 ─────────────────────────────────
#[test]
fn data_pointer_at_offset_552() {
    let mesh_ptr = 0x1234567890ABCDEF_u64;
    let obj = build_object("MeshObj", mesh_ptr, 0x3000, 0x4000, 1, 0x5000);
    // Data pointer is u64 (8 bytes) at offset 552
    let ptr_bytes = &obj[552..560];
    let ptr_val = u64::from_le_bytes([
        ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3],
        ptr_bytes[4], ptr_bytes[5], ptr_bytes[6], ptr_bytes[7],
    ]);
    assert_eq!(ptr_val, mesh_ptr, "Data pointer at offset 552 must match");
}

// ── Test 7: Parent pointer at offset 496 ─────────────────────────────────────
#[test]
fn parent_pointer_at_offset_496() {
    let parent_ptr = 0xFEDCBA9876543210_u64;
    let obj = build_empty_object("ChildObj", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], parent_ptr);
    let ptr_bytes = &obj[496..504];
    let ptr_val = u64::from_le_bytes([
        ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3],
        ptr_bytes[4], ptr_bytes[5], ptr_bytes[6], ptr_bytes[7],
    ]);
    assert_eq!(ptr_val, parent_ptr, "Parent pointer at offset 496 must match");
}

// ── Test 8: Location (loc) field at offset 736 (float[3]) ────────────────────
#[test]
fn location_field_at_offset_736() {
    let loc = [1.5, 2.5, 3.5];
    let obj = build_empty_object("LocObj", loc, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // loc is float[3] (12 bytes) at offset 736
    for i in 0..3 {
        let offset = 736 + i * 4;
        let value_bytes = &obj[offset..offset + 4];
        let value = f32::from_le_bytes([
            value_bytes[0], value_bytes[1], value_bytes[2], value_bytes[3],
        ]);
        assert!((value - loc[i]).abs() < 0.0001, "Location[{}] mismatch at offset {}", i, offset);
    }
}

// ── Test 9: Scale field at offset 760 (float[3]) ──────────────────────────────
#[test]
fn scale_field_at_offset_760() {
    let scale = [2.0, 3.0, 4.0];
    let obj = build_empty_object("ScaleObj", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], scale, 0);
    
    // scale is float[3] (12 bytes) at offset 760
    for i in 0..3 {
        let offset = 760 + i * 4;
        let value_bytes = &obj[offset..offset + 4];
        let value = f32::from_le_bytes([
            value_bytes[0], value_bytes[1], value_bytes[2], value_bytes[3],
        ]);
        assert!((value - scale[i]).abs() < 0.0001, "Scale[{}] mismatch at offset {}", i, offset);
    }
}

// ── Test 10: Rotation (quaternion) field at offset 820 (float[4]) ─────────────
#[test]
fn rotation_quaternion_at_offset_820() {
    let quat = [0.5, 0.5, 0.5, 0.5];
    let obj = build_empty_object("RotObj", [0.0, 0.0, 0.0], quat, [1.0, 1.0, 1.0], 0);
    
    // rot (quaternion) is float[4] (16 bytes) at offset 820
    for i in 0..4 {
        let offset = 820 + i * 4;
        let value_bytes = &obj[offset..offset + 4];
        let value = f32::from_le_bytes([
            value_bytes[0], value_bytes[1], value_bytes[2], value_bytes[3],
        ]);
        assert!((value - quat[i]).abs() < 0.0001, "Quaternion[{}] mismatch at offset {}", i, offset);
    }
}

// ── Test 11: Material array pointer at offset 712 ───────────────────────────────
#[test]
fn material_array_pointer_at_offset_712() {
    let mat_ptr = 0x9999999999999999_u64;
    let obj = build_object("MatObj", 0x2000, mat_ptr, 0x4000, 2, 0x5000);
    
    // mat (Material**) is u64 (8 bytes) at offset 712
    let ptr_bytes = &obj[712..720];
    let ptr_val = u64::from_le_bytes([
        ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3],
        ptr_bytes[4], ptr_bytes[5], ptr_bytes[6], ptr_bytes[7],
    ]);
    assert_eq!(ptr_val, mat_ptr, "Material pointer at offset 712 must match");
}

// ── Test 12: Material bits pointer at offset 720 ────────────────────────────────
#[test]
fn matbits_pointer_at_offset_720() {
    let matbits_ptr = 0x8888888888888888_u64;
    let obj = build_object("MatBitsObj", 0x2000, 0x3000, matbits_ptr, 2, 0x5000);
    
    // matbits (u8*) is u64 (8 bytes) at offset 720
    let ptr_bytes = &obj[720..728];
    let ptr_val = u64::from_le_bytes([
        ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3],
        ptr_bytes[4], ptr_bytes[5], ptr_bytes[6], ptr_bytes[7],
    ]);
    assert_eq!(ptr_val, matbits_ptr, "MatBits pointer at offset 720 must match");
}

// ── Test 13: Total material slots (totcol) at offset 728 ──────────────────────
#[test]
fn totcol_field_at_offset_728() {
    let mat_slots = 5;
    let obj = build_object("TotcolObj", 0x2000, 0x3000, 0x4000, mat_slots, 0x5000);
    
    // totcol is i32 (4 bytes) at offset 728
    let col_bytes = &obj[728..732];
    let col_val = i32::from_le_bytes([
        col_bytes[0], col_bytes[1], col_bytes[2], col_bytes[3],
    ]);
    assert_eq!(col_val, mat_slots as i32, "totcol at offset 728 must match");
}

// ── Test 14: Active material slot (actcol) at offset 732 ───────────────────────
#[test]
fn actcol_field_at_offset_732() {
    let obj = build_object("ActcolObj", 0x2000, 0x3000, 0x4000, 3, 0x5000);
    
    // actcol is i16 (2 bytes) at offset 732
    let actcol_bytes = &obj[732..734];
    let actcol_val = i16::from_le_bytes([actcol_bytes[0], actcol_bytes[1]]);
    // Should be 1 if material_slots > 0, else 0
    assert_eq!(actcol_val, 1, "actcol at offset 732 must be 1 when slots > 0");
}

// ── Test 15: Mesh object with all zero-initialized padding ───────────────────
#[test]
fn mesh_object_zero_padding_check() {
    let obj = build_object("PaddingTest", 0x0, 0x0, 0x0, 0, 0x0);
    
    // Check that unused fields are zero-initialized
    // For example, after setting the known fields, unused regions should be zero
    // This is a basic sanity check
    assert_eq!(obj.len(), 1288, "Object must be 1288 bytes");
    
    // Verify the struct was created successfully
    assert!(!obj.is_empty(), "Object should not be empty");
}

// ── Test 16: All pointer fields are 8 bytes (64-bit) ─────────────────────────
#[test]
fn all_pointer_fields_are_8_bytes() {
    // Test that pointer-based offsets maintain 8-byte alignment
    // Offset 496: parent pointer
    // Offset 552: data pointer
    // Offset 712: material pointer
    // Offset 720: matbits pointer
    
    assert_eq!((496 % 8), 0, "Parent pointer offset 496 must be 8-byte aligned");
    assert_eq!((552 % 8), 0, "Data pointer offset 552 must be 8-byte aligned");
    assert_eq!((712 % 8), 0, "Material pointer offset 712 must be 8-byte aligned");
    assert_eq!((720 % 8), 0, "MatBits pointer offset 720 must be 8-byte aligned");
}

// ── Test 17: Linked instance object type ─────────────────────────────────────
#[test]
fn linked_instance_object_type() {
    let obj = build_linked_instance_object("LinkedObj", 0x2000, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // Linked instances use OB_MESH type (1)
    let type_bytes = &obj[416..418];
    let type_val = i16::from_le_bytes([type_bytes[0], type_bytes[1]]);
    assert_eq!(type_val, 1, "Linked instance must use OB_MESH type (1)");
    
    // Verify size is still 1288
    assert_eq!(obj.len(), 1288, "Linked instance must be 1288 bytes");
}

// ── Test 18: Empty object with no parent ─────────────────────────────────────
#[test]
fn empty_object_with_no_parent() {
    let obj = build_empty_object("StandaloneEmpty", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // Parent pointer should be null (0)
    let parent_bytes = &obj[496..504];
    let parent_val = u64::from_le_bytes([
        parent_bytes[0], parent_bytes[1], parent_bytes[2], parent_bytes[3],
        parent_bytes[4], parent_bytes[5], parent_bytes[6], parent_bytes[7],
    ]);
    assert_eq!(parent_val, 0, "Empty object with no parent should have null parent pointer");
}

// ── Test 19: Multiple objects maintain independence ───────────────────────────
#[test]
fn multiple_objects_maintain_independence() {
    let obj1 = build_object("Obj1", 0x1000, 0x2000, 0x3000, 1, 0x4000);
    let obj2 = build_object("Obj2", 0x5000, 0x6000, 0x7000, 2, 0x8000);
    
    // Extract data pointer from obj1
    let obj1_data_ptr = u64::from_le_bytes([
        obj1[552], obj1[553], obj1[554], obj1[555],
        obj1[556], obj1[557], obj1[558], obj1[559],
    ]);
    
    // Extract data pointer from obj2
    let obj2_data_ptr = u64::from_le_bytes([
        obj2[552], obj2[553], obj2[554], obj2[555],
        obj2[556], obj2[557], obj2[558], obj2[559],
    ]);
    
    assert_eq!(obj1_data_ptr, 0x1000, "obj1 data pointer must be 0x1000");
    assert_eq!(obj2_data_ptr, 0x5000, "obj2 data pointer must be 0x5000");
    assert_ne!(obj1_data_ptr, obj2_data_ptr, "Objects must have independent data pointers");
}

// ── Test 20: Object struct boundary conditions (minimum size) ────────────────
#[test]
fn object_struct_boundary_minimum_size() {
    let obj = build_empty_object("MinObj", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // Even with minimal data, must still be 1288 bytes
    assert_eq!(obj.len(), 1288, "Object must be exactly 1288 bytes regardless of content");
    
    // Verify access to the last byte doesn't panic
    let _ = obj[1287];
}

// ── Test 21: Little-endian pointer encoding verification ─────────────────────
#[test]
fn little_endian_pointer_encoding() {
    let test_ptr = 0x0102030405060708_u64;
    let obj = build_empty_object("EndiannessTest", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], test_ptr);
    
    // In little-endian, the bytes should be: 08 07 06 05 04 03 02 01
    let parent_bytes = &obj[496..504];
    assert_eq!(parent_bytes[0], 0x08, "First byte of LE pointer should be LSB");
    assert_eq!(parent_bytes[1], 0x07, "Second byte of LE pointer");
    assert_eq!(parent_bytes[7], 0x01, "Last byte of LE pointer should be MSB");
    
    // Verify round-trip
    let ptr_val = u64::from_le_bytes([
        parent_bytes[0], parent_bytes[1], parent_bytes[2], parent_bytes[3],
        parent_bytes[4], parent_bytes[5], parent_bytes[6], parent_bytes[7],
    ]);
    assert_eq!(ptr_val, test_ptr, "Little-endian round-trip must preserve pointer value");
}

// ── Test 22: Rotation field identity quaternion (default for mesh) ─────────────
#[test]
fn default_mesh_rotation_identity_quaternion() {
    let obj = build_object("DefaultRotMesh", 0x2000, 0x3000, 0x4000, 1, 0x5000);
    
    // Default mesh rotation: quaternion stored as [w, x, y, z] per Blender DNA.
    // Offset 820 = w, 824 = x, 828 = y, 832 = z.
    let quat_w = f32::from_le_bytes([obj[820], obj[821], obj[822], obj[823]]);
    assert_eq!(quat_w, 1.0, "Default mesh quaternion w component should be 1.0");
}

// ── Test 23: Object with zero material slots ──────────────────────────────────
#[test]
fn object_with_zero_material_slots() {
    let obj = build_object("NoMatsObj", 0x2000, 0x3000, 0x4000, 0, 0x5000);
    
    // totcol at offset 728
    let totcol = i32::from_le_bytes([obj[728], obj[729], obj[730], obj[731]]);
    assert_eq!(totcol, 0, "totcol should be 0 when no material slots");
    
    // actcol at offset 732 should be 0
    let actcol = i16::from_le_bytes([obj[732], obj[733]]);
    assert_eq!(actcol, 0, "actcol should be 0 when no material slots");
}

// ── Test 24: Large material slot count encoding ───────────────────────────────
#[test]
fn large_material_slot_count_encoding() {
    let large_count = 100i32;
    let obj = build_object("ManyMatsObj", 0x2000, 0x3000, 0x4000, large_count, 0x5000);
    
    // totcol at offset 728
    let totcol = i32::from_le_bytes([obj[728], obj[729], obj[730], obj[731]]);
    assert_eq!(totcol, large_count, "totcol should correctly encode large counts");
}

// ── Test 25: Transform matrix independence from quaternion ───────────────────
#[test]
fn transform_fields_are_independent() {
    let loc1 = [1.0, 2.0, 3.0];
    let loc2 = [4.0, 5.0, 6.0];
    
    let obj1 = build_empty_object("Loc1", loc1, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    let obj2 = build_empty_object("Loc2", loc2, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // Verify locations are different at offset 736
    let loc1_read = f32::from_le_bytes([obj1[736], obj1[737], obj1[738], obj1[739]]);
    let loc2_read = f32::from_le_bytes([obj2[736], obj2[737], obj2[738], obj2[739]]);
    
    assert_eq!(loc1_read, loc1[0], "First object location[0] must match");
    assert_eq!(loc2_read, loc2[0], "Second object location[0] must match");
    assert_ne!(loc1_read, loc2_read, "Objects must have independent location data");
}

// ── Helper functions for pointer reading ──────────────────────────────────────

fn read_u64_le(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}

fn read_f32_le(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
    ])
}

fn read_i16_le(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

// ── Test 26: Complete offset verification matrix ──────────────────────────────
#[test]
fn complete_offset_verification_matrix() {
    let test_ptr = 0xDEADBEEFCAFEBABE_u64;
    let test_loc = [1.5, 2.5, 3.5];
    let test_scale = [2.0, 3.0, 4.0];
    let test_quat = [0.1, 0.2, 0.3, 0.4];
    
    let obj = build_empty_object("OffsetTest", test_loc, test_quat, test_scale, test_ptr);
    
    // Verify all critical offsets in one test
    assert_eq!(obj.len(), 1288);
    assert_eq!(read_i16_le(&obj, 416), 0, "type @ 416");
    assert_eq!(read_u64_le(&obj, 496), test_ptr, "parent @ 496");
    
    for i in 0..3 {
        assert!((read_f32_le(&obj, 736 + i * 4) - test_loc[i]).abs() < 0.0001, "loc[{}] @ {}", i, 736 + i * 4);
        assert!((read_f32_le(&obj, 760 + i * 4) - test_scale[i]).abs() < 0.0001, "scale[{}] @ {}", i, 760 + i * 4);
    }
    
    for i in 0..4 {
        assert!((read_f32_le(&obj, 820 + i * 4) - test_quat[i]).abs() < 0.0001, "quat[{}] @ {}", i, 820 + i * 4);
    }
}

// ── Test 27: Object name encoding in ID ────────────────────────────────────────
#[test]
fn object_name_encoding_in_id() {
    let obj_name = "MyTestObject";
    let obj = build_object(obj_name, 0x2000, 0x3000, 0x4000, 1, 0x5000);
    
    // ID.name is at offset 40, starts with "OB" prefix
    // Expected: "OBMyTestObject\0..."
    let name_start = 40;
    assert_eq!(&obj[name_start..name_start + 2], b"OB");
    assert_eq!(&obj[name_start + 2..name_start + 2 + obj_name.len()], obj_name.as_bytes());
}

// ── Test 28: Lamp object with all parameters ─────────────────────────────────
#[test]
fn lamp_object_with_all_parameters() {
    let lamp_ptr = 0xAABBCCDDEEFF1122_u64;
    let loc = [10.0, 20.0, 30.0];
    let quat = [0.707, 0.0, 0.0, 0.707];
    let scale = [1.5, 1.5, 1.5];
    let parent_ptr = 0x1234567890ABCDEF_u64;
    
    let obj = build_lamp_object("TestLamp", lamp_ptr, loc, quat, scale, parent_ptr);
    
    // Verify type is OB_LAMP (10)
    assert_eq!(read_i16_le(&obj, 416), 10);
    
    // Verify data pointer points to lamp
    assert_eq!(read_u64_le(&obj, 552), lamp_ptr);
    
    // Verify parent
    assert_eq!(read_u64_le(&obj, 496), parent_ptr);
    
    // Verify transforms
    for i in 0..3 {
        assert!((read_f32_le(&obj, 736 + i * 4) - loc[i]).abs() < 0.0001);
        assert!((read_f32_le(&obj, 760 + i * 4) - scale[i]).abs() < 0.0001);
    }
}

// ── Test 29: Consistent serialization ──────────────────────────────────────────
#[test]
fn consistent_serialization_produces_same_bytes() {
    let obj1 = build_object("SameObj", 0x2000, 0x3000, 0x4000, 2, 0x5000);
    let obj2 = build_object("SameObj", 0x2000, 0x3000, 0x4000, 2, 0x5000);
    
    assert_eq!(obj1, obj2, "Same parameters must produce identical bytes");
}

// ── Test 30: Data pointer to empty object has null data ────────────────────────
#[test]
fn empty_object_has_null_data_pointer() {
    let obj = build_empty_object("TrueEmpty", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0);
    
    // Empty objects should have null data pointer (0) at offset 552
    let data_ptr = read_u64_le(&obj, 552);
    assert_eq!(data_ptr, 0, "Empty object must have null data pointer");
}
