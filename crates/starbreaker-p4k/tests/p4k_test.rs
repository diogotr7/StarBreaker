use starbreaker_p4k::{DirEntry, MappedP4k, P4kArchive};

/// Helper: auto-discover and open the P4k, or skip if not installed.
fn open_p4k_or_skip() -> Option<MappedP4k> {
    match starbreaker_p4k::open_p4k() {
        Ok(p4k) => Some(p4k),
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    }
}

#[test]
fn open_real_p4k() {
    let (p4k_path, _channel) = match starbreaker_p4k::find_p4k() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return;
        }
    };

    let t0 = std::time::Instant::now();
    let file = std::fs::File::open(&p4k_path).unwrap();
    let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
    let t_mmap = t0.elapsed();

    let t1 = std::time::Instant::now();
    let archive = P4kArchive::from_bytes(&mmap).unwrap();
    let t_parse = t1.elapsed();

    println!(
        "mmap: {t_mmap:?}, parse: {t_parse:?}, entries: {}",
        archive.len()
    );
    assert!(archive.len() > 100_000);
}

#[test]
fn lookup_known_entry() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let path = "Data\\Libs\\CharacterCustomizer\\MasculineDefault.xml";
    let entry = p4k.entry(path);
    assert!(entry.is_some(), "Entry not found: {path}");
    let entry = entry.unwrap();
    println!(
        "Found entry: {} (compressed={}, uncompressed={}, encrypted={}, method={})",
        entry.name,
        entry.compressed_size,
        entry.uncompressed_size,
        entry.is_encrypted,
        entry.compression_method
    );
}

#[test]
fn read_entry_from_p4k() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let path = "Data\\Libs\\CharacterCustomizer\\MasculineDefault.xml";
    let entry = p4k.entry(path).expect("Entry not found");

    let p4k_data = p4k.read(entry).expect("Failed to read entry from P4k");
    println!("Read {} bytes from P4k", p4k_data.len());
    assert!(!p4k_data.is_empty(), "Entry data should not be empty");
}

#[test]
fn read_encrypted_entry() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let encrypted = p4k.entries().iter().find(|e| e.is_encrypted);
    assert!(encrypted.is_some(), "No encrypted entries found");

    let entry = encrypted.unwrap();
    println!(
        "Reading encrypted entry: {} (compressed={}, uncompressed={}, method={})",
        entry.name, entry.compressed_size, entry.uncompressed_size, entry.compression_method
    );

    let data = p4k.read(entry).expect("Failed to read encrypted entry");
    assert!(!data.is_empty(), "Decrypted data is empty");
    println!(
        "Successfully read {} bytes from encrypted entry",
        data.len()
    );
}

#[test]
fn read_socpak_as_zip() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    // Find a .socpak file
    let socpak = p4k.entries().iter().find(|e| {
        e.name
            .to_lowercase()
            .contains("charactercustomizer_pu.socpak")
    });

    if socpak.is_none() {
        eprintln!("SKIP: No charactercustomizer_pu.socpak found");
        return;
    }

    let entry = socpak.unwrap();
    println!(
        "Reading socpak: {} ({} bytes compressed)",
        entry.name, entry.compressed_size
    );

    let socpak_data = p4k.read(entry).expect("Failed to read socpak");
    println!("Extracted socpak: {} bytes", socpak_data.len());

    // Parse the socpak as a ZIP/P4k archive
    let inner = P4kArchive::from_bytes(&socpak_data).expect("Failed to parse socpak as ZIP");
    println!("Socpak contains {} entries", inner.len());
    assert!(!inner.is_empty(), "Socpak has no entries");

    // Print first few entries
    for entry in inner.entries().iter().take(10) {
        println!("  - {}", entry.name);
    }
}

#[test]
fn entry_stats() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let total = p4k.len();
    let encrypted = p4k.entries().iter().filter(|e| e.is_encrypted).count();
    let zstd = p4k
        .entries()
        .iter()
        .filter(|e| e.compression_method == 100)
        .count();
    let deflate = p4k
        .entries()
        .iter()
        .filter(|e| e.compression_method == 8)
        .count();
    let stored = p4k
        .entries()
        .iter()
        .filter(|e| e.compression_method == 0)
        .count();
    let other = total - zstd - deflate - stored;

    println!("P4k Entry Statistics:");
    println!("  Total:     {total}");
    println!("  Encrypted: {encrypted}");
    println!("  Zstd:      {zstd}");
    println!("  Deflate:   {deflate}");
    println!("  Stored:    {stored}");
    println!("  Other:     {other}");
}

#[test]
fn list_dir_root() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let root = p4k.list_dir("");
    let dirs: Vec<_> = root
        .iter()
        .filter_map(|e| match e {
            DirEntry::Directory(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let files: Vec<_> = root
        .iter()
        .filter_map(|e| match e {
            DirEntry::File(entry) => Some(entry.name.as_str()),
            _ => None,
        })
        .collect();

    println!("Root: {} dirs, {} files", dirs.len(), files.len());
    for d in &dirs {
        println!("  [DIR] {d}");
    }
    for f in files.iter().take(5) {
        println!("  [FILE] {f}");
    }

    assert!(dirs.contains(&"Data"), "expected 'Data' directory at root");
}

#[test]
fn list_dir_character_customizer() {
    let Some(p4k) = open_p4k_or_skip() else {
        return;
    };

    let items = p4k.list_dir("Data\\Libs\\CharacterCustomizer");
    let dirs: Vec<_> = items
        .iter()
        .filter_map(|e| match e {
            DirEntry::Directory(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let files: Vec<_> = items
        .iter()
        .filter_map(|e| match e {
            DirEntry::File(entry) => Some(entry.name.as_str()),
            _ => None,
        })
        .collect();

    println!(
        "CharacterCustomizer: {} dirs, {} files",
        dirs.len(),
        files.len()
    );
    for d in &dirs {
        println!("  [DIR] {d}");
    }
    for f in &files {
        println!("  [FILE] {f}");
    }

    assert!(dirs.contains(&"PU"), "expected 'PU' subdirectory");
    assert!(
        files.iter().any(|f| f.contains("MasculineDefault")),
        "expected MasculineDefault"
    );
}
