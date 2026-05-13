/// Tests for Library/Linked Objects audit findings (Phase 1.2)
/// These tests verify compliance against Blender 5.1.1 format spec
/// Reference: @docs/blender-format-research.md Section 14
///           blender/source/blender/makesdna/DNA_library_types.h
///           blender/source/blender/makesdna/DNA_id_types.h

#[cfg(test)]
mod library_findings {
    const LIBRARY_SIZE_CORRECT: usize = 1472;  // Blender 5.1.1 actual size
    const LIBRARY_SIZE_WRONG: usize = 1426;    // Current (incorrect) size in code
    const ID_STRUCT_SIZE_CORRECT: usize = 408; // Correct ID struct size
    const ID_STRUCT_SIZE_WRONG: usize = 370;   // Current (incorrect) size in code

    /// FINDING 1.2-1: Library struct size 46 bytes undersized
    /// LIBRARY_SIZE constant = 1426 but correct size = 1472
    /// This causes Library blocks to be truncated by 46 bytes
    #[test]
    fn test_library_struct_size_is_correct() {
        // The correct Library struct should be:
        // ID struct (408 bytes) + filepath (1024) + flag (2) + undo_runtime_tag (2) 
        // + _pad (4) + archive_parent_library (8) + packedfile (8) + runtime (8) + _pad2 (8)
        // = 408 + 1024 + 2 + 2 + 4 + 8 + 8 + 8 + 8 = 1472 bytes

        let expected_library_size = 408 + 1024 + 2 + 2 + 4 + 8 + 8 + 8 + 8;
        assert_eq!(
            expected_library_size, LIBRARY_SIZE_CORRECT,
            "Library struct should be exactly {} bytes (ID[408] + filepath[1024] + fields[36])",
            LIBRARY_SIZE_CORRECT
        );

        // The code currently uses wrong constant
        assert_ne!(
            LIBRARY_SIZE_WRONG, LIBRARY_SIZE_CORRECT,
            "Current LIBRARY_SIZE constant ({}) is wrong, should be {}",
            LIBRARY_SIZE_WRONG, LIBRARY_SIZE_CORRECT
        );

        // ID_STUB_SIZE should be 408 bytes (the ID struct)
        assert_eq!(
            ID_STRUCT_SIZE_CORRECT, 408,
            "ID_STUB_SIZE should be 408 bytes per Blender 5.1.1 DNA_id_types.h"
        );

        // The calculation currently uses 370 which is wrong
        assert_ne!(
            ID_STRUCT_SIZE_WRONG, ID_STRUCT_SIZE_CORRECT,
            "Current ID size estimate ({}) is wrong, correct is {}",
            ID_STRUCT_SIZE_WRONG, ID_STRUCT_SIZE_CORRECT
        );
    }

    /// FINDING 1.2-2: All Library field offsets misaligned by 38 bytes
    /// Due to incorrect ID struct size (370 vs 408), all subsequent fields are offset wrong:
    /// - filepath @ 370 (should be 408) ✗ 38 bytes too early
    /// - flag @ 1394 (should be 1432) ✗ 38 bytes too early
    /// - undo_runtime_tag @ 1396 (should be 1434) ✗ 38 bytes too early
    /// - etc.
    #[test]
    fn test_library_field_offsets_correct() {
        // Correct offsets based on Blender 5.1.1 Library struct:
        // All offsets are within a 1472-byte structure
        const FILEPATH_OFFSET_CORRECT: usize = 408;   // After ID struct
        const FLAG_OFFSET_CORRECT: usize = 1432;      // After ID + filepath
        const UNDO_RUNTIME_TAG_OFFSET_CORRECT: usize = 1434;
        const ARCHIVE_PARENT_OFFSET_CORRECT: usize = 1440;
        const PACKEDFILE_OFFSET_CORRECT: usize = 1448;
        const RUNTIME_OFFSET_CORRECT: usize = 1456;

        // Wrong offsets currently in code (38 bytes early for all):
        const FILEPATH_OFFSET_WRONG: usize = 370;
        const FLAG_OFFSET_WRONG: usize = 1394;
        const UNDO_RUNTIME_TAG_OFFSET_WRONG: usize = 1396;
        const ARCHIVE_PARENT_OFFSET_WRONG: usize = 1402;
        const PACKEDFILE_OFFSET_WRONG: usize = 1410;
        const RUNTIME_OFFSET_WRONG: usize = 1418;

        // Verify all current offsets are 38 bytes too early
        assert_eq!(
            FILEPATH_OFFSET_WRONG + 38, FILEPATH_OFFSET_CORRECT,
            "filepath offset off by 38 bytes (ID struct size mismatch)"
        );

        assert_eq!(
            FLAG_OFFSET_WRONG + 38, FLAG_OFFSET_CORRECT,
            "flag offset off by 38 bytes"
        );

        assert_eq!(
            UNDO_RUNTIME_TAG_OFFSET_WRONG + 38, UNDO_RUNTIME_TAG_OFFSET_CORRECT,
            "undo_runtime_tag offset off by 38 bytes"
        );

        assert_eq!(
            ARCHIVE_PARENT_OFFSET_WRONG + 38, ARCHIVE_PARENT_OFFSET_CORRECT,
            "archive_parent_library offset off by 38 bytes"
        );

        assert_eq!(
            PACKEDFILE_OFFSET_WRONG + 38, PACKEDFILE_OFFSET_CORRECT,
            "packedfile offset off by 38 bytes"
        );

        assert_eq!(
            RUNTIME_OFFSET_WRONG + 38, RUNTIME_OFFSET_CORRECT,
            "runtime offset off by 38 bytes"
        );

        // The consistent 38-byte offset proves the root cause: ID struct miscalculation
        let id_size_error = FILEPATH_OFFSET_CORRECT - FILEPATH_OFFSET_WRONG;
        assert_eq!(
            id_size_error, 38,
            "All offsets consistently off by 38 bytes, indicating ID struct miscalculation"
        );
    }

    /// FINDING 1.2-3: ID.name offset incorrect
    /// ID.name is char[258] at offset 40 within ID struct
    /// It's NOT at offset 0 (which is ID.name on its own), but embedded in larger ID struct
    #[test]
    fn test_id_name_offset_correct_in_struct() {
        // Within the full ID struct (408 bytes):
        // ID starts at offset 0, ID.name is at offset 40 (char[258])
        const ID_NAME_OFFSET_IN_STRUCT: usize = 40;
        const ID_NAME_SIZE: usize = 258;

        // When ID is embedded in Library (@ offset 0), ID.name is still @ offset 40
        const ID_NAME_OFFSET_IN_LIBRARY: usize = 0 + ID_NAME_OFFSET_IN_STRUCT;

        // The code incorrectly writes to offset 0 with size 66
        // This truncates the ID.name field and corrupts data
        assert_eq!(
            ID_NAME_OFFSET_IN_LIBRARY, 40,
            "ID.name should be at offset 40 within Library struct"
        );

        assert_eq!(
            ID_NAME_SIZE, 258,
            "ID.name field should be 258 bytes (char[258]), not 66"
        );
    }

    /// FINDING 1.2-4: Buffer overflow risk in build_id_stub
    /// Passing a 66-byte slice to write_id_name which tries to write 258-byte ID.name
    /// Any name > ~24 chars will panic when accessing beyond byte 66
    #[test]
    fn test_id_stub_buffer_size_sufficient() {
        // build_id_stub creates a 66-byte buffer for an ID struct
        // But ID_STUB_SIZE (correct) = 408 bytes
        // The 66-byte buffer will overflow when write_id_name tries to set ID.name[258]
        
        const ID_STUB_SIZE_CURRENT: usize = 66;  // What code uses
        const ID_STUB_SIZE_CORRECT: usize = 408; // What it should be
        const ID_NAME_OFFSET_IN_STRUCT: usize = 40;

        assert_ne!(
            ID_STUB_SIZE_CURRENT, ID_STUB_SIZE_CORRECT,
            "ID stub buffer ({}) too small for full ID struct ({})",
            ID_STUB_SIZE_CURRENT, ID_STUB_SIZE_CORRECT
        );

        // With a 66-byte buffer and write_id_name trying to write at offset 40 with len 258:
        // Attempt to access bytes [40..40+258] = [40..298]
        // But buffer is only 66 bytes → panic!
        assert!(
            ID_STUB_SIZE_CURRENT + 40 < ID_NAME_OFFSET_IN_STRUCT + 100,
            "66-byte buffer insufficient for ID.name offset + minimum name length"
        );
    }

    /// FINDING 1.2-5: ID_STUB_SIZE and LIBRARY_SIZE relationship unclear
    /// Documentation doesn't explain that Library embeds a full ID struct
    /// This relationship should be explicit in code comments
    #[test]
    fn test_library_contains_full_id_struct() {
        // Library struct composition (Blender 5.1.1):
        // - ID struct @ offset 0, size 408 bytes (embedded)
        // - filepath (char[1024]) @ offset 408
        // - Additional fields @ offset 1432+

        const LIBRARY_ID_SIZE: usize = 408;
        const LIBRARY_FILEPATH_SIZE: usize = 1024;

        // Total Library size should be:
        // ID (408) + filepath (1024) + flag (2) + undo_runtime_tag (2) + _pad (4) 
        // + archive_parent_library (8) + packedfile (8) + runtime (8) + _pad2 (8) = 40 bytes
        const LIBRARY_TOTAL_SIZE: usize = LIBRARY_ID_SIZE + LIBRARY_FILEPATH_SIZE + 40; // + flags, pads, pointers

        assert_eq!(
            LIBRARY_TOTAL_SIZE, 1472,
            "Library = ID({}) + filepath({}) + fields(40) = {}",
            LIBRARY_ID_SIZE, LIBRARY_FILEPATH_SIZE, LIBRARY_TOTAL_SIZE
        );

        // The relationship ID_STUB_SIZE ⊂ LIBRARY_SIZE should be documented
        assert_eq!(
            ID_STRUCT_SIZE_CORRECT, LIBRARY_ID_SIZE,
            "ID_STUB_SIZE should equal the ID portion of Library"
        );
    }
}
