//! Vehicle XML MultiLight ports and attachment-point bindings (including modifications).
use crate::error::Error;
use starbreaker_cryxml::{CryXml, CryXmlNode};
use starbreaker_datacore::{Database, types::Record};
use starbreaker_p4k::MappedP4k;
use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VehicleLightPort {
    pub name: String,
    /// Attachment point name -> authored bone/helper name.
    pub attachment_points: BTreeMap<String, String>,
}

pub fn load_vehicle_light_ports(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
) -> Result<Vec<VehicleLightPort>, Error> {
    let query = |field: &str| -> Result<Option<String>, Error> {
        let path = db.compile_path::<String>(
            record.struct_id(),
            &format!("Components[VehicleComponentParams].{field}"),
        )?;
        Ok(db.query_single::<String>(&path, record)?)
    };
    let Some(base) = query("vehicleDefinition")?.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut paths = vec![base.clone()];
    if let Some(modification) = query("modification")?.filter(|s| !s.is_empty()) {
        let normalized = base.replace('\\', "/");
        if let Some((dir, filename)) = normalized.rsplit_once('/') {
            let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
            paths.push(format!("{dir}/Modifications/{stem}_{modification}.xml"));
        }
    }
    let mut ports = BTreeMap::new();
    for path in paths {
        let path = super::datacore_path_to_p4k(&path);
        let bytes = p4k
            .read_file(&path)
            .map_err(|e| Error::P4kRead(format!("{path}: {e}")))?;
        let xml = starbreaker_cryxml::from_bytes(&bytes)?;
        collect(&xml, xml.root(), &mut ports);
    }
    Ok(ports.into_values().collect())
}

fn collect(xml: &CryXml, node: &CryXmlNode, ports: &mut BTreeMap<String, VehicleLightPort>) {
    fn descendants<'a>(xml: &'a CryXml, node: &'a CryXmlNode, out: &mut Vec<&'a CryXmlNode>) {
        for child in xml.node_children(node) {
            out.push(child);
            descendants(xml, child, out);
        }
    }
    if xml.node_tag(node) == "Part" {
        let name = xml
            .node_attributes(node)
            .find(|(k, _)| *k == "name")
            .map(|(_, v)| v);
        if let Some(name) = name {
            for item in xml
                .node_children(node)
                .filter(|n| xml.node_tag(n) == "ItemPort")
            {
                let mut children = Vec::new();
                descendants(xml, item, &mut children);
                let is_light = children.iter().any(|n| {
                    xml.node_tag(n) == "Type"
                        && xml
                            .node_attributes(n)
                            .any(|(k, v)| k == "type" && v == "MultiLight")
                });
                if is_light || ports.contains_key(name) {
                    let port = ports
                        .entry(name.to_owned())
                        .or_insert_with(|| VehicleLightPort {
                            name: name.to_owned(),
                            attachment_points: BTreeMap::new(),
                        });
                    for n in children
                        .iter()
                        .filter(|n| xml.node_tag(n) == "AttachmentPoint")
                    {
                        let attrs: BTreeMap<_, _> = xml.node_attributes(n).collect();
                        if let (Some(name), Some(bone)) = (attrs.get("name"), attrs.get("bone")) {
                            port.attachment_points
                                .insert((*name).to_owned(), (*bone).to_owned());
                        }
                    }
                }
            }
        }
    }
    for child in xml.node_children(node) {
        collect(xml, child, ports);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multilights_keep_point_to_bone_bindings_and_modification_overrides() {
        let mut ports = BTreeMap::new();
        for text in [
            r#"<Vehicle><Part name="arbitrary"><ItemPort><Types><Type type="MultiLight"/></Types><AttachmentPoints><AttachmentPoint name="tip" bone="joint"/></AttachmentPoints></ItemPort></Part></Vehicle>"#,
            r#"<Vehicle><Part name="arbitrary"><ItemPort><AttachmentPoints><AttachmentPoint name="tip" bone="other"/></AttachmentPoints></ItemPort></Part></Vehicle>"#,
        ] {
            let bytes = crate::test_xml::encode(text);
            let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
            collect(&xml, xml.root(), &mut ports);
        }
        assert_eq!(ports.len(), 1);
        assert_eq!(ports["arbitrary"].attachment_points["tip"], "other");
    }
}

/// Resolve only the scene owners' port definitions; child vehicles use their own XML.
pub(crate) fn load_owner_light_ports(
    db: &Database,
    p4k: &MappedP4k,
    owners: &[String],
) -> Result<std::collections::HashMap<String, Vec<VehicleLightPort>>, Error> {
    let mut result = std::collections::HashMap::new();
    for record in db.records_by_type_name("EntityClassDefinition") {
        let name = db.resolve_string2(record.name_offset);
        if let Some(owner) = owners.iter().find(|owner| name.eq_ignore_ascii_case(owner)) {
            result.insert(owner.clone(), load_vehicle_light_ports(db, p4k, record)?);
        }
    }
    Ok(result)
}
