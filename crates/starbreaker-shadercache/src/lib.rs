//! Access to Star Citizen's Vulkan shader cache as shipped in `Data.p4k`.
//!
//! The crate ships no game data: no shader names, masks, sources or extracted
//! blobs. Everything is read from the user's own installation at runtime,
//! lazily, through [`ShaderPak`]. The one format table it carries is the shader
//! keyword grammar for the latest known shader cache version ([`grammar`]);
//! shader bins of any other version are refused, never guessed at.
//!
//! ```no_run
//! use starbreaker_shadercache::ShaderPak;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pak = ShaderPak::open_in_p4k(r"C:\Games\StarCitizen\LIVE\Data.p4k")?;
//! println!("{} permutations", pak.len());
//! for perm in pak.family("HardSurface").take(3) {
//!     let r = pak.reflect(perm)?;
//!     println!("{}@{} -> sets {:?}", perm.shader, perm.entry, r.sets());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Module map:
//! - [`shcb`] — the `SHCB` container around a SPIR-V payload.
//! - [`name`] — permutation names (`Shader@Entry(GL…)(RT…)(MDV…).shcb`).
//! - [`reflect`] — entry points and decorated descriptor bindings via `rspirv`.
//! - [`pak`] — lazy index and reader for `Engine\ShaderCache_Vulkan.pak`, using
//!   the ZIP reader from `starbreaker-p4k`.
//! - [`cfxb`] — the `FXB1` shader-bin container (token stream + string table).
//! - [`grammar`] — the shipped keyword grammar, keyed by cache version.
//! - [`hlsl`] — token stream → HLSL text; [`hlsl::decode`] is the safe entry point.
//! - [`genmask`] — `.ext` flag → generation-mask bits.
//! - [`defaults`] — public parameter defaults from decoded shader source.

pub mod cfxb;
pub mod defaults;
pub mod error;
pub mod genmask;
pub mod grammar;
pub mod hlsl;
pub mod name;
pub mod pak;
pub mod reflect;
pub mod shcb;

pub use cfxb::ShaderBin;
pub use error::ShaderCacheError;
pub use genmask::{GenMask, GenMasks};
pub use grammar::{Grammar, SUPPORTED_CACHE_VERSION};
pub use name::{Masks, Permutation, Stage};
pub use pak::{EXT_PAK, ShaderPak, VULKAN_PAK, read_pak_entries};
pub use reflect::{Binding, EntryPoint, Member, Reflection, ResourceClass, StageVar, reflect};
pub use shcb::{SHCB_HEADER_LEN, SPIRV_MAGIC, Shcb};





