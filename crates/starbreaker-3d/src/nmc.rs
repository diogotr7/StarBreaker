use starbreaker_chunks::ChunkFile;
use starbreaker_common::reader::SpanReader;

pub const NMC_FULL: u32 = 0x70697FDA;

#[derive(Debug, Clone)]
pub struct NmcNode {
    pub name: String,
    pub parent_index: Option<u16>,    // None for root (0xFFFF)
    pub world_to_bone: [[f32; 4]; 3], // 3x4 row-major
    pub bone_to_world: [[f32; 4]; 3], // 3x4 row-major
    pub scale: [f32; 3],              // Per-node scale at entry +128
    pub geometry_type: u16,           // Node type at entry +150 (0=GEOM, 3=HELP2, etc.)
    /// Structured properties from the NMC metadata string table.
    /// Key-value pairs like "class" → "AnimatedJoint", "detach" → "True".
    pub properties: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct NodeMeshCombo {
    pub nodes: Vec<NmcNode>,
    pub material_indices: Vec<u16>,
}

/// Parse NMC_Full (0x70697FDA) from a .cga/.cgf metadata file.
///
/// Returns `(nodes, material_indices)` if the chunk is found and parseable.
///
/// Per-node entry layout (208 bytes, verified via 1.0f diagonal scan + parent index validation):
///   +0..+31:   pre-matrix metadata (32 bytes)
///   +32..+79:  WorldToBone (3x4 row-major, 48 bytes)
///   +80..+127: BoneToWorld (3x4 row-major, 48 bytes)
///   +128..+139: Scale (vec3, 12 bytes)
///   +140..+143: Id (u32)
///   +144..+147: Unknown (u32)
///   +148..+149: ParentIndex (u16, 0xFFFF = root)
///   +150..+151: GeometryType (u16)
///   +152..+175: BBox (min + max vec3, 24 bytes)
///   +176..+207: remaining metadata (32 bytes)
///
/// Header: `h[1]` = total node count (geometry + non-geometry).
/// Footer (immediately after entries): unk_indices, mat_indices, string_table.
pub fn parse_nmc_full(file_data: &[u8]) -> Option<(Vec<NmcNode>, Vec<u16>)> {
    let chunk_file = ChunkFile::from_bytes(file_data).ok()?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return None,
    };

    let nmc_entry = ivo.chunks().iter().find(|c| c.chunk_type == NMC_FULL)?;
    let nmc_data = ivo.chunk_data(nmc_entry);
    let mut r = SpanReader::new(nmc_data);

    // Header: 8 x i32
    let _h0 = r.read_i32().ok()?;
    let num_nodes = r.read_i32().ok()? as usize; // h[1] = total nodes
    let _num_geom_nodes = r.read_i32().ok()?; // h[2] = geometry nodes
    let num_unk_indices = r.read_i32().ok()? as usize;
    let num_mat_indices = r.read_i32().ok()? as usize;
    let string_table_size = r.read_i32().ok()? as usize;
    let _h6 = r.read_i32().ok()?;
    let _h7 = r.read_i32().ok()?;

    // Per-node entries: 208 bytes each
    let mut parent_indices = Vec::with_capacity(num_nodes);
    let mut matrices = Vec::with_capacity(num_nodes);

    for _ in 0..num_nodes {
        // Skip pre-matrix metadata (32 bytes)
        r.advance(32).ok()?;

        // WorldToBone: 3x4 matrix at entry +32 (48 bytes)
        let mut w2b = [[0.0f32; 4]; 3];
        for row in &mut w2b {
            for val in row.iter_mut() {
                *val = r.read_f32().ok()?;
            }
        }
        // BoneToWorld: 3x4 matrix at entry +80 (48 bytes)
        let mut b2w = [[0.0f32; 4]; 3];
        for row in &mut b2w {
            for val in row.iter_mut() {
                *val = r.read_f32().ok()?;
            }
        }

        // Scale: 3x f32 at entry +128 (12 bytes)
        let scale = [
            r.read_f32().ok()?,
            r.read_f32().ok()?,
            r.read_f32().ok()?,
        ];
        // Skip Id:4 + Unknown:4 (8 bytes)
        r.advance(8).ok()?;

        // ParentIndex: u16 at entry +148
        let parent_index = r.read_u16().ok()?;

        // GeometryType: u16 at entry +150
        let geometry_type = r.read_u16().ok()?;

        // Skip remaining 56 bytes to reach 208 total
        // (BBox:24 + remaining:32 = 56)
        r.advance(56).ok()?;


        parent_indices.push(parent_index);
        matrices.push((w2b, b2w, scale, geometry_type));
    }

    // Footer: 32-byte header (float data, e.g. -1.0 values) sits between entries and indices.
    r.advance(32).ok()?;

    // Unknown indices
    let _unk_indices: Vec<u16> = (0..num_unk_indices)
        .map(|_| r.read_u16().ok())
        .collect::<Option<Vec<_>>>()?;

    // Material indices
    let mut mat_indices = Vec::with_capacity(num_mat_indices);
    for _ in 0..num_mat_indices {
        mat_indices.push(r.read_u16().ok()?);
    }

    // String table: num_nodes null-separated names read sequentially.
    // Each string may contain structured data ("node=Part\nclass=ItemPort\n...") — take first line as name.
    let str_bytes = r.read_bytes(string_table_size).ok()?;
    let mut names = Vec::with_capacity(num_nodes);
    let mut pos = 0;
    for _ in 0..num_nodes {
        if pos >= str_bytes.len() {
            names.push(String::new());
            continue;
        }
        let end = str_bytes[pos..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| pos + p)
            .unwrap_or(str_bytes.len());
        let full = std::str::from_utf8(&str_bytes[pos..end]).unwrap_or("");
        names.push(full.lines().next().unwrap_or("").to_string());
        pos = end + 1;
    }



    // Second string table: per-node metadata with key=value properties.
    // Same structure as the first — num_nodes null-separated entries.
    let meta_remaining = r.remaining();
    let meta_bytes = r.read_bytes(meta_remaining).ok().unwrap_or(&[]);
    let mut node_properties: Vec<std::collections::HashMap<String, String>> = Vec::with_capacity(num_nodes);
    let mut meta_pos = 0;
    for _ in 0..num_nodes {
        let mut props = std::collections::HashMap::new();
        if meta_pos < meta_bytes.len() {
            let end = meta_bytes[meta_pos..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| meta_pos + p)
                .unwrap_or(meta_bytes.len());
            if let Ok(s) = std::str::from_utf8(&meta_bytes[meta_pos..end]) {
                for line in s.lines() {
                    let line = line.trim();
                    if let Some((k, v)) = line.split_once('=') {
                        props.insert(k.to_string(), v.to_string());
                    }
                }
            }
            meta_pos = end + 1;
        }
        node_properties.push(props);
    }

    // Build nodes: 1:1 positional mapping — name[i] corresponds to node[i].
    let nodes = (0..num_nodes)
        .map(|i| {
            let parent_index = parent_indices[i];
            let (w2b, b2w, scale, geometry_type) = matrices[i];
            NmcNode {
                name: names.get(i).cloned().unwrap_or_default(),
                parent_index: if parent_index == 0xFFFF {
                    None
                } else {
                    Some(parent_index)
                },
                world_to_bone: w2b,
                bone_to_world: b2w,
                scale,
                geometry_type,
                properties: node_properties.get(i).cloned().unwrap_or_default(),
            }
        })
        .collect();

    Some((nodes, mat_indices))
}
