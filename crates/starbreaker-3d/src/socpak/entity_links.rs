//! Entity identity, authored links and inline loadouts of `.soc` `<Entity>` nodes.
//!
//! Inside an object container every entity carries an `EntityCryGUID`, and
//! entities that drive another one name it in `<EntityLinks><Link
//! TargetCryGUID="…"/></EntityLinks>`: a door control panel links the door it
//! operates, a proximity sensor links the door it opens. An entity may also
//! author its own `PropertiesDataCore/EntityComponentDefaultLoadout`, which
//! replaces its entity class's default loadout (e.g. a door's control panels
//! at its `control_panel_a` / `control_panel_b` item ports).
//!
//! [`entity_ref`] reads the identity, links and inline loadout of one entity
//! into a `SocEntityRef` (`types.rs`) carried on its `InteriorMesh`. No entity class or
//! name is consulted here.

use starbreaker_cryxml::{CryXml, CryXmlNode};

pub use crate::types::{InlineLoadoutEntry, SocEntityRef};

/// Identity, links and inline loadout of `entity`.
pub(crate) fn entity_ref(xml: &CryXml, entity: &CryXmlNode) -> SocEntityRef {
    let entity_id = xml
        .node_attributes(entity)
        .find(|(key, _)| *key == "EntityCryGUID")
        .map(|(_, value)| value.to_string())
        .filter(|value| super::is_nonzero_guid(value));
    SocEntityRef {
        entity_id,
        links: entity_links(xml, entity),
        inline_loadout: inline_loadout(xml, entity),
    }
}

fn child<'a>(xml: &CryXml<'a>, node: &CryXmlNode, tag: &str) -> Option<&'a CryXmlNode> {
    xml.node_children(node).find(|c| xml.node_tag(c) == tag)
}

fn attr<'a>(xml: &CryXml<'a>, node: &CryXmlNode, name: &str) -> Option<&'a str> {
    xml.node_attributes(node)
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

fn entity_links(xml: &CryXml, entity: &CryXmlNode) -> Vec<String> {
    let Some(links) = child(xml, entity, "EntityLinks") else {
        return Vec::new();
    };
    xml.node_children(links)
        .filter(|link| xml.node_tag(link) == "Link")
        .filter_map(|link| attr(xml, link, "TargetCryGUID"))
        .filter(|target| super::is_nonzero_guid(target))
        .map(str::to_string)
        .collect()
}

fn inline_loadout(xml: &CryXml, entity: &CryXmlNode) -> Option<Vec<InlineLoadoutEntry>> {
    let properties = child(xml, entity, "PropertiesDataCore")?;
    let component = child(xml, properties, "EntityComponentDefaultLoadout")?;
    manual_entries(xml, child(xml, component, "loadout")?)
}

/// Entries of a `loadout` node, `None` unless it is a manual loadout (a
/// `<NullPtr>` or another loadout type authors no entries here).
fn manual_entries(xml: &CryXml, loadout: &CryXmlNode) -> Option<Vec<InlineLoadoutEntry>> {
    if attr(xml, loadout, "__type") != Some("SItemPortLoadoutManualParams") {
        return None;
    }
    let Some(entries) = child(xml, loadout, "entries") else {
        return Some(Vec::new());
    };
    Some(
        xml.node_children(entries)
            .filter(|entry| xml.node_tag(entry) == "SItemPortLoadoutEntryParams")
            .map(|entry| InlineLoadoutEntry {
                item_port_name: attr(xml, entry, "itemPortName").unwrap_or_default().to_string(),
                entity_class_name: attr(xml, entry, "entityClassName").unwrap_or_default().to_string(),
                entity_class_guid: attr(xml, entry, "entityClassReference")
                    .filter(|guid| super::is_nonzero_guid(guid))
                    .map(str::to_string),
                children: child(xml, entry, "loadout").and_then(|nested| manual_entries(xml, nested)),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> SocEntityRef {
        let bytes = crate::test_xml::encode(text);
        let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
        entity_ref(&xml, xml.root())
    }

    #[test]
    fn reads_id_and_links_in_authored_order() {
        let entity = parse(
            r#"<Entity EntityCryGUID="aaaa0000-0000-0000-0000-000000000001">
                 <EntityLinks>
                   <Link TargetCryGUID="bbbb0000-0000-0000-0000-000000000002" Name="NewLink"/>
                   <Link TargetCryGUID="00000000-0000-0000-0000-000000000000" Name="Null"/>
                   <Link TargetCryGUID="cccc0000-0000-0000-0000-000000000003" Name="Other"/>
                 </EntityLinks>
               </Entity>"#,
        );
        assert_eq!(entity.entity_id.as_deref(), Some("aaaa0000-0000-0000-0000-000000000001"));
        assert_eq!(
            entity.links,
            ["bbbb0000-0000-0000-0000-000000000002", "cccc0000-0000-0000-0000-000000000003"]
        );
        assert_eq!(entity.inline_loadout, None, "no loadout authored: the class loadout applies");
    }

    #[test]
    fn reads_inline_manual_loadout_entries() {
        let entity = parse(
            r#"<Entity EntityCryGUID="aaaa0000-0000-0000-0000-000000000001">
                 <PropertiesDataCore>
                   <EntityComponentDefaultLoadout __type="SEntityComponentDefaultLoadoutParams">
                     <loadout __type="SItemPortLoadoutManualParams">
                       <entries>
                         <SItemPortLoadoutEntryParams itemPortName="port_a" entityClassName="" entityClassReference="dddd0000-0000-0000-0000-000000000004">
                           <loadout __type="&lt;NullPtr&gt;"/>
                         </SItemPortLoadoutEntryParams>
                         <SItemPortLoadoutEntryParams itemPortName="port_b" entityClassName="Synthetic_Item" entityClassReference="00000000-0000-0000-0000-000000000000">
                           <loadout __type="SItemPortLoadoutManualParams">
                             <entries>
                               <SItemPortLoadoutEntryParams itemPortName="nested" entityClassName="Synthetic_Nested"/>
                             </entries>
                           </loadout>
                         </SItemPortLoadoutEntryParams>
                       </entries>
                     </loadout>
                   </EntityComponentDefaultLoadout>
                 </PropertiesDataCore>
               </Entity>"#,
        );
        let entries = entity.inline_loadout.expect("inline manual loadout");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].item_port_name, "port_a");
        assert_eq!(entries[0].entity_class_guid.as_deref(), Some("dddd0000-0000-0000-0000-000000000004"));
        assert_eq!(entries[0].children, None, "a null nested loadout leaves the item's class loadout");
        assert_eq!(entries[1].entity_class_name, "Synthetic_Item");
        assert_eq!(entries[1].entity_class_guid, None);
        let nested = entries[1].children.as_ref().expect("nested manual loadout");
        assert_eq!(nested[0].item_port_name, "nested");
        assert_eq!(nested[0].entity_class_name, "Synthetic_Nested");
    }

    #[test]
    fn an_empty_inline_manual_loadout_overrides_the_class_loadout() {
        let entity = parse(
            r#"<Entity>
                 <PropertiesDataCore>
                   <EntityComponentDefaultLoadout>
                     <loadout __type="SItemPortLoadoutManualParams"><entries/></loadout>
                   </EntityComponentDefaultLoadout>
                 </PropertiesDataCore>
               </Entity>"#,
        );
        assert_eq!(entity.entity_id, None);
        assert_eq!(entity.inline_loadout, Some(Vec::new()));
    }
}
