//! Tauri commands for the Character Builder: live head GLB preview,
//! `.chf` export, and `.chf` import. Bridges the frontend `DnaInput` DTO to
//! `starbreaker_chf::Dna` and reuses a per-gender cached
//! `starbreaker_3d::character::HeadIdentityContext` from app state.
//!
//! Commands: [`dna_preview_glb`] (live head GLB), and the disk-IO variants
//! [`dna_export_chf_to`] / [`dna_export_glb_to`] / [`dna_import_chf`], which
//! take/return a filesystem path and perform the read/write in Rust, matching
//! the app's existing dialog-then-Rust-IO convention (see `extract_p4k_file`).
//! This avoids a `@tauri-apps/plugin-fs` dependency. [`context_for`] lazily
//! builds + caches a per-gender `HeadIdentityContext` in
//! [`crate::state::AppState`].

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use starbreaker_3d::character::{identity, HeadIdentityContext};
use starbreaker_chf::{Dna, DnaBlend, FacePart};
use starbreaker_common::NameHash;
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// The 13 face parts in `FacePart`-enum order. Index = `FacePart as usize`.
const FACE_PART_ORDER: [FacePart; 13] = [
    FacePart::EyebrowLeft,
    FacePart::EyebrowRight,
    FacePart::EyeLeft,
    FacePart::EyeRight,
    FacePart::Nose,
    FacePart::EarLeft,
    FacePart::EarRight,
    FacePart::CheekLeft,
    FacePart::CheekRight,
    FacePart::Mouth,
    FacePart::Jaw,
    FacePart::Crown,
    FacePart::Neck,
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
}

impl Gender {
    pub(crate) fn gender_hash(self) -> NameHash {
        match self {
            Gender::Male => identity::MALE_T1_PU_GENDER_HASH,
            Gender::Female => identity::FEMALE_T1_PU_GENDER_HASH,
        }
    }
    fn from_gender_hash(h: NameHash) -> Self {
        if h == identity::FEMALE_T1_PU_GENDER_HASH {
            Gender::Female
        } else {
            Gender::Male
        }
    }

    fn head_folder(self) -> &'static str {
        match self {
            Gender::Male => "male",
            Gender::Female => "female",
        }
    }
}

/// One idle-emotion set for a gender: the emotion name and its sorted,
/// deduped variation indices (the `<NN>` in the `.caf` filename).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionSet {
    pub emotion: String,
    pub variations: Vec<u32>,
}

/// Enumerate idle face-anims for `gender` from the p4k by path convention.
/// Grouped by emotion (sorted), variations sorted ascending + deduped.
fn enumerate_expressions(
    p4k: &starbreaker_p4k::MappedP4k,
    gender: Gender,
) -> Vec<ExpressionSet> {
    use std::collections::{BTreeMap, BTreeSet};
    let prefix = format!(
        r"Data\Animations\Characters\Human\heads\{}\ai\idlesets\",
        gender.head_folder()
    );
    let lprefix = prefix.to_lowercase();
    let mut by_emotion: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for e in p4k.entries() {
        let name = &e.name;
        let lname = name.to_lowercase();
        if !lname.starts_with(&lprefix)
            || !lname.ends_with("_face.caf")
            || !lname.contains(r"\face\")
        {
            continue;
        }
        // Slice the original (case-preserving) name by the prefix BYTE length.
        // We matched the lowercased prefix of the same length, so the boundary
        // is valid for ASCII paths; `get` guards against a non-ASCII char
        // boundary panic just in case.
        let Some(rest) = name.get(prefix.len()..) else {
            continue;
        };
        let emotion = match rest.split('\\').next() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let file = name.rsplit('\\').next().unwrap_or("");
        let Some(idx) = file
            .rsplit_once("_idle_")
            .and_then(|(_, tail)| tail.split('_').next())
            .and_then(|d| d.parse::<u32>().ok())
        else {
            continue;
        };
        by_emotion.entry(emotion).or_default().insert(idx);
    }
    by_emotion
        .into_iter()
        .map(|(emotion, vars)| ExpressionSet {
            emotion,
            variations: vars.into_iter().collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlendInput {
    pub value: u16,
    pub head_id: u16,
}

/// Frontend DNA: gender + 13 face parts (ordered), each 4 blends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnaInput {
    pub gender: Gender,
    /// Length 13, indexed by `FacePart as usize`.
    pub face_parts: Vec<[BlendInput; 4]>,
}

impl DnaInput {
    /// Build a `starbreaker_chf::Dna` sufficient for the GLB pipeline.
    /// `raw_bytes`/`variant_hash` are not used by `compute_chf_weights`, so a
    /// minimal-but-valid struct is fine for preview.
    pub(crate) fn to_chf_dna(&self) -> Dna {
        let mut face_parts: BTreeMap<FacePart, [DnaBlend; 4]> = BTreeMap::new();
        let mut max_head_id = 0u16;
        for (i, blends) in self.face_parts.iter().enumerate() {
            let part = FACE_PART_ORDER[i];
            let arr = blends.map(|b| {
                max_head_id = max_head_id.max(b.head_id);
                DnaBlend {
                    value: b.value,
                    head_id: b.head_id,
                }
            });
            face_parts.insert(part, arr);
        }
        let part_count = self.face_parts.len() as u16;
        let mut dna = Dna {
            raw_bytes: Vec::new(),
            gender_hash: self.gender.gender_hash(),
            variant_hash: NameHash(0),
            part_count,
            blends_per_part: 4,
            header_unknown: 0,
            max_head_id,
            face_parts,
        };
        dna.raw_bytes = dna.to_raw_bytes();
        dna
    }

    pub(crate) fn from_chf_dna(dna: &Dna) -> Self {
        let face_parts = FACE_PART_ORDER
            .iter()
            .take(dna.part_count as usize)
            .map(|part| {
                let b = dna
                    .face_parts
                    .get(part)
                    .copied()
                    .unwrap_or([DnaBlend { value: 0, head_id: 0 }; 4]);
                b.map(|x| BlendInput {
                    value: x.value,
                    head_id: x.head_id,
                })
            })
            .collect();
        DnaInput {
            gender: Gender::from_gender_hash(dna.gender_hash),
            face_parts,
        }
    }
}

/// Get (or lazily build + cache) the `HeadIdentityContext` for `gender`.
fn context_for(
    state: &State<'_, AppState>,
    gender: Gender,
) -> Result<Arc<HeadIdentityContext>, AppError> {
    let gender_hash = gender.gender_hash();
    if let Some(ctx) = state.head_contexts.lock().get(&gender_hash.value()) {
        return Ok(ctx.clone());
    }
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();
    let ctx = Arc::new(HeadIdentityContext::load(&p4k, gender_hash)?);
    state
        .head_contexts
        .lock()
        .insert(gender_hash.value(), ctx.clone());
    Ok(ctx)
}

/// Build a head GLB preview for the given DNA. `full` includes eyes/teeth.
#[tauri::command]
pub fn dna_preview_glb(
    state: State<'_, AppState>,
    dna: DnaInput,
    full: bool,
) -> Result<tauri::ipc::Response, AppError> {
    let ctx = context_for(&state, dna.gender)?;
    let chf_dna = dna.to_chf_dna();
    // Preview is a STATIC neutral head: skip the ~400 expression morph targets
    // (tens of MB) — they bloat the GLB and three.js parse for no visual gain.
    // Return raw bytes via `ipc::Response` so the GLB crosses the IPC boundary
    // as an ArrayBuffer, NOT a JSON `number[]` (which would ~3.5× the payload
    // and force a full JSON encode/decode on every drag).
    let glb = ctx.build_glb(&chf_dna, full, false)?;
    Ok(tauri::ipc::Response::new(glb))
}

/// List the idle facial-expression sets (emotion + variation indices)
/// available for `gender`, enumerated from the p4k by path convention.
#[tauri::command]
pub fn dna_list_expressions(
    state: State<'_, AppState>,
    gender: Gender,
) -> Result<Vec<ExpressionSet>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();
    Ok(enumerate_expressions(&p4k, gender))
}

/// Build a looping facial-expression animation GLB for `dna`, driven by the
/// `<emotion>`/`<variation>` idle `.caf` at `intensity` (clip weight). Returns
/// raw GLB bytes via `ipc::Response` (same ArrayBuffer path as the static
/// preview — avoids a JSON `number[]` round-trip).
#[tauri::command]
pub fn dna_expression_anim_glb(
    state: State<'_, AppState>,
    dna: DnaInput,
    emotion: String,
    variation: u32,
    intensity: f32,
) -> Result<tauri::ipc::Response, AppError> {
    let ctx = context_for(&state, dna.gender)?;
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();
    let path = format!(
        r"Data\Animations\Characters\Human\heads\{}\ai\idlesets\{}\face\ai_civ_{}_idle_{:02}_face.caf",
        dna.gender.head_folder(),
        emotion,
        emotion,
        variation,
    );
    let caf_bytes = p4k
        .read_file(&path)
        .map_err(|e| AppError::Internal(format!("read {path}: {e}")))?;
    let chf_dna = dna.to_chf_dna();
    let glb = ctx.build_expression_glb(&chf_dna, &caf_bytes, intensity)?;
    Ok(tauri::ipc::Response::new(glb))
}

/// Serialize the DNA into a 4096-byte `.chf` by editing the gender's stock
/// default CHF container. Returns the CHF bytes.
fn build_chf_bytes(state: &State<'_, AppState>, dna: &DnaInput) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();
    let base_path = match dna.gender {
        Gender::Male => r"Data\Libs\CharacterCustomizer\masculinedefault.chf",
        Gender::Female => r"Data\Libs\CharacterCustomizer\femininedefault.chf",
    };
    let base_bytes = p4k.read_file(base_path)?;
    let base = starbreaker_chf::ChfFile::from_chf(&base_bytes)
        .map_err(|e| AppError::Internal(format!("parse base chf: {e}")))?;
    let mut data = base
        .parse()
        .map_err(|e| AppError::Internal(format!("parse base chf data: {e}")))?;

    let edited = dna.to_chf_dna();
    data.dna.face_parts = edited.face_parts;
    data.dna.max_head_id = edited.max_head_id;
    data.dna.part_count = edited.part_count;

    let chf = starbreaker_chf::ChfFile {
        data: data.write(),
        unknown: base.unknown,
        is_modded: true,
    };
    chf.to_chf()
        .map_err(|e| AppError::Internal(format!("write chf: {e}")))
}

/// Write `bytes` to `path`, creating parent directories as needed.
/// Mirrors the IO pattern in `commands::extract_p4k_file`.
fn write_to_path(path: &str, bytes: &[u8]) -> Result<(), AppError> {
    let out = std::path::Path::new(path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, bytes)?;
    Ok(())
}

/// Build the DNA's `.chf` and write it to `path` (chosen via the save dialog
/// on the frontend). Mirrors `extract_p4k_file`'s path-in / Rust-IO contract.
#[tauri::command]
pub fn dna_export_chf_to(
    state: State<'_, AppState>,
    dna: DnaInput,
    path: String,
) -> Result<(), AppError> {
    let bytes = build_chf_bytes(&state, &dna)?;
    write_to_path(&path, &bytes)
}

/// Build the head-preview GLB and write it to `path` (chosen via the save
/// dialog on the frontend). `full` includes eyes/teeth.
#[tauri::command]
pub fn dna_export_glb_to(
    state: State<'_, AppState>,
    dna: DnaInput,
    full: bool,
    path: String,
) -> Result<(), AppError> {
    let ctx = context_for(&state, dna.gender)?;
    let chf_dna = dna.to_chf_dna();
    // Exported asset keeps the expression morph targets (for posing in Blender).
    let bytes = ctx.build_glb(&chf_dna, full, true)?;
    write_to_path(&path, &bytes)
}

/// Read a `.chf` from `path` (chosen via the open dialog on the frontend) and
/// parse its DNA into the editor DTO.
#[tauri::command]
pub fn dna_import_chf(path: String) -> Result<DnaInput, AppError> {
    let bytes = std::fs::read(&path)?;
    let chf = starbreaker_chf::ChfFile::from_chf(&bytes)
        .map_err(|e| AppError::Internal(format!("parse chf: {e}")))?;
    let data = chf
        .parse()
        .map_err(|e| AppError::Internal(format!("parse chf data: {e}")))?;
    Ok(DnaInput::from_chf_dna(&data.dna))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DnaInput {
        DnaInput {
            gender: Gender::Female,
            face_parts: (0..13)
                .map(|i| {
                    [
                        BlendInput {
                            value: 65534,
                            head_id: (i % 5) as u16 + 1,
                        },
                        BlendInput { value: 0, head_id: 0 },
                        BlendInput { value: 0, head_id: 0 },
                        BlendInput { value: 0, head_id: 0 },
                    ]
                })
                .collect(),
        }
    }

    /// Open Data.p4k the same way `identity_context.rs` tests do: read
    /// `SC_DATA_P4K` (the established open-or-skip mechanism), falling back to
    /// the default LIVE install path. Returns `None` with an eprintln skip note
    /// if the archive can't be opened.
    fn open_p4k_or_skip() -> Option<starbreaker_p4k::MappedP4k> {
        const DEFAULT_P4K: &str =
            r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";
        let path = std::env::var("SC_DATA_P4K").unwrap_or_else(|_| DEFAULT_P4K.to_string());
        if !std::path::Path::new(&path).exists() {
            eprintln!("skip: Data.p4k not found at {path}");
            return None;
        }
        match starbreaker_p4k::MappedP4k::open(&path) {
            Ok(p4k) => Some(p4k),
            Err(e) => {
                eprintln!("skip: failed to open Data.p4k at {path}: {e}");
                None
            }
        }
    }

    #[test]
    fn list_expressions_male_has_emotions_with_variations() {
        let Some(p4k) = open_p4k_or_skip() else {
            return;
        };
        let sets = enumerate_expressions(&p4k, Gender::Male);
        assert!(!sets.is_empty(), "male should have idle emotions");
        assert!(sets.iter().all(|s| !s.variations.is_empty()));
        assert!(sets.iter().any(|s| s.emotion == "happy"));
        let happy = sets.iter().find(|s| s.emotion == "happy").unwrap();
        assert!(happy.variations.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn dna_input_round_trips_through_chf_dna() {
        let input = sample();
        let dna = input.to_chf_dna();
        let back = DnaInput::from_chf_dna(&dna);
        assert_eq!(back.gender, input.gender);
        assert_eq!(back.face_parts.len(), 13);
        for (a, b) in back.face_parts.iter().zip(&input.face_parts) {
            for (x, y) in a.iter().zip(b) {
                assert_eq!(x.value, y.value);
                assert_eq!(x.head_id, y.head_id);
            }
        }
        assert_eq!(dna.gender_hash, identity::FEMALE_T1_PU_GENDER_HASH);
        assert_eq!(dna.max_head_id, 5);
    }
}
