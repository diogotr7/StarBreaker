use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};

use starbreaker_wem::WemFile;
use starbreaker_wwise::{BnkFile, Hierarchy};

use crate::common::load_p4k;
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum WwiseCommand {
    /// Print soundbank metadata (version, sections, WEM count)
    Info {
        /// Input .bnk file path
        input: String,
        /// Path to Data.p4k (for P4k paths)
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// List all embedded WEM entries in a soundbank
    List {
        /// Input .bnk file path
        input: String,
        /// Path to Data.p4k (for P4k paths)
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Extract WEM files from a soundbank
    Extract {
        /// Input .bnk file path
        input: String,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Decode to Ogg (Vorbis/Opus) instead of raw WEM
        #[arg(long)]
        decode: bool,
        /// Path to Data.p4k (for P4k paths)
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// List all events in a soundbank
    Events {
        input: String,
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Trace an event to its leaf sounds
    Trace {
        input: String,
        /// Event name or numeric ID (e.g., "Play_weapon_fire" or "0xA3B2C1D0" or "12345")
        #[arg(long)]
        event: String,
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Dump full HIRC hierarchy as JSON
    Dump {
        input: String,
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Search for audio by trigger name or entity
    Search {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Search by trigger name (ATL -> bank -> HIRC -> sounds)
        #[arg(long, group = "search_mode")]
        trigger: Option<String>,
        /// Search by DataCore entity name (entity -> triggers -> ATL -> banks -> sounds)
        #[arg(long, group = "search_mode")]
        entity: Option<String>,
    },
}

impl WwiseCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Info { input, p4k } => info(&input, p4k.as_deref()),
            Self::List { input, p4k } => list(&input, p4k.as_deref()),
            Self::Extract {
                input,
                output,
                decode,
                p4k,
            } => extract(&input, &output, decode, p4k.as_deref()),
            Self::Events { input, p4k } => events(&input, p4k.as_deref()),
            Self::Trace { input, event, p4k } => trace(&input, &event, p4k.as_deref()),
            Self::Dump { input, p4k } => dump(&input, p4k.as_deref()),
            Self::Search {
                p4k,
                trigger,
                entity,
            } => search(p4k.as_deref(), trigger, entity),
        }
    }
}

fn load_bnk_bytes(input: &str, p4k_path: Option<&Path>) -> Result<Vec<u8>> {
    let path = Path::new(input);
    if path.exists() {
        return Ok(fs::read(path)
            .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?);
    }
    // Try P4k path
    let p4k = load_p4k(p4k_path)?;
    Ok(p4k.read_file(input)?)
}

fn info(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;

    eprintln!("Bank version:  {}", bnk.header.version);
    eprintln!("Bank ID:       {}", bnk.header.bank_id);
    eprintln!(
        "Sections:      {}",
        bnk.section_tags()
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("Embedded WEMs: {}", bnk.wem_count());

    if let Some(ref hirc) = bnk.hirc {
        eprintln!("HIRC objects:  {}", hirc.entries.len());
        let counts = hirc.type_counts();
        let mut sorted: Vec<_> = counts.iter().collect();
        sorted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (type_id, count) in sorted {
            let name = starbreaker_wwise::HircObjectType::from_u8(*type_id)
                .map(|t| t.name().to_string())
                .unwrap_or_else(|| format!("Unknown({})", type_id));
            eprintln!("  {name:<30} {count}");
        }
    }

    if !bnk.string_ids.is_empty() {
        eprintln!("String IDs:    {}", bnk.string_ids.len());
        for (id, name) in &bnk.string_ids {
            eprintln!("  {id}: {name}");
        }
    }

    Ok(())
}

fn list(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;

    if bnk.data_index.is_empty() {
        eprintln!("No embedded WEM entries.");
        return Ok(());
    }

    eprintln!(
        "{:<12} {:<10} {:<10} {:<12} {}",
        "WEM ID", "Offset", "Size", "Codec", "Duration"
    );
    eprintln!("{}", "-".repeat(60));

    for entry in &bnk.data_index {
        // Copy packed fields to locals to avoid misaligned reference errors.
        let id = entry.id;
        let offset = entry.offset;
        let size = entry.size;

        let wem_data = bnk.wem_data(entry)?;
        let (codec_str, duration_str) = match WemFile::parse(wem_data) {
            Ok(wem) => {
                let dur = wem
                    .estimated_duration_secs()
                    .map(|d| format!("{d:.2}s"))
                    .unwrap_or_else(|| "?".into());
                (wem.codec_type().to_string(), dur)
            }
            Err(_) => ("(parse err)".into(), "?".into()),
        };

        eprintln!(
            "{:<12} {:<10} {:<10} {:<12} {}",
            id, offset, size, codec_str, duration_str
        );
    }

    Ok(())
}

fn extract(input: &str, output: &Path, decode: bool, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;

    if bnk.data_index.is_empty() {
        eprintln!("No embedded WEM entries to extract.");
        return Ok(());
    }

    fs::create_dir_all(output)?;

    let pb = ProgressBar::new(bnk.data_index.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len}")?,
    );

    for entry in &bnk.data_index {
        // Copy packed field to local to avoid misaligned reference errors.
        let id = entry.id;
        let wem_bytes = bnk.wem_data(entry)?;

        if decode {
            match decode_and_write(id, wem_bytes, output) {
                Ok(()) => {}
                Err(e) => eprintln!("Error decoding WEM {id}: {e}"),
            }
        } else {
            let path = output.join(format!("{id}.wem"));
            fs::write(&path, wem_bytes)?;
        }

        pb.inc(1);
    }

    pb.finish_and_clear();
    eprintln!(
        "Extracted {} files to {}",
        bnk.data_index.len(),
        output.display()
    );
    Ok(())
}

fn decode_and_write(id: u32, wem_bytes: &[u8], output: &Path) -> Result<()> {
    let wem = WemFile::parse(wem_bytes)?;

    match wem.codec_type() {
        starbreaker_wem::WemCodec::Vorbis => {
            let ogg = starbreaker_wem::decode::vorbis_to_ogg(wem_bytes)?;
            let path = output.join(format!("{id}.ogg"));
            fs::write(&path, ogg)?;
        }
        other => {
            // Unsupported codec — fall back to raw WEM
            eprintln!(
                "  WEM {id}: codec {other} not yet supported for decode, writing raw .wem"
            );
            let path = output.join(format!("{id}.wem"));
            fs::write(&path, wem_bytes)?;
        }
    }

    Ok(())
}

fn events(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;
    let hirc = bnk.hirc.as_ref().ok_or_else(|| CliError::NotFound("bank has no HIRC section".into()))?;
    let hierarchy = Hierarchy::from_section(hirc);

    let mut evts: Vec<_> = hierarchy.events().collect();
    evts.sort_by_key(|(id, _)| *id);

    for (id, event) in &evts {
        let resolved = hierarchy.resolve_event(*id);
        eprintln!(
            "Event {:#010x}  actions: {}  sounds: {}",
            id,
            event.action_ids.len(),
            resolved.len()
        );
    }

    eprintln!("\nTotal: {} events", evts.len());
    Ok(())
}

fn trace(input: &str, event_str: &str, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;
    let hirc = bnk.hirc.as_ref().ok_or_else(|| CliError::NotFound("bank has no HIRC section".into()))?;
    let hierarchy = Hierarchy::from_section(hirc);

    // Parse event identifier: hex (0x...), decimal, or name (FNV-1 hash)
    let event_id = if let Some(hex) = event_str.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)?
    } else if let Ok(num) = event_str.parse::<u32>() {
        num
    } else {
        let hash = starbreaker_wwise::fnv1_hash(event_str);
        eprintln!("Event \"{}\" -> hash {:#010x}", event_str, hash);
        hash
    };

    let results = hierarchy.resolve_event(event_id);
    if results.is_empty() {
        eprintln!("No sounds resolved for event {:#010x}", event_id);
        return Ok(());
    }

    for sound in &results {
        let path_str = sound.path.iter()
            .map(|id| {
                hierarchy.get(*id)
                    .map(|obj| format!("{}({:#010x})", obj.type_name(), id))
                    .unwrap_or_else(|| format!("?({:#010x})", id))
            })
            .collect::<Vec<_>>()
            .join(" -> ");
        let source_str = match sound.source {
            starbreaker_wwise::SoundSource::Embedded => "Embedded",
            starbreaker_wwise::SoundSource::PrefetchStream => "PrefetchStream",
            starbreaker_wwise::SoundSource::Stream => "Stream",
        };
        eprintln!("{} -> media {} [{}]", path_str, sound.media_id, source_str);
    }

    eprintln!("\nResolved {} sounds", results.len());
    Ok(())
}

fn dump(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let data = load_bnk_bytes(input, p4k_path)?;
    let bnk = BnkFile::parse(&data)?;
    let hirc = bnk.hirc.as_ref().ok_or_else(|| CliError::NotFound("bank has no HIRC section".into()))?;
    let hierarchy = Hierarchy::from_section(hirc);

    let mut objects: Vec<&starbreaker_wwise::HircObject> = Vec::new();
    for entry in &hirc.entries {
        if let Some(obj) = hierarchy.get(entry.object_id) {
            objects.push(obj);
        }
    }

    let json = serde_json::to_string_pretty(&objects)?;
    println!("{json}");
    Ok(())
}

fn search(
    p4k_path: Option<&Path>,
    trigger: Option<String>,
    entity: Option<String>,
) -> Result<()> {
    let p4k = load_p4k(p4k_path)?;
    if let Some(trigger_name) = trigger {
        return search_by_trigger(&p4k, &trigger_name);
    }
    if let Some(entity_name) = entity {
        return search_by_entity(&p4k, &entity_name);
    }
    return Err(CliError::InvalidInput("specify --trigger or --entity".into()));
}

fn search_by_trigger(p4k: &starbreaker_p4k::MappedP4k, trigger_name: &str) -> Result<()> {
    eprintln!("Building ATL index...");
    let atl = starbreaker_wwise::AtlIndex::from_p4k(p4k)?;
    eprintln!("ATL index: {} triggers", atl.len());

    let trigger = atl
        .get_trigger(trigger_name)
        .ok_or_else(|| CliError::NotFound(format!("trigger '{}' not found in ATL", trigger_name)))?;

    eprintln!(
        "Trigger: {} -> bank {} ({}, radius {:?})",
        trigger.trigger_name, trigger.bank_name, trigger.duration_type, trigger.radius_max
    );

    // Load bank and resolve
    let bank_path = format!("Data\\Sounds\\wwise\\{}", trigger.bank_name);
    let bank_data = p4k.read_file(&bank_path)?;
    let bnk = BnkFile::parse(&bank_data)?;
    let hirc = bnk.hirc.as_ref().ok_or_else(|| CliError::NotFound("bank has no HIRC".into()))?;
    let hierarchy = Hierarchy::from_section(hirc);

    let sounds = hierarchy.resolve_event_by_name(&trigger.wwise_event_name);
    for sound in &sounds {
        let source_str = match sound.source {
            starbreaker_wwise::SoundSource::Embedded => "Embedded",
            starbreaker_wwise::SoundSource::PrefetchStream => "PrefetchStream",
            starbreaker_wwise::SoundSource::Stream => "Stream",
        };
        eprintln!("  media {} [{}]", sound.media_id, source_str);
    }
    eprintln!(
        "\n{} -> {} sounds ({})",
        trigger_name,
        sounds.len(),
        trigger.bank_name
    );
    Ok(())
}

fn search_by_entity(p4k: &starbreaker_p4k::MappedP4k, entity_query: &str) -> Result<()> {
    // Load DataCore
    eprintln!("Loading DataCore...");
    let dcb_bytes = p4k
        .read_file("Data\\Game2.dcb")
        .or_else(|_| p4k.read_file("Data\\Game.dcb"))?;
    let db = starbreaker_datacore::Database::from_bytes(&dcb_bytes)?;

    // Find entities with audio triggers
    eprintln!("Searching for entities matching '{entity_query}'...");
    let entities =
        starbreaker_wwise::datacore_audio::search_entities_with_audio(&db, entity_query);

    if entities.is_empty() {
        eprintln!("No entities with audio triggers found matching '{entity_query}'");
        return Ok(());
    }

    // Build ATL index
    eprintln!("Building ATL index...");
    let atl = starbreaker_wwise::AtlIndex::from_p4k(p4k)?;

    // Cache loaded bank hierarchies
    let mut bank_cache: HashMap<String, Option<Hierarchy>> = HashMap::new();

    for entity in &entities {
        eprintln!(
            "\nEntity: {} ({})",
            entity.entity_name, entity.record_path
        );

        let mut total_triggers = 0;
        let mut total_sounds = 0;
        let mut banks_used = HashSet::new();

        for tref in &entity.triggers {
            total_triggers += 1;

            let (sound_count, bank_name) = match atl.get_trigger(&tref.trigger_name) {
                Some(trigger) => {
                    banks_used.insert(trigger.bank_name.clone());

                    let hierarchy = bank_cache
                        .entry(trigger.bank_name.clone())
                        .or_insert_with(|| {
                            let path =
                                format!("Data\\Sounds\\wwise\\{}", trigger.bank_name);
                            let data = match p4k.read_file(&path) {
                                Ok(d) => d,
                                Err(e) => {
                                    log::debug!("failed to read {path}: {e}");
                                    return None;
                                }
                            };
                            let bnk = match BnkFile::parse(&data) {
                                Ok(b) => b,
                                Err(e) => {
                                    log::debug!("failed to parse {path}: {e}");
                                    return None;
                                }
                            };
                            let hirc = bnk.hirc.as_ref()?;
                            Some(Hierarchy::from_section(hirc))
                        });

                    let count = hierarchy
                        .as_ref()
                        .map(|h| {
                            h.resolve_event_by_name(&trigger.wwise_event_name)
                                .len()
                        })
                        .unwrap_or(0);

                    (count, trigger.bank_name.clone())
                }
                None => (0, "?".to_string()),
            };

            total_sounds += sound_count;
            eprintln!(
                "  {} -> {} sounds ({})",
                tref.trigger_name, sound_count, bank_name
            );
        }

        eprintln!(
            "  Total: {} triggers, {} sounds across {} banks",
            total_triggers,
            total_sounds,
            banks_used.len()
        );
    }

    Ok(())
}
