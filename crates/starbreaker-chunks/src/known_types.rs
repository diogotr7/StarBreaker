/// Hash-based chunk type constants used in IVO (Star Citizen) files.
pub mod ivo {
    pub const BSHAPES: u32 = 0xF5C6EB5B;
    pub const BSHAPES_GPU: u32 = 0x57A38888;
    pub const COMPILED_BONES: u32 = 0xC201973C;
    pub const COMPILED_BONES_IVO320: u32 = 0xC2011111;
    pub const DBA: u32 = 0xF7351608;
    pub const DB_DATA: u32 = 0x194FBC50;
    pub const EXPORT_FLAGS: u32 = 0xBE5E493E;
    pub const IVO_SKIN2: u32 = 0xB8757777;
    pub const MESH_IVO320: u32 = 0x92914444;
    pub const MTL_NAME_IVO320: u32 = 0x83353333;
    pub const NODE_MESH_COMBOS: u32 = 0x70697FDA;
    pub const PHYSICAL_HIERARCHY: u32 = 0x90C62222;
    pub const PROTOS_INFO: u32 = 0xF7086666;
    pub const RIG_INFO: u32 = 0x7E035555;
    pub const RIG_LOGIC: u32 = 0x0A2485B6;
    pub const SKELETON: u32 = 0x1BBC4103;
    pub const VIS_AREAS: u32 = 0xB32459D2;
    pub const LOD_DISTANCE: u32 = 0x9351756F;
    pub const STATOBJ_PHYSICS: u32 = 0x58DE1772;
    pub const POSITION_BONEMAP: u32 = 0x2B7ECF9F;

    /// Returns a human-readable name for a known IVO chunk type, or `None`.
    pub fn name(chunk_type: u32) -> Option<&'static str> {
        match chunk_type {
            BSHAPES => Some("BShapes"),
            BSHAPES_GPU => Some("BShapesGpu"),
            COMPILED_BONES => Some("CompiledBones"),
            COMPILED_BONES_IVO320 => Some("CompiledBonesIvo320"),
            DBA => Some("DBA"),
            DB_DATA => Some("DbData"),
            EXPORT_FLAGS => Some("ExportFlags"),
            IVO_SKIN2 => Some("IvoSkin2"),
            MESH_IVO320 => Some("MeshIvo320"),
            MTL_NAME_IVO320 => Some("MtlNameIvo320"),
            NODE_MESH_COMBOS => Some("NodeMeshCombos"),
            PHYSICAL_HIERARCHY => Some("PhysicalHierarchy"),
            PROTOS_INFO => Some("ProtosInfo"),
            RIG_INFO => Some("RigInfo"),
            RIG_LOGIC => Some("RigLogic"),
            SKELETON => Some("Skeleton"),
            VIS_AREAS => Some("VisAreas"),
            LOD_DISTANCE => Some("LODDistance"),
            STATOBJ_PHYSICS => Some("StatObjPhysics"),
            POSITION_BONEMAP => Some("PositionBonemap"),
            _ => None,
        }
    }
}

/// Numeric chunk type constants used in CrCh (Legacy CryEngine) files.
pub mod crch {
    pub const ANY: u16 = 0x0000;
    pub const MESH: u16 = 0x1000;
    pub const HELPER: u16 = 0x1001;
    pub const BONE_ANIM: u16 = 0x1003;
    pub const BONE_NAME_LIST: u16 = 0x1005;
    pub const SCENE_PROPS: u16 = 0x1008;
    pub const NODE: u16 = 0x100B;
    pub const CONTROLLER: u16 = 0x100D;
    pub const TIMING: u16 = 0x100E;
    pub const BONE_MESH: u16 = 0x100F;
    pub const MESH_MORPH_TARGET: u16 = 0x1011;
    pub const SOURCE_INFO: u16 = 0x1013;
    pub const MTL_NAME: u16 = 0x1014;
    pub const EXPORT_FLAGS: u16 = 0x1015;
    pub const DATA_STREAM: u16 = 0x1016;
    pub const MESH_SUBSETS: u16 = 0x1017;
    pub const MESH_PHYSICS_DATA: u16 = 0x1018;

    // SOC-specific chunk types (used in .soc files inside .socpak archives).
    // These occupy the low range (0x0002..0x0010) and do not collide with
    // standard CrCh geometry types (0x1000+).
    pub const UNKNOWN_SC2: u16 = 0x0002;
    pub const CRYXMLB: u16 = 0x0004;
    pub const UNKNOWN_SC5: u16 = 0x0008;
    pub const AREA_SHAPE: u16 = 0x000E;
    pub const INCLUDED_OBJECTS: u16 = 0x0010;

    /// Returns a human-readable name for a known CrCh chunk type, or `None`.
    pub fn name(chunk_type: u16) -> Option<&'static str> {
        match chunk_type {
            ANY => Some("Any"),
            MESH => Some("Mesh"),
            HELPER => Some("Helper"),
            BONE_ANIM => Some("BoneAnim"),
            BONE_NAME_LIST => Some("BoneNameList"),
            SCENE_PROPS => Some("SceneProps"),
            NODE => Some("Node"),
            CONTROLLER => Some("Controller"),
            TIMING => Some("Timing"),
            BONE_MESH => Some("BoneMesh"),
            MESH_MORPH_TARGET => Some("MeshMorphTarget"),
            SOURCE_INFO => Some("SourceInfo"),
            MTL_NAME => Some("MtlName"),
            EXPORT_FLAGS => Some("ExportFlags"),
            DATA_STREAM => Some("DataStream"),
            MESH_SUBSETS => Some("MeshSubsets"),
            MESH_PHYSICS_DATA => Some("MeshPhysicsData"),
            UNKNOWN_SC2 => Some("UnknownSC2"),
            CRYXMLB => Some("CryXMLB"),
            UNKNOWN_SC5 => Some("UnknownSC5"),
            AREA_SHAPE => Some("AreaShape"),
            INCLUDED_OBJECTS => Some("IncludedObjects"),
            _ => None,
        }
    }
}
