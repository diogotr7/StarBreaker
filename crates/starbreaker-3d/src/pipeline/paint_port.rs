//! The hull's paint port and the paint items it accepts.
//!
//! A vehicle's paint port is the `<ItemPort>` in its implementation XML whose
//! `<Types>` list `Paints`. Its `requiredTags` must all be among a paint
//! item's `AttachDef.Tags` for the item to fit; the base XML's
//! `<Modifications><Modification name=…><Elems>` block overrides the value per
//! hull (`<Elem idRef=<port id> name="requiredTags" …/>`), so hulls sharing one
//! geometry (the 300 series) each accept only their own paints.
//!
//! - [`paint_ports`]: the paint ports of a parsed vehicle XML for one modification.
//! - [`port_accepts`]: whether an item's tags satisfy a port's required tags.
//! - [`accepted_paint_tags`]: the `@Tag` selectors of every paint item an
//!   entity's paint port accepts, plus the item its default loadout fits there.

use std::collections::HashSet;

use starbreaker_cryxml::{CryXml, CryXmlNode};
use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::LoadoutNode;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

/// `AttachDef.Type` of a paint item and the `<Type type=…>` of its port.
const PAINT_ITEM_TYPE: &str = "Paints";

/// A paint port: its part name (the loadout port name) and required tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaintPort {
    pub name: String,
    pub required_tags: Vec<String>,
}

fn attr<'x>(xml: &'x CryXml, node: &'x CryXmlNode, name: &str) -> Option<&'x str> {
    xml.node_attributes(node)
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

fn is_paint_item_port(xml: &CryXml, port: &CryXmlNode) -> bool {
    xml.node_children(port)
        .filter(|c| xml.node_tag(c) == "Types")
        .flat_map(|types| xml.node_children(types))
        .any(|t| xml.node_tag(t) == "Type" && attr(xml, t, "type").is_some_and(|v| v.eq_ignore_ascii_case(PAINT_ITEM_TYPE)))
}

fn collect_paint_ports(xml: &CryXml, node: &CryXmlNode, out: &mut Vec<(String, Option<String>, Vec<String>)>) {
    if xml.node_tag(node) == "Part" {
        let name = attr(xml, node, "name").unwrap_or("");
        for port in xml.node_children(node).filter(|c| xml.node_tag(c) == "ItemPort") {
            if !name.is_empty() && is_paint_item_port(xml, port) {
                let id = attr(xml, port, "id").map(str::to_owned);
                out.push((name.to_owned(), id, words(attr(xml, port, "requiredTags").unwrap_or(""))));
            }
        }
    }
    for child in xml.node_children(node) {
        collect_paint_ports(xml, child, out);
    }
}

/// The `requiredTags` overrides of `modification`'s `<Elems>`, by port id.
fn required_tag_overrides(xml: &CryXml, modification: &str) -> Vec<(String, Vec<String>)> {
    let mut overrides = Vec::new();
    let root = xml.root();
    for mods in xml.node_children(root).filter(|c| xml.node_tag(c) == "Modifications") {
        for m in xml.node_children(mods).filter(|c| xml.node_tag(c) == "Modification") {
            if !attr(xml, m, "name").is_some_and(|n| n.eq_ignore_ascii_case(modification)) {
                continue;
            }
            for elems in xml.node_children(m).filter(|c| xml.node_tag(c) == "Elems") {
                for elem in xml.node_children(elems).filter(|c| xml.node_tag(c) == "Elem") {
                    if !attr(xml, elem, "name").is_some_and(|n| n.eq_ignore_ascii_case("requiredTags")) {
                        continue;
                    }
                    if let Some(id) = attr(xml, elem, "idRef") {
                        overrides.push((id.to_owned(), words(attr(xml, elem, "value").unwrap_or(""))));
                    }
                }
            }
        }
    }
    overrides
}

/// The paint ports of a base vehicle XML, with `modification`'s
/// `requiredTags` overrides applied.
pub(crate) fn paint_ports(xml: &CryXml, modification: Option<&str>) -> Vec<PaintPort> {
    let mut found = Vec::new();
    collect_paint_ports(xml, xml.root(), &mut found);
    let overrides = modification
        .filter(|m| !m.is_empty())
        .map(|m| required_tag_overrides(xml, m))
        .unwrap_or_default();
    found
        .into_iter()
        .map(|(name, id, required_tags)| {
            let required_tags = id
                .and_then(|id| overrides.iter().rev().find(|(r, _)| r.eq_ignore_ascii_case(&id)).map(|(_, t)| t.clone()))
                .unwrap_or(required_tags);
            PaintPort { name, required_tags }
        })
        .collect()
}

/// Whether an item whose `AttachDef.Tags` are `item_tags` fits a port
/// requiring `required_tags`: every required tag is one of the item's tags
/// (any case).
pub(crate) fn port_accepts(required_tags: &[String], item_tags: &str) -> bool {
    required_tags
        .iter()
        .all(|req| item_tags.split_whitespace().any(|t| t.eq_ignore_ascii_case(req)))
}

/// The `@Tag` selector of a paint item's `AttachDef.Tags`.
pub(crate) fn paint_selector(item_tags: &str) -> Option<&str> {
    item_tags.split_whitespace().find_map(|t| t.strip_prefix('@'))
}

fn vehicle_paint_ports(db: &Database, p4k: &MappedP4k, record: &Record) -> Vec<PaintPort> {
    let modification = db
        .compile_path::<String>(record.struct_id(), "Components[VehicleComponentParams].modification")
        .ok()
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten());
    let mut ports = Vec::new();
    super::vehicle_xml::visit_vehicle_xml(db, p4k, record, |xml, is_modification| {
        // A modification's own patch file can carry a stale copy of the port;
        // the base XML's `<Modifications>` block is what selects per hull.
        if !is_modification {
            ports = paint_ports(xml, modification.as_deref());
        }
    });
    ports
}

/// The `@Tag` selectors (lower-case) of every paint item the entity's paint
/// port accepts, plus that of the item its default loadout fits on the port.
/// `None` when the entity has no paint port, so nothing is filtered.
pub(crate) fn accepted_paint_tags(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    root: &LoadoutNode,
) -> Option<HashSet<String>> {
    let ports = vehicle_paint_ports(db, p4k, record);
    if ports.is_empty() {
        return None;
    }
    let type_path = db
        .compile_rooted::<String>("EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Type")
        .ok()?;
    let tags_path = db
        .compile_rooted::<String>("EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Tags")
        .ok()?;
    let item_tags = |item: &Record| db.query_single::<String>(&tags_path, item).ok().flatten().unwrap_or_default();

    let mut accepted = HashSet::new();
    for item in db.records_by_type_name("EntityClassDefinition") {
        if !db.is_main_record(item) {
            continue;
        }
        let is_paint = db
            .query_single::<String>(&type_path, item)
            .ok()
            .flatten()
            .is_some_and(|t| t.eq_ignore_ascii_case(PAINT_ITEM_TYPE));
        if !is_paint {
            continue;
        }
        let tags = item_tags(item);
        if ports.iter().any(|port| port_accepts(&port.required_tags, &tags)) {
            accepted.extend(paint_selector(&tags).map(str::to_ascii_lowercase));
        }
    }
    for port in &ports {
        let equipped = root.children.iter().find(|c| c.item_port_name.eq_ignore_ascii_case(&port.name));
        if let Some(node) = equipped {
            accepted.extend(paint_selector(&item_tags(&node.record)).map(str::to_ascii_lowercase));
        }
    }
    Some(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(modification: Option<&str>) -> Vec<PaintPort> {
        let bytes = crate::test_xml::encode(HULLS);
        paint_ports(&starbreaker_cryxml::from_bytes(&bytes).unwrap(), modification)
    }

    const HULLS: &str = r#"<Vehicle name="SyntheticHull">
  <Parts>
    <Part name="body">
      <Parts>
        <Part name="hardpoint_paint" class="ItemPort">
          <ItemPort minSize="1" maxSize="1" requiredTags="HullA_Paint" id="modPaint">
            <Types><Type type="Paints" /></Types>
          </ItemPort>
        </Part>
        <Part name="hardpoint_seat_paint" class="ItemPort">
          <ItemPort minSize="1" maxSize="1" requiredTags="HullA_Seat">
            <Types><Type type="Misc" /></Types>
          </ItemPort>
        </Part>
      </Parts>
    </Part>
  </Parts>
  <Modifications>
    <Modification name="HullB">
      <Elems>
        <Elem idRef="modPaint" name="requiredTags" value="HullB_Paint" />
        <Elem idRef="modPaint" name="flags" value="invisible" />
      </Elems>
    </Modification>
  </Modifications>
</Vehicle>"#;

    #[test]
    fn the_paint_port_is_the_item_port_typed_paints() {
        assert_eq!(
            ports(None),
            vec![PaintPort { name: "hardpoint_paint".into(), required_tags: vec!["HullA_Paint".into()] }]
        );
    }

    /// Hulls sharing one implementation XML differ by their modification's
    /// `requiredTags` override on the port id.
    #[test]
    fn a_modification_overrides_the_paint_ports_required_tags() {
        assert_eq!(ports(Some("HullB"))[0].required_tags, vec!["HullB_Paint".to_string()]);
        assert_eq!(ports(Some("HullC"))[0].required_tags, vec!["HullA_Paint".to_string()]);
    }

    #[test]
    fn a_port_accepts_items_carrying_all_its_required_tags() {
        let a = vec!["HullA_Paint".to_string()];
        assert!(port_accepts(&a, "Family_Base hulla_paint webcustom @HullA_Red"));
        assert!(port_accepts(&a, "Family_Base webcustom @Shared_Blue HullA_Paint HullB_Paint"));
        assert!(!port_accepts(&a, "Family_Base HullB_Paint webcustom @HullB_Red"), "a sibling hull's paint");
        assert!(!port_accepts(&a, "Family_Base webcustom @HullA_Selector_Only"), "an @selector is not a tag");
        assert!(port_accepts(&[], "anything"));
    }

    #[test]
    fn the_paint_selector_is_the_at_token() {
        assert_eq!(paint_selector("Family_Base HullA_Paint webcustom @HullA_Red"), Some("HullA_Red"));
        assert_eq!(paint_selector("Family_Base"), None);
    }
}
