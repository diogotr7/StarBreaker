use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\PTU\Data.p4k";

fn main() {
    let search = std::env::args()
        .nth(1)
        .expect("Usage: search_p4k <pattern>");
    let search_lower = search.to_lowercase();
    let p4k = MappedP4k::open(DEFAULT_P4K).expect("failed to open Data.p4k");
    for e in p4k.entries() {
        if e.name.to_lowercase().contains(&search_lower) {
            println!("{} ({} bytes)", e.name, e.uncompressed_size);
        }
    }
}
