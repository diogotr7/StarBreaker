//! chrparams search-rule expansion and selective DBA/CAF decoding for player kits.
//! Only referenced clips are decoded; metadata indexing does not expand bone tracks.
use super::{
    clips,
    loadouts::read,
    xml::{self, Result, error},
};
use crate::animation::{AnimationClip, parse_caf, parse_dba};
use serde_json::Value;
use starbreaker_chunks::ChunkFile;
use starbreaker_p4k::MappedP4k;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn stem(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let file = normalized.rsplit('/').next().unwrap_or(&normalized);
    file.rsplit_once('.')
        .map(|(n, _)| n)
        .unwrap_or(file)
        .to_ascii_lowercase()
}
fn norm(path: &str) -> String {
    let s = path.replace('\\', "/").to_ascii_lowercase();
    if s.starts_with("data/") { s } else { format!("data/{s}") }
}
fn matches(pattern: &str, path: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == path;
    }
    let mut rest = path;
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if let Some(j) = rest.find(p) {
            rest = &rest[j + p.len()..];
        } else {
            return false;
        }
    }
    parts.last() == Some(&"") || rest.is_empty()
}

pub(super) struct Sources<'a> {
    pub p4k: &'a MappedP4k,
    pub files: BTreeMap<String, String>,
    dbas: Vec<String>,
    index: BTreeMap<String, (String, u32)>,
    decoded: BTreeMap<String, (AnimationClip, u32)>,
    indexed: bool,
}
impl<'a> Sources<'a> {
    pub fn new(p4k: &'a MappedP4k, skeleton: &str) -> Result<Self> {
        let path = format!(
            "{}.chrparams",
            skeleton.rsplit_once('.').ok_or_else(|| error("skeleton extension"))?.0
        );
        let doc = xml::parse(&read(p4k, &path)?)?;
        let (patterns, db_patterns) = search_rules(
            &doc,
            &mut |path| xml::parse(&read(p4k, path)?),
            &mut BTreeSet::from([path.to_ascii_lowercase()]),
        )?;
        let mut files = BTreeMap::new();
        let mut dbas = Vec::new();
        for e in p4k.entries() {
            let n = norm(&e.name);
            if db_patterns.iter().any(|p| matches(p, &n)) {
                dbas.push(e.name.clone());
            }
            for (alias, p) in &patterns {
                if matches(p, &n) {
                    files
                        .entry(if alias.contains('*') {
                            stem(&n)
                        } else {
                            alias.to_ascii_lowercase()
                        })
                        .or_insert(e.name.clone());
                    break;
                }
            }
        }
        dbas.sort();
        dbas.dedup();
        Ok(Self {
            p4k,
            files,
            dbas,
            index: BTreeMap::new(),
            decoded: BTreeMap::new(),
            indexed: false,
        })
    }
    fn index(&mut self) -> Result<()> {
        if self.indexed {
            return Ok(());
        }
        use rayon::prelude::*;
        let rows = self
            .dbas
            .par_iter()
            .map(|p| -> Result<_> {
                let data = read(self.p4k, p)?;
                Ok((p.clone(), metadata(&data)?))
            })
            .collect::<Result<Vec<_>>>()?;
        for (path, rows) in rows {
            for (name, flags) in rows {
                self.index.entry(stem(&name)).or_insert((path.clone(), flags));
            }
        }
        self.indexed = true;
        Ok(())
    }
    pub fn clip(&mut self, name: &str) -> Result<&(AnimationClip, u32)> {
        let key = name.to_ascii_lowercase();
        if !self.decoded.contains_key(&key) {
            if let Some(path) = self
                .files
                .get(&key)
                .filter(|p| p.to_ascii_lowercase().ends_with(".caf"))
            {
                let bytes = read(self.p4k, path)?;
                let flags = caf_flags(&bytes)?;
                let mut clip = parse_caf(&bytes)
                    .map_err(|e| error(e.to_string()))?
                    .clips
                    .into_iter()
                    .next()
                    .ok_or_else(|| error("empty CAF"))?;
                clip.name = key.clone();
                self.decoded.insert(key.clone(), (clip, flags));
            } else {
                self.index()?;
                let (path, _) = self
                    .index
                    .get(&key)
                    .ok_or_else(|| error(format!("clip not in chrparams sources: {name}")))?
                    .clone();
                let bytes = read(self.p4k, &path)?;
                for mut clip in parse_dba(&bytes).map_err(|e| error(format!("DBA {path}: {e}")))?.clips {
                    let k = stem(&clip.name);
                    let flags = self
                        .index
                        .get(&k)
                        .filter(|(p, _)| p == &path)
                        .map(|(_, f)| *f)
                        .ok_or_else(|| error(format!("DBA metadata missing {k}")))?;
                    clip.name = k.clone();
                    self.decoded.entry(k).or_insert((clip, flags));
                }
            }
        }
        self.decoded
            .get(&key)
            .ok_or_else(|| error(format!("DBA did not decode {name}")))
    }
    pub fn export(
        &mut self,
        names: &BTreeSet<String>,
        skeleton: &str,
        bones: &[crate::skeleton::Bone],
    ) -> Result<(BTreeMap<String, Value>, BTreeSet<String>)> {
        let mut out = BTreeMap::new();
        let mut missing = BTreeSet::new();
        self.index()?;
        for name in names {
            // Registered DBA names may refer to another rig's asset directory.
            // Registration, not the gender in a path, determines availability.
            let loose = self
                .files
                .get(name)
                .is_some_and(|p| p.to_ascii_lowercase().ends_with(".caf"));
            if !loose && !self.index.contains_key(name) {
                missing.insert(name.clone());
                continue;
            }
            let (clip, flags) = self.clip(name)?;
            out.insert(name.clone(), clips::sidecar(clip, *flags, skeleton, bones)?);
        }
        Ok((out, missing))
    }
}
fn caf_flags(bytes: &[u8]) -> Result<u32> {
    let ChunkFile::Ivo(f) = ChunkFile::from_bytes(bytes).map_err(|e| error(e.to_string()))? else {
        return Err(error("CAF is not IVO"));
    };
    let c = f
        .chunks()
        .iter()
        .find(|c| c.chunk_type == 0x4733C6ED)
        .ok_or_else(|| error("CAF has no IvoAnimInfo"))?;
    let b = f.chunk_data(c);
    Ok(u32::from_le_bytes(
        b.get(..4).ok_or_else(|| error("short AnimInfo"))?.try_into().unwrap(),
    ))
}
fn metadata(bytes: &[u8]) -> Result<Vec<(String, u32)>> {
    let ChunkFile::Ivo(f) = ChunkFile::from_bytes(bytes).map_err(|e| error(e.to_string()))? else {
        return Err(error("DBA is not IVO"));
    };
    let c = f
        .chunks()
        .iter()
        .find(|c| c.chunk_type == 0xF7351608)
        .ok_or_else(|| error("DBA has no metadata"))?;
    let b = f.chunk_data(c);
    let count = u32::from_le_bytes(
        b.get(..4)
            .ok_or_else(|| error("short DBA metadata"))?
            .try_into()
            .unwrap(),
    ) as usize;
    let end = count
        .checked_mul(48)
        .and_then(|n| n.checked_add(4))
        .filter(|n| *n <= b.len())
        .ok_or_else(|| error("invalid DBA metadata length"))?;
    let mut pos = end.next_multiple_of(8);
    while b.get(pos) == Some(&0) {
        pos += 1;
    }
    let mut out = Vec::new();
    for i in 0..count {
        let len = b
            .get(pos..)
            .and_then(|b| b.iter().position(|b| *b == 0))
            .ok_or_else(|| error("DBA name missing terminator"))?;
        let name = std::str::from_utf8(&b[pos..pos + len])
            .map_err(|e| error(e.to_string()))?
            .to_owned();
        pos += len + 1;
        // DBA v902 embeds AnimInfo after its four-byte entry prefix.
        let flags = u32::from_le_bytes(b[8 + i * 48..12 + i * 48].try_into().unwrap());
        out.push((name, flags));
    }
    Ok(out)
}
/// Expand authored includes and path chains without inventing cross-rig roots.
pub(super) fn search_rules(
    doc: &xml::Node,
    load: &mut impl FnMut(&str) -> Result<xml::Node>,
    seen: &mut BTreeSet<String>,
) -> Result<(Vec<(String, String)>, Vec<String>)> {
    let list = doc.child("AnimationList").ok_or_else(|| error("no AnimationList"))?;
    let mut prefix = String::new();
    let mut patterns = Vec::new();
    let mut db_patterns = Vec::new();
    for n in &list.children {
        match n.attr("name") {
            "$Include" => {
                let path = n.attr("path");
                if !seen.insert(norm(path)) {
                    return Err(error(format!("chrparams include cycle: {path}")));
                }
                let (files, dbs) = search_rules(&load(path)?, load, seen)?;
                patterns.extend(files);
                db_patterns.extend(dbs);
                seen.remove(&norm(path));
            }
            "#filepath" => prefix = n.attr("path").into(),
            "$TracksDatabase" => db_patterns.push(norm(n.attr("path"))),
            name if !name.starts_with('$') => {
                patterns.push((name.to_owned(), norm(&format!("{prefix}/{}", n.attr("path")))))
            }
            _ => {}
        }
    }
    Ok((patterns, db_patterns))
}
