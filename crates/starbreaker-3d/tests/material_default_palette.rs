//! A hull whose DataCore entity names no palette is drawn with the palette
//! its exterior material names as `DefaultPalette` (the Javelin): the root's
//! `palette_id` is that palette, and palettes.json carries its colours.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout_indexed};

#[test]
fn a_hull_without_an_entity_palette_uses_its_material_default_palette() {
    let Ok(found) = starbreaker_common::discover::find_p4k() else {
        eprintln!("skipping: no Data.p4k installed");
        return;
    };
    let p4k = starbreaker_p4k::MappedP4k::open(found.path).unwrap();
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    let record = index.find_record("AEGS_Javelin").expect("Javelin");
    let tree = resolve_loadout_indexed(&index, record);
    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Glb, ..Default::default() };
    let files = starbreaker_3d::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts).unwrap().decomposed.unwrap().files;
    let json = |suffix: &str| -> serde_json::Value {
        let file = files.iter().find(|f| f.relative_path.ends_with(suffix)).unwrap_or_else(|| panic!("no {suffix}"));
        serde_json::from_slice(&file.bytes).unwrap()
    };

    let scene = json("/scene.json");
    assert_eq!(scene["root_entity"]["palette_id"], "palette/aegs_javelin_default");

    let palettes = json("/palettes.json");
    let palette = palettes["palettes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "palette/aegs_javelin_default")
        .expect("the default palette is in palettes.json");
    for channel in ["primary", "secondary", "tertiary"] {
        assert!(palette[channel].as_array().is_some_and(|c| c.len() == 3), "{channel}: {palette}");
    }
}
