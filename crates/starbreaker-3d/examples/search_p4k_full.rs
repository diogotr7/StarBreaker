//! Search P4k entries by substring.
use std::env;

fn main() {
    let search = env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: search_p4k <substring>");
            std::process::exit(1);
        })
        .to_lowercase();

    let ext_filter = env::args().nth(2).map(|s| s.to_lowercase());

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let mut count = 0;
    for entry in p4k.entries() {
        let name = entry.name.to_lowercase();
        if name.contains(&search) {
            if let Some(ref ext) = ext_filter
                && !name.ends_with(ext.as_str())
            {
                continue;
            }
            println!("{} ({} bytes)", entry.name, entry.uncompressed_size);
            count += 1;
            if count > 50 {
                println!("... (truncated)");
                break;
            }
        }
    }
    if count == 0 {
        println!("No matches");
    }
}
