fn main() {
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    for e in p4k.entries() {
        if e.name.to_lowercase().contains(".dcb") {
            println!(
                "{} ({} bytes / {} compressed)",
                e.name, e.uncompressed_size, e.compressed_size
            );
        }
    }
}
