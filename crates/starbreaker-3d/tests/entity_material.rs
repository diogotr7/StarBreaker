//! The material an entity record draws with, against real data: the
//! entity's `Material` node (`SMaterialNodeParams`) overrides the material
//! its geometry names, and the geometry's is used when the node has none.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_p4k::MappedP4k;

fn p4k() -> Option<MappedP4k> {
    let Ok(found) = starbreaker_common::discover::find_p4k() else {
        eprintln!("Data.p4k not found, skipping");
        return None;
    };
    Some(MappedP4k::open(&found.path).expect("open p4k"))
}

fn material(index: &EntityIndex, name: &str) -> Option<String> {
    let record = index.find_record(name).unwrap_or_else(|| panic!("no record {name}"));
    index.query_geometry_paths(record).1.map(|m| m.to_ascii_lowercase())
}

#[test]
fn material_node_overrides_the_geometry_material() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);

    // Geometry names no material; the node does.
    assert_eq!(
        material(&index, "Carryable_2H_FL_MissionItem_iasi_box").as_deref(),
        Some("objects/props/flair/vehicle_box/vehicle_box_1_iasi_a.mtl"),
    );
    // Both set and different: the geometry's is the template's (the dog
    // envelope); the node's is this record's own.
    assert_eq!(
        material(&index, "carryable_1h_cy_lunar_envelope_1_pig_a").as_deref(),
        Some("objects/props/seasonal/lunar/lunar_envelope_1_pig_a.mtl"),
    );
}
