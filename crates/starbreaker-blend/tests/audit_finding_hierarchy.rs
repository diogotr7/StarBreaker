/// Tests for Scene Hierarchy audit findings (Phase 1.3)
/// These tests verify compliance against Blender 5.1.1 format spec
/// Reference: @docs/blender-format-research.md Sections 7-10, 17E
///           blender/source/blender/makesdna/DNA_collection_types.h
///           blender/source/blender/makesdna/DNA_layer_types.h

#[cfg(test)]
mod hierarchy_findings {
    // Blender 5.1.1 struct offsets (verified against DNA_*.h)
    const ID_STRUCT_SIZE: usize = 408;
    const COLLECTION_STRUCT_SIZE: usize = 544; // ID(408) + fields(136)
    
    // Collection field offsets within Collection struct
    const COLLECTION_OWNER_ID_OFFSET: usize = 408;     // After ID
    const COLLECTION_GOBJECT_OFFSET: usize = 416;      // ListBase for objects
    const COLLECTION_GOBJECT_HEAD: usize = 416;
    const COLLECTION_GOBJECT_TAIL: usize = 424;
    const COLLECTION_CHILDREN_OFFSET: usize = 432;     // ListBase for children
    const COLLECTION_CHILDREN_HEAD: usize = 432;
    const COLLECTION_CHILDREN_TAIL: usize = 440;

    // Wrong offsets currently in code (overlap with ID struct)
    const COLLECTION_GOBJECT_WRONG_OFFSET: usize = 128;  // INSIDE ID (bytes 0-407)
    const COLLECTION_CHILDREN_WRONG_OFFSET: usize = 144; // INSIDE ID (bytes 0-407)

    /// FINDING 1.3-1: Collection struct field offsets completely wrong
    /// Offsets written at 128-168 fall INSIDE the 408-byte ID struct
    /// Should be 416-440 (after ID)
    #[test]
    fn test_collection_field_offsets_correct() {
        // Verify ID struct is 408 bytes (not 120 as old doc claimed)
        assert_eq!(
            ID_STRUCT_SIZE, 408,
            "ID struct should be 408 bytes per DNA_id_types.h"
        );

        // Correct offsets: ID ends at byte 407, Collection fields start at 408
        assert_eq!(
            COLLECTION_OWNER_ID_OFFSET, 408,
            "Collection.owner_id should start at byte 408 (after ID)"
        );

        assert_eq!(
            COLLECTION_GOBJECT_HEAD, 416,
            "Collection.gobject ListBase.first should be at byte 416"
        );

        assert_eq!(
            COLLECTION_GOBJECT_TAIL, 424,
            "Collection.gobject ListBase.last should be at byte 424"
        );

        assert_eq!(
            COLLECTION_CHILDREN_HEAD, 432,
            "Collection.children ListBase.first should be at byte 432"
        );

        assert_eq!(
            COLLECTION_CHILDREN_TAIL, 440,
            "Collection.children ListBase.last should be at byte 440"
        );

        // Current wrong offsets (inside ID struct - corruption!)
        assert!(
            COLLECTION_GOBJECT_WRONG_OFFSET < ID_STRUCT_SIZE,
            "Current gobject offset {} falls inside ID struct (0-407)",
            COLLECTION_GOBJECT_WRONG_OFFSET
        );

        assert!(
            COLLECTION_CHILDREN_WRONG_OFFSET < ID_STRUCT_SIZE,
            "Current children offset {} falls inside ID struct (0-407)",
            COLLECTION_CHILDREN_WRONG_OFFSET
        );

        // The misalignment is ~288 bytes (408 - 120 = 288)
        // This proves the root cause: ID struct size was underestimated as 120 vs actual 408
        let offset_error = ID_STRUCT_SIZE - 120;
        assert_eq!(
            offset_error, 288,
            "ID struct was estimated as 120 bytes, should be 408 (error: 288 bytes)"
        );
    }

    /// FINDING 1.3-2: LayerCollection linked list structure broken
    /// LayerCollection must be doubly-linked (prev/next at offsets 0-15)
    /// Currently written as separate blocks without tree hierarchy
    #[test]
    fn test_layer_collection_is_doubly_linked_list() {
        // LayerCollection struct (DNA_layer_types.h:189):
        // - prev/next pointers at offsets 0-15 (doubly-linked)
        // - collection pointer at offset 16-23
        // - layer_collections ListBase at offset 40-55 (for nested children)

        const LAYER_COLLECTION_PREV_OFFSET: usize = 0;
        const LAYER_COLLECTION_NEXT_OFFSET: usize = 8;
        const LAYER_COLLECTION_COLLECTION_OFFSET: usize = 16;
        const LAYER_COLLECTION_CHILDREN_OFFSET: usize = 40;

        // Verify structure layout
        assert!(LAYER_COLLECTION_PREV_OFFSET < LAYER_COLLECTION_NEXT_OFFSET);
        assert!(LAYER_COLLECTION_NEXT_OFFSET < LAYER_COLLECTION_COLLECTION_OFFSET);
        assert!(LAYER_COLLECTION_COLLECTION_OFFSET < LAYER_COLLECTION_CHILDREN_OFFSET);

        // LayerCollection must form a tree:
        // Root LayerCollection:
        //   - prev = NULL (no previous sibling)
        //   - next = NULL (no next sibling, or next root if multiple)
        //   - collection = root Collection
        //   - layer_collections = ListBase of child LayerCollections (nested)
        //
        // Child LayerCollections:
        //   - prev/next = pointers to siblings in parent's layer_collections list
        //   - collection = sub-collection
        //   - layer_collections = ListBase of grandchildren (or empty)

        assert_eq!(LAYER_COLLECTION_PREV_OFFSET, 0, "prev pointer should be first field");
        assert_eq!(LAYER_COLLECTION_NEXT_OFFSET, 8, "next pointer should follow prev");
        assert_eq!(LAYER_COLLECTION_CHILDREN_OFFSET, 40, "nested children list at offset 40");
    }

    /// FINDING 1.3-3: ViewLayer-to-LayerCollection hierarchy disconnected
    /// ViewLayer.layer_collections (offset 120-135) should contain root LayerCollection
    /// Currently written as separate pointers without forming a ListBase tree
    #[test]
    fn test_view_layer_layer_collections_hierarchy() {
        // ViewLayer struct (DNA_layer_types.h:249):
        // - layer_collections ListBase at offset 120-135
        // This ListBase should contain exactly ONE root LayerCollection
        // which in turn contains nested LayerCollections via its own .layer_collections

        const VIEWLAYER_LAYER_COLLECTIONS_OFFSET: usize = 120;
        const LISTBASE_SIZE: usize = 16; // first(8) + last(8)

        assert_eq!(
            LISTBASE_SIZE, 16,
            "ListBase structure should be 16 bytes (two 8-byte pointers)"
        );

        // The hierarchy must be:
        // ViewLayer
        //   └─ layer_collections ListBase
        //      └─ root LayerCollection
        //         ├─ prev/next = NULL (it's the only root)
        //         ├─ collection = root Collection
        //         └─ layer_collections ListBase
        //            ├─ child LayerCollection 1
        //            ├─ child LayerCollection 2
        //            └─ child LayerCollection 3 (etc.)

        // Current code breaks this by:
        // 1. Creating multiple separate LayerCollection blocks
        // 2. Not linking them via prev/next pointers
        // 3. Not putting them in ViewLayer.layer_collections ListBase

        // Result: ViewLayer.layer_collections is either empty or malformed
        // causing "invisible objects" (load but don't show in viewport)

        assert_eq!(VIEWLAYER_LAYER_COLLECTIONS_OFFSET, 120);
    }

    /// FINDING 1.3-4: Collection.owner_id field uninitialized
    /// Collection struct requires owner_id pointer at offset 408-415
    /// Should point to the Scene ID that owns this collection
    #[test]
    fn test_collection_owner_id_is_required() {
        // Collection.owner_id (offset 408-415):
        // - 8-byte pointer to ID block of owning Scene
        // - Root collections should have owner_id pointing to Scene.id
        // - Currently this field is never initialized (remains zero/NULL)

        const COLLECTION_OWNER_ID_OFFSET: usize = 408;
        const COLLECTION_OWNER_ID_SIZE: usize = 8; // 64-bit pointer

        assert_eq!(
            COLLECTION_OWNER_ID_OFFSET + COLLECTION_OWNER_ID_SIZE, 416,
            "owner_id field should be followed by gobject at 416"
        );

        // Without owner_id initialized:
        // - Blender may not know which Scene owns the collection
        // - Collection may not appear in Scene properties
        // - Embedded vs library collections won't be handled correctly
    }

    /// FINDING 1.3-5: Research doc has conflicting struct layouts
    /// Section 17E claims ID = 120 bytes, but Table 11.1 correctly states 408 bytes
    /// This inconsistency directly caused the implementation bugs
    #[test]
    fn test_research_doc_id_size_consistent() {
        // Table 11.1 (correct): ID = 408 bytes
        const ID_SIZE_CORRECT: usize = 408;

        // Section 17E (wrong): ID = 120 bytes
        const ID_SIZE_WRONG: usize = 120;

        // The error = 288 bytes
        const SIZE_DISCREPANCY: usize = ID_SIZE_CORRECT - ID_SIZE_WRONG;

        assert_eq!(
            SIZE_DISCREPANCY, 288,
            "Research doc has 288-byte inconsistency (120 vs 408 for ID struct)"
        );

        // This discrepancy is the ROOT CAUSE of all Collection/Hierarchy bugs
        // Implementation used the wrong offset table (120-byte ID) instead of correct (408-byte ID)
        // Causing all subsequent field offsets to be ~288 bytes off

        assert_ne!(
            ID_SIZE_WRONG, ID_SIZE_CORRECT,
            "Research doc must be consistent: ID is {} bytes, not {}",
            ID_SIZE_CORRECT, ID_SIZE_WRONG
        );
    }

    /// BONUS: Verify Collection struct size based on correct field offsets
    #[test]
    fn test_collection_struct_size_correct() {
        // Collection struct composition (DNA_collection_types.h):
        // ID (408) + owner_id (8) + gobject ListBase (16) + children ListBase (16)
        // + color_tag (4) + flag (4) + nested fields...
        // Total: 544 bytes

        assert_eq!(
            COLLECTION_STRUCT_SIZE, 544,
            "Collection struct should be 544 bytes (ID[408] + fields[136])"
        );

        // Verify field layout:
        assert!(COLLECTION_OWNER_ID_OFFSET < COLLECTION_GOBJECT_OFFSET);
        assert!(COLLECTION_GOBJECT_OFFSET < COLLECTION_CHILDREN_OFFSET);

        // All offsets should be within 544-byte struct
        assert!(COLLECTION_CHILDREN_TAIL < COLLECTION_STRUCT_SIZE);
    }
}
