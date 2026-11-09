use starbreaker_cryxml::CryXml;
use starbreaker_datacore::{Database, types::Record};
use starbreaker_p4k::MappedP4k;

/// Visit base then modification XML. Missing optional vehicle data must not
/// prevent geometry or light export; each failed source is diagnosed separately.
pub(super) fn visit_vehicle_xml(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    mut visit: impl FnMut(&CryXml, bool),
) {
    let query = |field: &str| {
        let path = match db.compile_path::<String>(
            record.struct_id(),
            &format!("Components[VehicleComponentParams].{field}"),
        ) {
            Ok(path) => path,
            Err(error) => {
                log::warn!("Vehicle XML query {field}: {error}");
                return None;
            }
        };
        match db.query_single::<String>(&path, record) {
            Ok(value) => value.filter(|value| !value.is_empty()),
            Err(error) => {
                log::warn!("Vehicle XML query {field}: {error}");
                None
            }
        }
    };
    let Some(base) = query("vehicleDefinition") else {
        return;
    };
    let mut paths = vec![base.clone()];
    if let Some(modification) = query("modification") {
        let normalized = base.replace('\\', "/");
        if let Some((dir, filename)) = normalized.rsplit_once('/') {
            let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
            paths.push(format!("{dir}/Modifications/{stem}_{modification}.xml"));
        }
    }
    for (index, path) in paths.iter().enumerate() {
        let path = super::datacore_path_to_p4k(path);
        visit_xml_source(
            &path,
            || {
                p4k.entry_case_insensitive(&path)
                    .ok_or_else(|| "archive entry missing".to_owned())
                    .and_then(|entry| p4k.read(entry).map_err(|e| e.to_string()))
            },
            |xml| visit(xml, index != 0),
        );
    }
}

fn visit_xml_source(
    path: &str,
    read: impl FnOnce() -> Result<Vec<u8>, String>,
    visit: impl FnOnce(&CryXml),
) {
    let result = read().and_then(|bytes| {
        let xml = starbreaker_cryxml::from_bytes(&bytes).map_err(|e| e.to_string())?;
        visit(&xml);
        Ok(())
    });
    if let Err(error) = result {
        log::warn!("Vehicle XML {path}: {error}; continuing without this source");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_invalid_base_does_not_prevent_valid_modification() {
        let mut visited = 0;
        for source in [
            Err("missing".into()),
            Ok(vec![0]),
            Ok(crate::test_xml::encode("<Vehicle/>")),
        ] {
            visit_xml_source("synthetic.xml", || source, |_| visited += 1);
        }
        assert_eq!(visited, 1);
    }
}
