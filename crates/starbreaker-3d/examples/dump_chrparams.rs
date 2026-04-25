use starbreaker_cryxml::{from_bytes, CryXml, CryXmlNode};

fn main() {
    let p = std::env::args().nth(1).expect("path");
    let bytes = std::fs::read(&p).unwrap();
    let xml = from_bytes(&bytes).expect("parse");
    walk(&xml, xml.root(), 0);
}

fn walk(xml: &CryXml, n: &CryXmlNode, depth: usize) {
    let tag = xml.node_tag(n);
    let attrs: Vec<(&str, &str)> = xml.node_attributes(n).collect();
    println!("{}<{tag} {attrs:?}>", "  ".repeat(depth));
    for c in xml.node_children(n) {
        walk(xml, c, depth + 1);
    }
}
