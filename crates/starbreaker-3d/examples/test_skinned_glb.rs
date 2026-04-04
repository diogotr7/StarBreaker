//! Export a skinned animated GLB (e.g. landing gear).
//!
//! Parses a `.cdf` to find the `.chr` skeleton and `.skin` mesh, then combines
//! them with an optional `.dba` animation into a single GLB with proper skinning.
//!
//! Usage: test_skinned_glb [cdf_search] [dba_search] [output.glb]

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let cdf_search = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("Hornet/F7A/F7A_LandingGear/LandingGear_Front");
    let dba_search = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("Landing_Gear/Hornet_F7A.dba");
    let output = args
        .get(3)
        .map(|s| s.as_str())
        .unwrap_or("landing_gear.glb");

    let p4k = starbreaker_p4k::open_p4k().expect("no P4k");

    // Find CDF
    let cdf_lower = cdf_search.to_lowercase().replace('/', "\\");
    let cdf_entry = p4k
        .entries()
        .iter()
        .find(|e| {
            e.name.to_lowercase().contains(&cdf_lower)
                && e.name.to_lowercase().ends_with(".cdf")
        })
        .unwrap_or_else(|| panic!("No .cdf matching '{cdf_search}'"));
    eprintln!("CDF: {}", cdf_entry.name);

    // Parse CDF XML to get .chr and .skin paths
    let cdf_data = p4k.read(cdf_entry).unwrap();
    let cdf_xml = starbreaker_cryxml::from_bytes(&cdf_data).unwrap();
    let root = cdf_xml.root();

    let mut chr_path = String::new();
    let mut skin_path = String::new();
    for child in cdf_xml.node_children(root) {
        let tag = cdf_xml.node_tag(child);
        if tag == "Model" {
            chr_path = cdf_xml
                .node_attributes(child)
                .find(|(k, _)| *k == "File")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
        }
        if tag == "AttachmentList" {
            for att in cdf_xml.node_children(child) {
                let attrs: std::collections::HashMap<&str, &str> =
                    cdf_xml.node_attributes(att).collect();
                if attrs.get("Type") == Some(&"CA_SKIN") {
                    skin_path = attrs.get("Binding").unwrap_or(&"").to_string();
                }
            }
        }
    }
    eprintln!("CHR: {chr_path}");
    eprintln!("SKIN: {skin_path}");

    let load = |path: &str| -> Vec<u8> {
        let p4k_path = format!("Data\\{}", path.replace('/', "\\"));
        let entry = p4k
            .entry_case_insensitive(&p4k_path)
            .unwrap_or_else(|| panic!("Not found: {p4k_path}"));
        p4k.read(entry).unwrap()
    };

    let chr_data = load(&chr_path);
    let skin_path_m = skin_path.replace(".skin", ".skinm").replace(".cgf", ".cgfm");
    let skin_data = load(&skin_path_m);

    // Find DBA
    let dba_lower = dba_search.to_lowercase().replace('/', "\\");
    let dba_data = p4k
        .entries()
        .iter()
        .find(|e| {
            let n = e.name.to_lowercase();
            n.contains(&dba_lower) && n.ends_with(".dba")
        })
        .map(|e| {
            eprintln!("DBA: {}", e.name);
            p4k.read(e).unwrap()
        });

    let glb = starbreaker_3d::skinned_mesh_to_glb(&skin_data, &chr_data, dba_data.as_deref())
        .expect("failed to build GLB");

    std::fs::write(output, &glb).unwrap();
    eprintln!(
        "Wrote {} ({:.1} KB)",
        output,
        glb.len() as f64 / 1024.0
    );
}
