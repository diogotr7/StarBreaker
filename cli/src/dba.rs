//! DBA / animation-database inspection CLI.
//!
//! Provides the `dump` subcommand that reads a `.dba` (or `.caf`) animation
//! file and prints, for each clip, the metadata name, channel count, and
//! per-channel bone hash + first/last keyframe. With `--skeleton <path>` the
//! tool resolves bone hashes to bone names for human-readable output.
//!
//! Useful for diagnosing animation mismatches such as the wings_deploy
//! diagonal-pairing issue documented in
//! `docs/StarBreaker/todo.md` (Phase 23A residual).

use std::path::PathBuf;

use clap::Subcommand;

use starbreaker_3d::animation::{
    AnimationDatabase, bone_name_hash, parse_caf, parse_dba,
};
use starbreaker_3d::skeleton::parse_skeleton;

use crate::common::load_p4k;
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum DbaCommand {
    /// Dump clips, channels, and first/last keyframes from a DBA or CAF file.
    Dump {
        /// Path to a `.dba` or `.caf` file. If the path begins with `Data/`
        /// or matches a P4k-internal path, the file is read from the P4k
        /// instead (requires `--p4k` or `SC_DATA_P4K`).
        path: String,
        /// Optional `.chr`/`.skin` skeleton path used to resolve bone
        /// hashes to names. Same path resolution rules as `path`.
        #[arg(long)]
        skeleton: Option<String>,
        /// Path to Data.p4k (only required when reading P4k-internal paths).
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Show every keyframe per channel, not just first/last.
        #[arg(long)]
        all_keyframes: bool,
        /// Filter clips by case-insensitive substring match on the clip
        /// name (DBA metadata name).
        #[arg(long)]
        filter: Option<String>,
    },
}

impl DbaCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Dump { path, skeleton, p4k, all_keyframes, filter } => {
                dump(path, skeleton, p4k, all_keyframes, filter)
            }
        }
    }
}

fn read_input(path: &str, p4k_path: Option<&PathBuf>) -> Result<Vec<u8>> {
    let direct = std::path::Path::new(path);
    if direct.is_file() {
        return std::fs::read(direct).map_err(|e| CliError::IoPath {
            source: e,
            path: direct.display().to_string(),
        });
    }
    let p4k = load_p4k(p4k_path.map(|p| p.as_path()))
        .map_err(|e| CliError::InvalidInput(format!("p4k open failed: {e}")))?;
    let entry = p4k.entry_case_insensitive(path).ok_or_else(|| {
        CliError::NotFound(format!("'{path}' not found in P4k"))
    })?;
    p4k.read(entry).map_err(|e| CliError::InvalidInput(format!("P4k read error: {e}")))
}

fn dump(
    path: String,
    skeleton: Option<String>,
    p4k_path: Option<PathBuf>,
    all_keyframes: bool,
    filter: Option<String>,
) -> Result<()> {
    let bytes = read_input(&path, p4k_path.as_ref())?;
    let db: AnimationDatabase = if path.to_ascii_lowercase().ends_with(".caf") {
        parse_caf(&bytes).map_err(|e| CliError::InvalidInput(format!("parse_caf: {e}")))?
    } else {
        parse_dba(&bytes).map_err(|e| CliError::InvalidInput(format!("parse_dba: {e}")))?
    };

    // Optional skeleton-based hash resolution.
    let mut hash_to_name: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    if let Some(skel_path) = skeleton.as_ref() {
        let skel_bytes = read_input(skel_path, p4k_path.as_ref())?;
        if let Some(bones) = parse_skeleton(&skel_bytes) {
            for bone in &bones {
                hash_to_name.insert(bone_name_hash(&bone.name), bone.name.clone());
            }
        } else {
            eprintln!("warning: failed to parse skeleton '{skel_path}'");
        }
    }

    let filter_lc = filter.map(|f| f.to_ascii_lowercase());

    println!("file: {path}");
    println!("clip count: {}", db.clips.len());
    if !hash_to_name.is_empty() {
        println!("skeleton: {} bones loaded for hash resolution", hash_to_name.len());
    }
    println!();

    for (idx, clip) in db.clips.iter().enumerate() {
        if let Some(needle) = filter_lc.as_ref() {
            if !clip.name.to_ascii_lowercase().contains(needle) {
                continue;
            }
        }
        let frame_count = clip
            .channels
            .iter()
            .map(|ch| ch.rotations.len().max(ch.positions.len()))
            .max()
            .unwrap_or(0);
        println!(
            "[{idx:>3}] {name} (fps={fps}, channels={ch}, frames~{frames})",
            idx = idx,
            name = clip.name,
            fps = clip.fps,
            ch = clip.channels.len(),
            frames = frame_count,
        );
        for ch in &clip.channels {
            let name = hash_to_name
                .get(&ch.bone_hash)
                .cloned()
                .unwrap_or_else(|| String::from("?"));
            println!(
                "    bone 0x{hash:08X} ({name}): rot={rot} pos={pos}",
                hash = ch.bone_hash,
                name = name,
                rot = ch.rotations.len(),
                pos = ch.positions.len(),
            );
            if all_keyframes {
                for (i, kf) in ch.rotations.iter().enumerate() {
                    let [a, b, c, d] = kf.value;
                    println!(
                        "      rot[{i}] t={t:.2}: ({a:+.4}, {b:+.4}, {c:+.4}, {d:+.4})",
                        t = kf.time,
                    );
                }
                for (i, kf) in ch.positions.iter().enumerate() {
                    let [a, b, c] = kf.value;
                    println!(
                        "      pos[{i}] t={t:.2}: ({a:+.4}, {b:+.4}, {c:+.4})",
                        t = kf.time,
                    );
                }
            } else {
                if let (Some(first), Some(last)) =
                    (ch.rotations.first(), ch.rotations.last())
                {
                    let [a, b, c, d] = first.value;
                    let [e, f, g, h] = last.value;
                    println!(
                        "      rot first=({a:+.4}, {b:+.4}, {c:+.4}, {d:+.4})  last=({e:+.4}, {f:+.4}, {g:+.4}, {h:+.4})",
                    );
                }
                if let (Some(first), Some(last)) =
                    (ch.positions.first(), ch.positions.last())
                {
                    let [a, b, c] = first.value;
                    let [e, f, g] = last.value;
                    println!(
                        "      pos first=({a:+.4}, {b:+.4}, {c:+.4})  last=({e:+.4}, {f:+.4}, {g:+.4})",
                    );
                }
            }
        }
        println!();
    }

    Ok(())
}
