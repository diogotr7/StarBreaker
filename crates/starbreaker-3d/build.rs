//! Embed a location-independent source digest, without clocks, Git state or executable mtimes.
#[path = "build_support/identity.rs"]
mod identity;
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Watch only the exporter's inputs (identity::watched_paths): directory
    // watching also detects added/deleted source files, while other crates
    // and every crate's tests and fixtures do not rerun this script.
    for path in identity::watched_paths(&root).expect("list exporter sources") {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!(
        "cargo:rustc-env=STARBREAKER_SOURCE_SHA256={}",
        identity::source_digest(&root).expect("hash exporter sources")
    );
}
