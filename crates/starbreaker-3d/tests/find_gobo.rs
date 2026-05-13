#[test]
#[ignore]
fn find_gobo_dds_files() {
    use starbreaker_p4k::MappedP4k;

    let p4k_path = "/home/tom/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k";
    let p4k = MappedP4k::open(p4k_path).expect("Failed to open P4k");

    println!("\n=== Finding gobo DDS files ===");

    // Search for DDS files with "light" and "spot" in name
    let mut found = 0;
    for entry in p4k.entries() {
        let path_str = entry.name.as_str();
        if path_str.to_lowercase().contains("light")
            && path_str.to_lowercase().contains("spot")
            && path_str.ends_with(".dds")
        {
            println!("Found: {}", path_str);
            found += 1;
            if found > 5 {
                break;
            }
        }
    }

    if found == 0 {
        println!("No DDS files found matching 'light' and 'spot'");
        println!("\nSearching for any light*.dds files:");
        let mut count = 0;
        for entry in p4k.entries() {
            let path_str = entry.name.as_str();
            if (path_str.to_lowercase().contains("light")
                || path_str.to_lowercase().contains("gobo"))
                && path_str.ends_with(".dds")
            {
                println!("  {}", path_str);
                count += 1;
                if count > 10 {
                    break;
                }
            }
        }
    }
}
