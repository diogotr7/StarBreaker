//! Dump raw XML texture slots from a MTL file.
use starbreaker_cryxml::{CryXml, CryXmlNode};

fn main() {
    let search = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: dump_mtl_raw <p4k_path_substring>");
        std::process::exit(1);
    }).to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let entry = p4k.entries().iter().find(|e| {
        e.name.to_lowercase().contains(&search) && e.name.to_lowercase().ends_with(".mtl")
    }).unwrap_or_else(|| {
        eprintln!("No .mtl matching '{search}'");
        std::process::exit(1);
    });

    eprintln!("File: {}", entry.name);
    let data = p4k.read_file(&entry.name).expect("failed to read");
    let xml = starbreaker_cryxml::from_bytes(&data).expect("failed to parse");
    let root = xml.root();
    walk(&xml, root, 0);
}

fn attr<'a>(xml: &'a CryXml, node: &CryXmlNode, key: &str) -> Option<&'a str> {
    xml.node_attributes(node).find(|(k, _)| *k == key).map(|(_, v)| v)
}

fn walk(xml: &CryXml, node: &CryXmlNode, depth: usize) {
    let tag = xml.node_tag(node);
    if tag == "Material" {
        let name = attr(xml, node, "Name").unwrap_or("");
        let shader = attr(xml, node, "Shader").unwrap_or("");
        let gen_mask = attr(xml, node, "StringGenMask").unwrap_or("");
        let opacity = attr(xml, node, "Opacity").unwrap_or("1");
        let alpha_test = attr(xml, node, "AlphaTest").unwrap_or("0");
        println!("\n{}Material: {} (shader={}, opacity={}, alphaTest={})",
            "  ".repeat(depth), name, shader, opacity, alpha_test);
        if !gen_mask.is_empty() {
            println!("{}  GenMask: {}", "  ".repeat(depth), gen_mask);
        }
        // Dump ALL attributes for decal materials
        if gen_mask.contains("%DECAL") || gen_mask.contains("STENCIL") || shader == "MeshDecal" {
            println!("{}  [ALL ATTRS]:", "  ".repeat(depth));
            for (k, v) in xml.node_attributes(node) {
                println!("{}    {}={}", "  ".repeat(depth), k, v);
            }
        }
        // Find Textures child
        for child in xml.node_children(node) {
            if xml.node_tag(child) == "Textures" {
                for tex in xml.node_children(child) {
                    if xml.node_tag(tex) == "Texture" {
                        let map = attr(xml, tex, "Map").unwrap_or("?");
                        let file = attr(xml, tex, "File").unwrap_or("");
                        if !file.is_empty() {
                            println!("{}  Tex[{}]: {}", "  ".repeat(depth), map, file);
                        }
                    }
                }
            }
        }
    }
    for child in xml.node_children(node) {
        walk(xml, child, depth + 1);
    }
}
