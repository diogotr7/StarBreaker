use std::path::PathBuf;

use clap::Subcommand;

use crate::common::load_dcb_bytes;
use crate::error::{CliError, Result};

/// What a character export wears and plays (face and/or body clip; with
/// both, one clip named after the body clip).
#[derive(clap::Args, Debug, Clone)]
pub struct CharacterArgs {
    /// Dress the character in a loadout: an item-port loadout XML inside
    /// Data.p4k (e.g. `Data\Scripts\Loadouts\Character\Human\Armor\NPC\UEE\marine_Armor_Heavy.xml`).
    /// Zones the garments cover (the body under an undersuit) are left out.
    #[arg(long)]
    loadout: Option<String>,
    /// Bake a face animation: a `.caf` face clip inside Data.p4k
    /// (e.g. `Data\Animations\...\ai_civ_happy_idle_02_face.caf`).
    #[arg(long)]
    face_anim: Option<String>,
    /// Weight of the face animation (1.0 = as authored).
    #[arg(long, default_value = "1.0")]
    face_anim_intensity: f32,
    /// Bake a body animation: a `.caf` or `.dba` inside Data.p4k
    /// (e.g. `Data\Animations\Characters\Human\male_v7\character_select_screen.dba`).
    #[arg(long)]
    body_anim: Option<String>,
    /// The clip of a `--body-anim` `.dba` to play: its file stem
    /// (e.g. `char_customizer_stand_idle_01`) or a unique part of its path.
    #[arg(long, requires = "body_anim")]
    body_anim_clip: Option<String>,
}

/// A p4k file by a path written with either slash.
pub(crate) fn read_p4k(p4k: &starbreaker_p4k::MappedP4k, path: &str) -> std::result::Result<Vec<u8>, starbreaker_p4k::P4kError> {
    p4k.read_file(&path.replace('/', "\\"))
}

/// The loadout of `args`, read from the p4k (empty without one).
pub(crate) fn load_loadout(p4k: &starbreaker_p4k::MappedP4k, args: &CharacterArgs) -> Result<Vec<starbreaker_3d::character::loadout::LoadoutEntry>> {
    let Some(path) = args.loadout.as_deref() else { return Ok(Vec::new()) };
    let bytes = read_p4k(p4k, path).map_err(|e| CliError::InvalidInput(format!("loadout {path}: {e}")))?;
    starbreaker_3d::character::loadout::parse_loadout(&bytes).map_err(|e| CliError::InvalidInput(format!("{path}: {e}")))
}

/// The animation files of `args`, read from the p4k: the face `.caf` bytes
/// and the selected body clip.
pub(crate) fn load_animation(
    p4k: &starbreaker_p4k::MappedP4k,
    args: &CharacterArgs,
) -> Result<(Option<Vec<u8>>, Option<starbreaker_3d::animation::AnimationClip>)> {
    let read = |path: &str, what: &str| read_p4k(p4k, path).map_err(|e| CliError::InvalidInput(format!("{what} {path}: {e}")));
    let caf = args.face_anim.as_deref().map(|path| read(path, "face animation")).transpose()?;
    let body = match args.body_anim.as_deref() {
        Some(path) => Some(
            starbreaker_3d::character::clips::load_body_clip(&read(path, "body animation")?, path, args.body_anim_clip.as_deref())
                .map_err(|e| CliError::InvalidInput(e.to_string()))?,
        ),
        None => None,
    };
    Ok((caf, body))
}

#[derive(Subcommand)]
pub enum ChfCommand {
    /// Convert a .chf file to JSON
    ToJson {
        /// Input .chf file
        input: PathBuf,
        /// Output .json file [default: <input>.json]
        output: Option<PathBuf>,
    },
    /// Convert a JSON file back to .chf
    FromJson {
        /// Input .json file
        input: PathBuf,
        /// Output .chf file [default: <input>.chf]
        output: Option<PathBuf>,
    },
    /// Export a .chf character as a decomposed package for the Blender addon.
    ///
    /// Writes `Packages/<name>/scene.json` + `Data/...` (`.blend` parts,
    /// material sidecars, PNG textures) under OUTPUT: the DNA-shaped head with
    /// the CHF-selected HumanSkin_V2 material, eyes/teeth/eye-details and the
    /// CHF's hair/brow/lash/beard/scalp items and body, on one skeleton (the
    /// player actor's). The root instance carries the
    /// resolved CHF customization (skin tone, blemishes, makeup, colours).
    ToPackage {
        /// Input .chf file
        input: PathBuf,
        /// Export root directory
        output: PathBuf,
        /// Path to Data.p4k [default: auto-discover via SC_DATA_P4K]
        #[arg(long)]
        p4k: Option<PathBuf>,
        /// Texture mip to export (0 = full resolution)
        #[arg(long, default_value = "0")]
        texture_mip: u32,
        #[command(flatten)]
        animation: CharacterArgs,
    },
    /// Export a .chf character as a rigged glTF binary.
    ///
    /// The DNA-shaped head with the CHF's head items and body on the player
    /// actor's skeleton — the same rig as `to-package` — at rest, or playing
    /// the face and/or body clip, dressed in an optional loadout.
    ToGlb {
        /// Input .chf file
        input: PathBuf,
        /// Output .glb file [default: <input>.glb]
        output: Option<PathBuf>,
        /// Path to Data.p4k [default: auto-discover via SC_DATA_P4K]
        #[arg(long)]
        p4k: Option<PathBuf>,
        #[command(flatten)]
        animation: CharacterArgs,
    },
}

impl ChfCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::ToJson { input, output } => to_json(input, output),
            Self::FromJson { input, output } => from_json(input, output),
            Self::ToPackage { input, output, p4k, texture_mip, animation } => to_package(input, output, p4k, texture_mip, animation),
            Self::ToGlb { input, output, p4k, animation } => to_glb(input, output, p4k, animation),
        }
    }
}

fn to_json(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let json = starbreaker_chf::chf_to_json(&data)?;
    let output = output.unwrap_or_else(|| input.with_extension("json"));
    std::fs::write(&output, json.as_bytes())
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn from_json(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let json = std::fs::read_to_string(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let chf_bytes = starbreaker_chf::json_to_chf(&json)?;
    let output = output.unwrap_or_else(|| input.with_extension("chf"));
    std::fs::write(&output, &chf_bytes)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn to_package(
    input: PathBuf,
    output: PathBuf,
    p4k_path: Option<PathBuf>,
    texture_mip: u32,
    animation: CharacterArgs,
) -> Result<()> {
    let chf_bytes = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let p4k = p4k.ok_or_else(|| CliError::InvalidInput("Data.p4k is required".into()))?;
    let db = starbreaker_datacore::Database::from_bytes(&dcb_bytes)?;
    let name = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "character".into());
    let opts = starbreaker_3d::ExportOptions {
        kind: starbreaker_3d::ExportKind::Decomposed,
        format: starbreaker_3d::ExportFormat::Blend,
        texture_mip,
        decomposed_package_subdir: None,
        ..starbreaker_3d::ExportOptions::default()
    };
    let (caf, body) = load_animation(&p4k, &animation)?;
    let clips = character_animation(&animation, caf.as_deref(), body.as_ref());
    let loadout = load_loadout(&p4k, &animation)?;
    let export = starbreaker_3d::character::package::export_character_package(&p4k, &db, &chf_bytes, &name, &opts, &clips, &loadout)
        .map_err(|e| CliError::InvalidInput(format!("chf package export failed: {e}")))?;
    for file in &export.files {
        let path = output.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CliError::IoPath { source: e, path: parent.display().to_string() })?;
        }
        std::fs::write(&path, &file.bytes)
            .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
    }
    let scene = export.files.iter().find(|f| f.relative_path.ends_with("/scene.json")).map(|f| f.relative_path.as_str()).unwrap_or("?");
    eprintln!("Wrote {} files under {} (scene: {scene})", export.files.len(), output.display());
    Ok(())
}

/// The library's view of `args` over the loaded files.
pub(crate) fn character_animation<'a>(
    args: &CharacterArgs,
    caf: Option<&'a [u8]>,
    body: Option<&'a starbreaker_3d::animation::AnimationClip>,
) -> starbreaker_3d::character::clips::CharacterAnimation<'a> {
    use starbreaker_3d::character::clips::{CharacterAnimation, FaceAnimation};
    CharacterAnimation { face: caf.map(|caf| FaceAnimation { caf, intensity: args.face_anim_intensity }), body }
}

fn to_glb(input: PathBuf, output: Option<PathBuf>, p4k_path: Option<PathBuf>, animation: CharacterArgs) -> Result<()> {
    let chf_bytes = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let p4k = p4k.ok_or_else(|| CliError::InvalidInput("Data.p4k is required".into()))?;
    let db = starbreaker_datacore::Database::from_bytes(&dcb_bytes)?;
    let (caf, body) = load_animation(&p4k, &animation)?;
    let clips = character_animation(&animation, caf.as_deref(), body.as_ref());
    let loadout = load_loadout(&p4k, &animation)?;
    let glb = starbreaker_3d::character::chf_to_character_glb(&chf_bytes, &p4k, &db, &clips, &loadout)
        .map_err(|e| CliError::InvalidInput(format!("chf character GLB failed: {e}")))?;
    let output = output.unwrap_or_else(|| input.with_extension("glb"));
    std::fs::write(&output, &glb).map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written {} bytes to {}", glb.len(), output.display());
    Ok(())
}

