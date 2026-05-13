fn main() {
    // Derive a build version for the bundled Blender addon from the workspace
    // Cargo version and the current git commit count.  This means every new
    // commit produces a unique ADDON_BUILD_VERSION, so users are offered an
    // update automatically without requiring a manual VERSION bump in the
    // Python __init__.py.
    //
    // Format: "<cargo_pkg_version>+addon.<git_commit_count>"
    // e.g.    "0.2.2+addon.147"
    let git_count = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "0".to_string());

    let cargo_version = env!("CARGO_PKG_VERSION");
    let addon_build_version = format!("{cargo_version}+addon.{git_count}");
    println!("cargo:rustc-env=ADDON_BUILD_VERSION={addon_build_version}");

    // Embed commit SHA if available, otherwise derive from git
    let commit_sha = if let Ok(commit_sha) = std::env::var("COMMIT_SHA") {
        // CI environment - use provided SHA
        commit_sha[..7].to_string()
    } else {
        // Local development - get SHA from git and check for dirty state
        let git_sha = std::process::Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Check if working tree is dirty
        let is_dirty = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        if is_dirty && git_sha != "unknown" {
            format!("{}-dirty", git_sha)
        } else {
            git_sha
        }
    };

    println!("cargo:rustc-env=COMMIT_SHA={}", commit_sha);

    // Re-run this build script whenever the git HEAD or refs change.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");

    tauri_build::build();
}
