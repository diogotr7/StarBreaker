/// Decode standalone .wem files to .ogg
use std::env;
use std::fs;
use std::path::Path;

use starbreaker_wem::{WemFile, WemCodec};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: decode_wem <file.wem> [file2.wem ...]");
        std::process::exit(1);
    }

    for path_str in &args[1..] {
        let path = Path::new(path_str);
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => { eprintln!("  Error reading {}: {e}", path.display()); continue; }
        };

        let wem = match WemFile::parse(&data) {
            Ok(w) => w,
            Err(e) => { eprintln!("  Error parsing {}: {e}", path.display()); continue; }
        };

        let id = path.file_stem().unwrap().to_string_lossy();
        eprintln!(
            "{}: {} {}Hz {}ch ~{:.1}s",
            id, wem.codec_type(), wem.sample_rate(), wem.channels(),
            wem.estimated_duration_secs().unwrap_or(0.0)
        );

        if wem.codec_type() == WemCodec::Vorbis {
            match starbreaker_wem::decode::vorbis_to_ogg(&data) {
                Ok(ogg) => {
                    let out = path.with_extension("ogg");
                    fs::write(&out, &ogg).unwrap();
                    eprintln!("  -> {} ({} bytes)", out.display(), ogg.len());
                }
                Err(e) => eprintln!("  Decode error: {e}"),
            }
        } else {
            eprintln!("  Codec {} not supported for decode", wem.codec_type());
        }
    }
}
