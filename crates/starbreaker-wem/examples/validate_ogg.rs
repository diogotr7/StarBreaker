/// Validate decoded .ogg files and diagnose issues
use std::env;
use std::fs;
use std::io::Cursor;

use starbreaker_wem::{WemFile, WemCodec};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: validate_ogg <file.wem>");
        std::process::exit(1);
    }

    let path = &args[1];
    let data = fs::read(path).unwrap();

    // Parse WEM
    let wem = WemFile::parse(&data).unwrap();
    eprintln!("WEM: {} {}Hz {}ch", wem.codec_type(), wem.sample_rate(), wem.channels());
    eprintln!("Audio data size: {} bytes", wem.audio_data().map(|d| d.len()).unwrap_or(0));

    // Try default codebooks
    eprintln!("\n--- Default codebooks ---");
    match try_decode(&data, ww2ogg::CodebookLibrary::default_codebooks) {
        Ok(ogg) => {
            eprintln!("  Decode OK: {} bytes", ogg.len());
            match ww2ogg::validate(&ogg) {
                Ok(()) => eprintln!("  Validation: PASS"),
                Err(e) => eprintln!("  Validation: FAIL - {e}"),
            }
        }
        Err(e) => eprintln!("  Decode FAILED: {e}"),
    }

    // Try aoTuV codebooks
    eprintln!("\n--- aoTuV codebooks ---");
    match try_decode(&data, ww2ogg::CodebookLibrary::aotuv_codebooks) {
        Ok(ogg) => {
            eprintln!("  Decode OK: {} bytes", ogg.len());
            match ww2ogg::validate(&ogg) {
                Ok(()) => eprintln!("  Validation: PASS"),
                Err(e) => eprintln!("  Validation: FAIL - {e}"),
            }
        }
        Err(e) => eprintln!("  Decode FAILED: {e}"),
    }
}

fn try_decode(
    wem_bytes: &[u8],
    codebook_fn: fn() -> ww2ogg::WemResult<ww2ogg::CodebookLibrary>,
) -> Result<Vec<u8>, String> {
    let codebooks = codebook_fn().map_err(|e| e.to_string())?;
    let input = Cursor::new(wem_bytes);
    let mut converter = ww2ogg::WwiseRiffVorbis::new(input, codebooks).map_err(|e| e.to_string())?;
    let mut ogg = Vec::new();
    converter.generate_ogg(&mut ogg).map_err(|e| e.to_string())?;
    Ok(ogg)
}
