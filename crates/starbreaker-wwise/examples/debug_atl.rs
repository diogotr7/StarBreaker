/// Debug ATL CryXmlB parsing — dump the tree structure
use starbreaker_cryxml::{from_bytes, CryXml, CryXmlNode};
use std::{env, fs};

fn dump_node(xml: &CryXml, node: &CryXmlNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let tag = xml.node_tag(node);
    let attrs: Vec<_> = xml.node_attributes(node).collect();
    let attr_str: String = attrs
        .iter()
        .map(|(k, v)| {
            let v_short = if v.len() > 40 { &v[..40] } else { v };
            format!("{}=\"{}\"", k, v_short)
        })
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("{}<{} {}>", indent, tag, attr_str);
    if depth < 7 {
        for child in xml.node_children(node) {
            dump_node(xml, child, depth + 1);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: debug_atl <file.xml>");
        std::process::exit(1);
    });
    let data = fs::read(&path)?;
    let xml = from_bytes(&data)?;
    dump_node(&xml, xml.root(), 0);
    Ok(())
}
