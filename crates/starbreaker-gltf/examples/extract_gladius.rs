use std::fs;

fn main() {
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let files = [
        (
            r"Data\Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.cga",
            "gladius.cga",
        ),
        (
            r"Data\Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.cgam",
            "gladius.cgam",
        ),
    ];

    for (p4k_path, out_name) in &files {
        if let Some(entry) = p4k.entry_case_insensitive(p4k_path) {
            let data = p4k.read(entry).unwrap();
            fs::write(out_name, &data).unwrap();
            eprintln!("Extracted {p4k_path} -> {out_name} ({} bytes)", data.len());
        } else {
            eprintln!("Not found: {p4k_path}");
        }
    }
}
