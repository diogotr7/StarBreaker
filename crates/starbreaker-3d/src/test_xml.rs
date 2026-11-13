//! Synthetic CryXmlB fixtures for authored material and light regression tests.

pub(crate) fn encode(text: &str) -> Vec<u8> {
    use quick_xml::{Reader, events::Event};
    #[derive(Default)]
    struct Node {
        tag: u32,
        parent: u32,
        attrs: Vec<(u32, u32)>,
        children: Vec<u32>,
    }
    fn string(pool: &mut Vec<u8>, s: &str) -> u32 {
        let offset = pool.len() as u32;
        pool.extend(s.as_bytes());
        pool.push(0);
        offset
    }
    let mut reader = Reader::from_str(text);
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut strings = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let index = nodes.len();
                let parent = stack.last().copied();
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            string(&mut strings, a.key.as_ref()),
                            string(&mut strings, a.value.as_ref()),
                        )
                    })
                    .collect();
                let node = Node {
                    tag: string(&mut strings, e.name().as_ref()),
                    parent: parent.map_or(u32::MAX, |i| i as u32),
                    attrs,
                    children: Vec::new(),
                };
                if let Some(p) = parent {
                    nodes[p].children.push(index as u32);
                }
                nodes.push(node);
                // Quick-xml's consumed source distinguishes empty tags without losing the event.
                let end = reader.buffer_position() as usize;
                if !text[..end].ends_with("/>") {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut node_bytes = Vec::new();
    let mut children = Vec::new();
    let mut attrs = Vec::new();
    for n in &nodes {
        node_bytes.extend(n.tag.to_le_bytes());
        node_bytes.extend(0u32.to_le_bytes());
        node_bytes.extend((n.attrs.len() as u16).to_le_bytes());
        node_bytes.extend((n.children.len() as u16).to_le_bytes());
        for v in [
            n.parent,
            attrs.len() as u32 / 8,
            children.len() as u32 / 4,
            0,
        ] {
            node_bytes.extend(v.to_le_bytes());
        }
        for c in &n.children {
            children.extend(c.to_le_bytes());
        }
        for (k, v) in &n.attrs {
            attrs.extend(k.to_le_bytes());
            attrs.extend(v.to_le_bytes());
        }
    }
    let child_offset = 44 + node_bytes.len() as u32;
    let attr_offset = child_offset + children.len() as u32;
    let string_offset = attr_offset + attrs.len() as u32;
    let mut out = b"CryXmlB\0".to_vec();
    for v in [
        string_offset + strings.len() as u32,
        44,
        nodes.len() as u32,
        attr_offset,
        attrs.len() as u32 / 8,
        child_offset,
        children.len() as u32 / 4,
        string_offset,
        strings.len() as u32,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(node_bytes);
    out.extend(children);
    out.extend(attrs);
    out.extend(strings);
    out
}
