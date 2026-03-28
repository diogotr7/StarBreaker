//! Standalone example: download all characters from star-citizen-characters.com
//!
//! Usage: cargo run -p starbreaker-chf --example download -- --output ./characters

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct SccRoot {
    body: SccBody,
}

#[derive(Deserialize)]
struct SccBody {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    rows: Vec<SccCharacter>,
}

#[derive(Deserialize)]
struct SccCharacter {
    id: String,
    title: String,
    #[serde(rename = "dnaUrl")]
    dna_url: Option<String>,
}

fn main() {
    let output = parse_output_arg();
    std::fs::create_dir_all(&output).expect("failed to create output directory");

    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .expect("failed to build HTTP client");

    eprintln!("Downloading all missing characters...");

    let mut page = 1;
    let mut total = 0;
    let mut retries = 0;
    loop {
        let url = format!(
            "https://www.star-citizen-characters.com/api/heads?page={page}&orderBy=latest"
        );
        let resp: SccRoot = match client.get(&url).send().unwrap().json() {
            Ok(r) => {
                retries = 0;
                r
            }
            Err(e) => {
                if retries >= 5 {
                    panic!("API request failed after 5 retries on page {page}: {e}");
                }
                retries += 1;
                let wait = retries * 10;
                eprintln!("Rate limited on page {page}, waiting {wait}s (retry {retries}/5)...");
                std::thread::sleep(std::time::Duration::from_secs(wait));
                continue;
            }
        };

        for character in &resp.body.rows {
            let Some(dna_url) = &character.dna_url else {
                continue;
            };

            if !dna_url.contains("chf") {
                eprintln!("Skipping {}, invalid dna url", character.title);
                continue;
            }

            let dir_name = safe_dir_name(&character.title, &character.id);
            let char_dir = output.join(&dir_name);
            std::fs::create_dir_all(&char_dir).expect("failed to create character directory");

            let dna_filename = dna_url.rsplit('/').next().unwrap_or("character.chf");
            let dna_path = char_dir.join(dna_filename);

            if dna_path.exists() {
                continue;
            }

            eprintln!("Downloading {}...", character.title);

            if let Err(e) = download_file(&client, dna_url, &dna_path) {
                eprintln!("Error downloading CHF for {}: {e}", character.title);
                continue;
            }

            total += 1;
        }

        if !resp.body.has_next_page {
            break;
        }
        page += 1;
        // Throttle to avoid rate limiting (Vercel edge function limits)
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    eprintln!("Downloaded {total} new characters.");
}

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = client.get(url).send()?.bytes()?;
    std::fs::write(path, &bytes)?;
    Ok(())
}

/// Match the C# `GetSafeDirectoryName` exactly: only replace invalid path chars
/// and spaces with '_', keep everything else (dots, parens, etc.).
fn safe_dir_name(title: &str, id: &str) -> String {
    const INVALID: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*', ' '];
    let safe: String = title
        .chars()
        .map(|c| if INVALID.contains(&c) || c.is_control() { '_' } else { c })
        .collect();
    let prefix = if id.len() >= 8 { &id[..8] } else { id };
    format!("{safe}-{prefix}")
}

fn parse_output_arg() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--output" || args[i] == "-o" {
            return PathBuf::from(args.get(i + 1).expect("missing value for --output"));
        }
        i += 1;
    }
    eprintln!("Usage: download --output <dir>");
    std::process::exit(1);
}
