//! Embed a location-independent source digest, without clocks, Git state or executable mtimes.
#[path = "build_support/identity.rs"]
mod identity;
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Directory watching also detects added/deleted source files. Test fixtures
    // outside src and build products are not exporter inputs.
    println!("cargo:rerun-if-changed={}", root.join("crates").display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
    println!(
        "cargo:rustc-env=STARBREAKER_SOURCE_SHA256={}",
        identity::source_digest(&root).expect("hash exporter sources")
    );
}
