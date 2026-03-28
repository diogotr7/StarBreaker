use std::env;
use std::fs;
use std::path::Path;

use starbreaker_wem::WemFile;
use starbreaker_wwise::BnkFile;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: dump_bnk <path.bnk> [output_dir]");
        std::process::exit(1);
    }

    let bnk_path = &args[1];
    let output_dir = args.get(2).map(|s| s.as_str()).unwrap_or("./bnk_dump");

    let data = fs::read(bnk_path)?;
    let bnk = BnkFile::parse(&data)?;

    eprintln!("Bank version: {}", bnk.header.version);
    eprintln!("Bank ID:      {}", bnk.header.bank_id);
    eprintln!("WEM count:    {}", bnk.wem_count());

    if let Some(ref hirc) = bnk.hirc {
        eprintln!("HIRC objects: {}", hirc.entries.len());
        let counts = hirc.type_counts();
        let mut sorted: Vec<_> = counts.iter().collect();
        sorted.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        for (type_id, count) in sorted {
            let name = starbreaker_wwise::HircObjectType::from_u8(*type_id)
                .map(|t| t.name().to_string())
                .unwrap_or_else(|| format!("Unknown({})", type_id));
            eprintln!("  {name}: {count}");
        }
    }

    if bnk.wem_count() > 0 {
        fs::create_dir_all(output_dir)?;
        eprintln!("\nExtracting WEMs to {output_dir}/...");

        for entry in &bnk.data_index {
            // Copy packed fields to locals to avoid alignment issues with #[repr(C, packed)].
            let entry_id = entry.id;

            let wem_bytes = bnk.wem_data(entry)?;

            // Try to parse WEM header for info
            match WemFile::parse(wem_bytes) {
                Ok(wem) => {
                    eprintln!(
                        "  WEM {}: {} {}Hz {}ch ~{:.1}s",
                        entry_id,
                        wem.codec_type(),
                        wem.sample_rate(),
                        wem.channels(),
                        wem.estimated_duration_secs().unwrap_or(0.0)
                    );

                    // Try Vorbis decode
                    if wem.codec_type() == starbreaker_wem::WemCodec::Vorbis {
                        match starbreaker_wem::decode::vorbis_to_ogg(wem_bytes) {
                            Ok(ogg) => {
                                let path =
                                    Path::new(output_dir).join(format!("{}.ogg", entry_id));
                                fs::write(&path, ogg)?;
                                continue;
                            }
                            Err(e) => {
                                eprintln!("    decode failed: {e}, writing raw .wem");
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  WEM {}: parse error: {e}", entry_id);
                }
            }

            // Fallback: write raw WEM
            let path = Path::new(output_dir).join(format!("{}.wem", entry_id));
            fs::write(&path, wem_bytes)?;
        }
    }

    Ok(())
}
