//! Stage 3: CDF (CharacterDefinition) parser.
//!
//! `.cdf` files are CryXmlB binary documents listing the skeleton model and a
//! flat list of attachments (head, eyes, teeth, clothing, etc.). [`parse_cdf`]
//! extracts the skeleton path, the `Head` attachment, and ALL attachments
//! (`CharacterDefinition::attachments`) — the head pipeline selects the head
//! sub-meshes (eyes/teeth/eye-details) from those by directory co-location
//! with the head `.skin` (see `character::mod::load_head_submeshes`).
//!
//! Entry point: [`parse_cdf`].
//! Key types: [`CharacterDefinition`], [`Attachment`], [`DeformerKind`].

use starbreaker_cryxml::CryXmlNode;

use super::CharacterError;

/// Parsed content of a `.cdf` file.
#[derive(Debug, Clone)]
pub struct CharacterDefinition {
    /// Engine-relative path to the character skeleton (`.chr`).
    pub skeleton_path: String,
    /// The `Head` attachment entry.
    pub head_attachment: Attachment,
    /// Every `<Attachment>` with a binding `.skin` (Head, Eyes, Teeth,
    /// clothing, …), in document order. Callers select the head sub-meshes
    /// structurally — e.g. those whose `binding_skin` is co-located with the
    /// head `.skin` directory — rather than by name.
    pub attachments: Vec<Attachment>,
}

/// A single `<Attachment>` entry from the CDF.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Engine-relative path to the bound `.skin` file.
    pub binding_skin: String,
    /// Which deformer stack drives this attachment.
    pub deformer: DeformerKind,
    /// Whether DNA-driven deformation is active (`DynamicDNA="1"`).
    pub dynamic_dna: bool,
}

/// Known deformer kinds found in CDF files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeformerKind {
    Protos,
    ProtosMisc,
    WdElastic,
    WdBshapeExclusion,
    Other(String),
}

impl DeformerKind {
    fn parse(s: &str) -> Self {
        match s {
            "Protos" => Self::Protos,
            "ProtosMisc" => Self::ProtosMisc,
            "WD_Elastic" => Self::WdElastic,
            "WD_BShapeExclusion" => Self::WdBshapeExclusion,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Parse a CryXmlB `.cdf` byte buffer into a [`CharacterDefinition`].
///
/// Errors:
/// - Malformed CryXmlB → `CdfParse`
/// - Missing `<Model File=...>` element → `CdfParse`
/// - Missing `<Attachment AName="Head">` entry → `CdfParse`
pub fn parse_cdf(bytes: &[u8]) -> Result<CharacterDefinition, CharacterError> {
    let xml = starbreaker_cryxml::from_bytes(bytes).map_err(|e| CharacterError::CdfParse {
        context: "CryXmlB decode".into(),
        message: format!("{e:?}"),
    })?;

    let mut skeleton_path: Option<String> = None;
    let mut head_attachment: Option<Attachment> = None;
    let mut attachments: Vec<Attachment> = Vec::new();

    walk(
        &xml,
        xml.root(),
        &mut skeleton_path,
        &mut head_attachment,
        &mut attachments,
    );

    let skeleton_path = skeleton_path.ok_or_else(|| CharacterError::CdfParse {
        context: "<CharacterDefinition>".into(),
        message: "no <Model File=...> element found".into(),
    })?;
    let head_attachment = head_attachment.ok_or_else(|| CharacterError::CdfParse {
        context: "<AttachmentList>".into(),
        message: "no <Attachment AName=\"Head\"> found".into(),
    })?;

    Ok(CharacterDefinition {
        skeleton_path,
        head_attachment,
        attachments,
    })
}

fn walk(
    xml: &starbreaker_cryxml::CryXml<'_>,
    node: &CryXmlNode,
    skeleton_path: &mut Option<String>,
    head_attachment: &mut Option<Attachment>,
    attachments: &mut Vec<Attachment>,
) {
    let tag = xml.node_tag(node);

    match tag {
        "Model" => {
            for (k, v) in xml.node_attributes(node) {
                if k == "File" {
                    *skeleton_path = Some(v.to_owned());
                }
            }
        }
        "Attachment" => {
            let mut aname: Option<&str> = None;
            let mut binding: Option<&str> = None;
            let mut deformer = DeformerKind::Other(String::new());
            let mut dynamic_dna = false;

            for (k, v) in xml.node_attributes(node) {
                match k {
                    "AName" => aname = Some(v),
                    "Binding" => binding = Some(v),
                    "Deformer" => deformer = DeformerKind::parse(v),
                    "DynamicDNA" => dynamic_dna = v == "1",
                    _ => {}
                }
            }

            if let Some(b) = binding {
                let att = Attachment {
                    binding_skin: b.to_owned(),
                    deformer,
                    dynamic_dna,
                };
                if aname == Some("Head") {
                    *head_attachment = Some(att.clone());
                }
                attachments.push(att);
            }
        }
        _ => {}
    }

    for child in xml.node_children(node) {
        walk(xml, child, skeleton_path, head_attachment, attachments);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CryXmlB fixture builder ──────────────────────────────────────────────
    //
    // CryXmlHeader (36 bytes = 9 × u32, little-endian):
    //   xml_size, node_table_position, node_count,
    //   attribute_table_position, attribute_count,
    //   child_table_position, child_count,
    //   string_data_position, string_data_size
    //
    // CryXmlNode (28 bytes, packed):
    //   tag_string_offset: u32
    //   item_type: u32
    //   attribute_count: u16
    //   child_count: u16
    //   parent_index: i32
    //   first_attribute_index: i32
    //   first_child_index: i32
    //   reserved: i32
    //
    // CryXmlAttribute (8 bytes):
    //   key_string_offset: u32
    //   value_string_offset: u32
    //
    // Data order after magic: header → nodes → child_indices → attributes → strings
    // (from_bytes reads sequentially in this order)

    const MAGIC: &[u8] = b"CryXmlB\0";
    const HEADER_SIZE: u32 = 9 * 4; // 36 bytes
    const MAGIC_SIZE: u32 = 8;
    const NODE_SIZE: u32 = 28;
    const ATTR_SIZE: u32 = 8;
    const CHILD_SIZE: u32 = 4;

    fn build_string_pool(strings: &[&str]) -> (Vec<u8>, std::collections::HashMap<String, u32>) {
        let mut pool: Vec<u8> = Vec::new();
        let mut offsets: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        for &s in strings {
            if !offsets.contains_key(s) {
                offsets.insert(s.to_owned(), pool.len() as u32);
                pool.extend_from_slice(s.as_bytes());
                pool.push(0); // NUL terminator
            }
        }
        (pool, offsets)
    }

    fn u32le(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn i32le(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn u16le(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }

    /// Encode a CDF document:
    /// ```xml
    /// <CharacterDefinition>
    ///   <Model File="{model_file}" />
    ///   <AttachmentList>
    ///     (one <Attachment> per entry in attachments)
    ///   </AttachmentList>
    /// </CharacterDefinition>
    /// ```
    /// Each attachment entry is `(AName, Binding, Deformer, DynamicDNA)`.
    fn encode_cdf(
        model_file: &str,
        attachments: &[(&str, &str, &str, &str)],
    ) -> Vec<u8> {
        let n_attach = attachments.len();

        // Collect all strings that appear.
        let mut all_strings: Vec<&str> = vec![
            "CharacterDefinition",
            "Model",
            "File",
            model_file,
            "AttachmentList",
            "Attachment",
            "AName",
            "Binding",
            "Deformer",
            "DynamicDNA",
        ];
        for &(aname, binding, deformer, dna) in attachments {
            all_strings.extend_from_slice(&[aname, binding, deformer, dna]);
        }
        let (string_data, offsets) = build_string_pool(&all_strings);

        let o = |s: &str| -> u32 { *offsets.get(s).unwrap() };

        // Nodes (indices):
        //  0 = CharacterDefinition  (root, parent=-1)
        //  1 = Model                (parent=0)
        //  2 = AttachmentList       (parent=0)
        //  3..3+n = Attachment[i]   (parent=2)
        let n_nodes = 3 + n_attach;

        // Child-index table:
        //  [0,1]: CharacterDefinition's children → nodes 1, 2
        //  [2..2+n]: AttachmentList's children → nodes 3, 4, ..., 3+n-1
        let mut child_indices: Vec<i32> = vec![1, 2];
        for i in 0..n_attach {
            child_indices.push(3 + i as i32);
        }

        // Attribute table:
        //  [0]: Model.File=model_file
        //  [1+i*4 .. 1+i*4+4]: Attachment[i].{AName, Binding, Deformer, DynamicDNA}
        let mut attrs: Vec<(u32, u32)> = Vec::new();
        attrs.push((o("File"), o(model_file)));
        for &(aname, binding, deformer, dna) in attachments {
            attrs.push((o("AName"), o(aname)));
            attrs.push((o("Binding"), o(binding)));
            attrs.push((o("Deformer"), o(deformer)));
            attrs.push((o("DynamicDNA"), o(dna)));
        }

        // Compute section positions (relative to file start, after magic).
        let node_table_position = MAGIC_SIZE + HEADER_SIZE;
        let child_table_position =
            node_table_position + NODE_SIZE * n_nodes as u32;
        let attr_table_position =
            child_table_position + CHILD_SIZE * child_indices.len() as u32;
        let string_data_position =
            attr_table_position + ATTR_SIZE * attrs.len() as u32;
        let xml_size = string_data_position + string_data.len() as u32;

        let mut out: Vec<u8> = Vec::new();

        // Magic
        out.extend_from_slice(MAGIC);

        // Header
        out.extend_from_slice(&u32le(xml_size));
        out.extend_from_slice(&u32le(node_table_position));
        out.extend_from_slice(&u32le(n_nodes as u32));
        out.extend_from_slice(&u32le(attr_table_position));
        out.extend_from_slice(&u32le(attrs.len() as u32));
        out.extend_from_slice(&u32le(child_table_position));
        out.extend_from_slice(&u32le(child_indices.len() as u32));
        out.extend_from_slice(&u32le(string_data_position));
        out.extend_from_slice(&u32le(string_data.len() as u32));

        // Nodes
        // Node 0: CharacterDefinition — 0 attrs, 2 children (child_start=0)
        // first_attr_idx=0 is safe even with 0 attrs (empty slice [0..0])
        write_node(
            &mut out,
            o("CharacterDefinition"),
            0,
            2,
            -1,
            0,
            0,
        );
        // Node 1: Model — 1 attr (attr_start=0), 0 children (child_start=0)
        // child_count=0 so first_child_idx value doesn't matter, but must be valid index
        write_node(&mut out, o("Model"), 1, 0, 0, 0, 0);
        // Node 2: AttachmentList — 0 attrs, n_attach children (child_start=2)
        write_node(
            &mut out,
            o("AttachmentList"),
            0,
            n_attach as u16,
            0,
            0,
            2,
        );
        // Nodes 3..3+n: Attachment[i] — 4 attrs each, 0 children
        for i in 0..n_attach {
            write_node(
                &mut out,
                o("Attachment"),
                4,
                0,
                2,
                1 + i as i32 * 4,
                0, // 0 children so first_child_idx doesn't matter; use 0 (valid)
            );
        }

        // Child indices
        for &ci in &child_indices {
            out.extend_from_slice(&i32le(ci));
        }

        // Attributes
        for &(k, v) in &attrs {
            out.extend_from_slice(&u32le(k));
            out.extend_from_slice(&u32le(v));
        }

        // String data
        out.extend_from_slice(&string_data);

        out
    }

    /// Write a CryXmlNode (28 bytes) into the buffer.
    ///
    /// Fields: tag_string_offset, attribute_count (u16), child_count (u16),
    ///         parent_index, first_attribute_index, first_child_index, reserved=0
    fn write_node(
        out: &mut Vec<u8>,
        tag_offset: u32,
        attr_count: u16,
        child_count: u16,
        parent_index: i32,
        first_attr_idx: i32,
        first_child_idx: i32,
    ) {
        out.extend_from_slice(&u32le(tag_offset)); // tag_string_offset
        out.extend_from_slice(&u32le(0)); // item_type (unused)
        out.extend_from_slice(&u16le(attr_count)); // attribute_count
        out.extend_from_slice(&u16le(child_count)); // child_count
        out.extend_from_slice(&i32le(parent_index)); // parent_index
        out.extend_from_slice(&i32le(first_attr_idx)); // first_attribute_index
        out.extend_from_slice(&i32le(first_child_idx)); // first_child_index
        out.extend_from_slice(&i32le(0)); // reserved
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn parses_minimal_cdf() {
        let bytes = encode_cdf(
            "objects/characters/human/male_v7/export/bhm_skeleton_v7.chr",
            &[
                (
                    "Head",
                    "objects/.../protos_human_male_face_t1_pu_head.skin",
                    "Protos",
                    "1",
                ),
                ("Eyes", "objects/.../eyes.skin", "ProtosMisc", "0"),
            ],
        );
        let cdf = parse_cdf(&bytes).expect("minimal cdf should parse");
        assert!(cdf.skeleton_path.contains("bhm_skeleton_v7.chr"));
        assert!(cdf
            .head_attachment
            .binding_skin
            .contains("protos_human_male_face_t1_pu_head.skin"));
        assert_eq!(cdf.head_attachment.deformer, DeformerKind::Protos);
        assert!(cdf.head_attachment.dynamic_dna);
    }

    #[test]
    fn rejects_no_head_attachment() {
        let bytes = encode_cdf(
            "anything.chr",
            &[("Eyes", "eyes.skin", "ProtosMisc", "0")],
        );
        let result = parse_cdf(&bytes);
        assert!(
            matches!(result, Err(CharacterError::CdfParse { .. })),
            "expected CdfParse error, got: {result:?}"
        );
    }

    #[test]
    fn rejects_malformed_binary() {
        let garbage = b"not a cdf file at all";
        let result = parse_cdf(garbage);
        assert!(
            matches!(result, Err(CharacterError::CdfParse { .. })),
            "expected CdfParse error for garbage input"
        );
    }

    #[test]
    fn parses_real_cdf_fixture() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_CDF) else { return };
        let cdf = parse_cdf(&bytes).expect("real cdf should parse");
        assert!(cdf.head_attachment.binding_skin.contains("_head.skin"));
    }
}
