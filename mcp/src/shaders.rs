//! Shader-cache helpers for the `shader_*` MCP tools: the game's Vulkan shader
//! cache ([`Shaders`], over `starbreaker-shadercache`), opened lazily per
//! `Data.p4k`, and the text helpers that make decoded HLSL readable.
//!
//! - [`Shaders::list`] (`shader_list`): the shader bins, effects (`.cfxb`) and
//!   includes (`.cfib`).
//! - [`Shaders::source`] (`shader_source`): one bin decoded to HLSL, filtered
//!   ([`grep`]) or windowed ([`window`]).
//! - [`Shaders::search`] (`shader_search`): a needle across every decoded
//!   bin (decoded once, cached).
//! - [`Shaders::reflect`] (`shader_reflect`): a family's entries, or one
//!   entry's permutations and a variant's SPIR-V reflection.
//! - [`statements`]: decoded source (whole functions on one line) split into
//!   one statement per line at `;`, `{` and `}`, preprocessor lines kept.
//! - [`grep`]: statements containing a needle (case-insensitive), each with
//!   `context` statements around it, numbered as in [`statements`].
//! - [`window`]: statements `start..start + max`, numbered.
//!
//! Decoded shader source is game content: these tools return it to the
//! caller and never write it anywhere.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use rmcp::schemars;
use starbreaker_shadercache::{SUPPORTED_CACHE_VERSION, ShaderPak, cfxb, hlsl};

/// Default and maximum number of statements a `shader_source` call returns.
const DEFAULT_LINES: usize = 200;
const MAX_LINES: usize = 2000;
/// Default and maximum hits `shader_search` returns.
const DEFAULT_HITS: usize = 50;
const MAX_HITS: usize = 500;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ShaderListRequest {
    #[schemars(description = "Optional case-insensitive substring filter on the bin name (e.g. 'decal', 'layer').")]
    pub filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ShaderSourceRequest {
    #[schemars(description = "Shader bin name: an effect (e.g. 'HardSurface', 'MeshDecal') or an include (e.g. 'layerLib', 'common'). Exact case wins; otherwise matched case-insensitively.")]
    pub name: String,
    #[schemars(description = "Optional case-insensitive text to find; returns only matching statements with context.")]
    pub grep: Option<String>,
    #[schemars(description = "Statements of context around each grep hit (default 3).")]
    pub context: Option<u32>,
    #[schemars(description = "Without grep: first statement to return, 1-based (default 1).")]
    pub start: Option<u32>,
    #[schemars(description = "Maximum statements to return (default 200, max 2000).")]
    pub max_lines: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ShaderSearchRequest {
    #[schemars(description = "Case-insensitive text to find across every decoded shader bin (e.g. 'MeshInvBounds', 'void SetGBufferBlending').")]
    pub query: String,
    #[schemars(description = "Maximum hits (default 50, max 500).")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ShaderReflectRequest {
    #[schemars(description = "Shader family (effect name, e.g. 'HardSurface').")]
    pub family: String,
    #[schemars(description = "Entry point (e.g. 'HardSurfaceZPassPS'). Omit to list the family's entries.")]
    pub entry: Option<String>,
    #[schemars(description = "Optional substring of the permutation path picking the variant to reflect (e.g. 'GL908', 'RT0').")]
    pub variant: Option<String>,
    #[schemars(description = "Maximum permutation paths listed (default 20).")]
    pub limit: Option<u32>,
}

/// The shader cache of one `Data.p4k`, with decoded sources cached.
pub struct Shaders {
    pak: ShaderPak,
    decoded: OnceLock<BTreeMap<String, Result<Vec<String>, String>>>,
}

impl Shaders {
    /// Open the Vulkan shader cache inside `p4k`.
    pub fn open(p4k: &Path) -> Result<Shaders, String> {
        let pak = ShaderPak::open_in_p4k(p4k).map_err(|e| format!("shader cache: {e}"))?;
        Ok(Shaders { pak, decoded: OnceLock::new() })
    }

    /// Bin `name`: the exact stem, else its one case-insensitive match.
    fn stem(&self, name: &str) -> Result<String, String> {
        if self.pak.shader_bin_ext(name).is_some() {
            return Ok(name.to_owned());
        }
        let hits: Vec<&str> = self.pak.shader_bins().filter(|s| s.eq_ignore_ascii_case(name)).collect();
        match hits.as_slice() {
            [one] => Ok((*one).to_owned()),
            [] => Err(format!("no shader bin named '{name}' (see shader_list)")),
            many => Err(format!("'{name}' matches {many:?}: give the exact case")),
        }
    }

    fn decode(&self, stem: &str) -> Result<Vec<String>, String> {
        let raw = self.pak.read_shader_bin(stem).map_err(|e| e.to_string())?;
        let bin = cfxb::parse(&raw).map_err(|e| e.to_string())?;
        Ok(statements(&hlsl::decode(&bin).map_err(|e| e.to_string())?))
    }

    fn decoded_all(&self) -> &BTreeMap<String, Result<Vec<String>, String>> {
        self.decoded.get_or_init(|| {
            let stems: Vec<String> = self.pak.shader_bins().map(str::to_owned).collect();
            stems.into_iter().map(|s| {
                let d = self.decode(&s);
                (s, d)
            }).collect()
        })
    }

    /// The shader bins, effects and includes, optionally filtered.
    pub fn list(&self, filter: Option<&str>) -> String {
        let filter = filter.map(str::to_lowercase);
        let (mut effects, mut includes) = (Vec::new(), Vec::new());
        for stem in self.pak.shader_bins() {
            if filter.as_ref().is_some_and(|f| !stem.to_lowercase().contains(f)) {
                continue;
            }
            match self.pak.shader_bin_ext(stem) {
                Some("cfib") => includes.push(stem),
                _ => effects.push(stem),
            }
        }
        serde_json::to_string_pretty(&serde_json::json!({
            "api": self.pak.api(),
            "grammar_cache_version": format!("{SUPPORTED_CACHE_VERSION:#010x}"),
            "effects": effects,
            "includes": includes,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    /// One bin decoded, filtered by `grep` or windowed.
    pub fn source(&self, req: &ShaderSourceRequest) -> String {
        let stem = match self.stem(&req.name) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let lines = match self.decode(&stem) {
            Ok(l) => l,
            Err(e) => return format!("{stem}: {e}"),
        };
        let max = req.max_lines.map_or(DEFAULT_LINES, |m| (m as usize).min(MAX_LINES));
        let ext = self.pak.shader_bin_ext(&stem).unwrap_or("");
        let body: Vec<String> = match &req.grep {
            Some(needle) => grep(&lines, needle, req.context.map_or(3, |c| c as usize)),
            None => window(&lines, req.start.map_or(1, |s| s as usize), usize::MAX),
        };
        let shown = body.len().min(max);
        let more = if body.len() > shown { format!(" (first {shown} of {} shown)", body.len()) } else { String::new() };
        format!("// {stem}.{ext}: {} statements{more}\n{}", lines.len(), body[..shown].join("\n"))
    }

    /// `query` across every decoded bin.
    pub fn search(&self, query: &str, limit: Option<u32>) -> String {
        let limit = limit.map_or(DEFAULT_HITS, |l| (l as usize).min(MAX_HITS));
        let needle = query.to_lowercase();
        let (mut out, mut total, mut failed) = (Vec::new(), 0usize, Vec::new());
        for (stem, decoded) in self.decoded_all() {
            let Ok(lines) = decoded else {
                failed.push(stem.as_str());
                continue;
            };
            for (i, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    total += 1;
                    if out.len() < limit {
                        out.push(format!("{stem}:{}: {line}", i + 1));
                    }
                }
            }
        }
        let mut head = format!("{total} hits for '{query}'");
        if total > out.len() {
            head.push_str(&format!(" (first {} shown)", out.len()));
        }
        if !failed.is_empty() {
            head.push_str(&format!("; {} bins did not decode: {failed:?}", failed.len()));
        }
        format!("{head}\n{}", out.join("\n"))
    }

    /// A family's entries, or one entry's permutations and a variant's reflection.
    pub fn reflect(&self, req: &ShaderReflectRequest) -> String {
        let perms: Vec<_> = self.pak.family(&req.family).collect();
        if perms.is_empty() {
            return format!("no permutations of family '{}' (family names are effect names, see shader_list)", req.family);
        }
        let Some(entry) = &req.entry else {
            let mut entries: BTreeMap<(String, String), usize> = BTreeMap::new();
            for p in &perms {
                *entries.entry((p.entry.clone(), format!("{:?}", p.stage))).or_default() += 1;
            }
            let rows: Vec<_> =
                entries.into_iter().map(|((e, st), n)| serde_json::json!({ "entry": e, "stage": st, "permutations": n })).collect();
            return serde_json::to_string_pretty(&rows).unwrap_or_else(|e| format!("JSON error: {e}"));
        };
        let matching: Vec<_> = perms
            .into_iter()
            .filter(|p| &p.entry == entry && req.variant.as_ref().is_none_or(|v| p.path.contains(v.as_str())))
            .collect();
        let Some(first) = matching.first() else {
            return format!("no permutation of {}@{entry} matches", req.family);
        };
        let reflection = match self.pak.reflect(first) {
            Ok(r) => serde_json::to_value(&r).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        let hex = |m: Option<u64>| m.map(|m| format!("{m:x}"));
        let limit = req.limit.map_or(20, |l| l as usize);
        serde_json::to_string_pretty(&serde_json::json!({
            "permutations": matching.len(),
            "listed": matching.iter().take(limit).map(|p| &p.path).collect::<Vec<_>>(),
            "reflected": first.path,
            "masks": { "gl": hex(first.masks.gl), "rt": hex(first.masks.rt), "mdv": hex(first.masks.mdv) },
            "reflection": reflection,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }
}

/// Split decoded HLSL into one statement per line.
pub fn statements(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() {
            out.push(s.to_owned());
        }
    };
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            push(line);
            continue;
        }
        let mut current = String::new();
        for c in line.chars() {
            match c {
                ';' | '{' => {
                    current.push(c);
                    push(&current);
                    current.clear();
                }
                '}' => {
                    push(&current);
                    current.clear();
                    push("}");
                }
                _ => current.push(c),
            }
        }
        push(&current);
    }
    out
}

/// Numbered statements (1-based) containing `needle`, case-insensitively,
/// each with `context` statements before and after; overlapping windows merge.
/// A `--` line separates windows.
pub fn grep(lines: &[String], needle: &str, context: usize) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.to_lowercase().contains(&needle) {
            continue;
        }
        let (start, end) = (i.saturating_sub(context), (i + context).min(lines.len() - 1));
        match windows.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => windows.push((start, end)),
        }
    }
    let mut out = Vec::new();
    for (k, (start, end)) in windows.into_iter().enumerate() {
        if k > 0 {
            out.push("--".to_owned());
        }
        out.extend((start..=end).map(|i| format!("{}: {}", i + 1, lines[i])));
    }
    out
}

/// Statements `start..start + max` (1-based), numbered.
pub fn window(lines: &[String], start: usize, max: usize) -> Vec<String> {
    let first = start.max(1) - 1;
    lines.iter().enumerate().skip(first).take(max).map(|(i, l)| format!("{}: {l}", i + 1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "float4 Unpack (float4 c) { const float m = 0.5; c = mad (c, 2, - m); return c; }\n#if %DETAIL\nfloat d = Unpack (x).z;\n#endif";

    #[test]
    fn statements_split_at_semicolons_and_braces() {
        assert_eq!(
            statements(SRC),
            [
                "float4 Unpack (float4 c) {",
                "const float m = 0.5;",
                "c = mad (c, 2, - m);",
                "return c;",
                "}",
                "#if %DETAIL",
                "float d = Unpack (x).z;",
                "#endif",
            ]
        );
    }

    #[test]
    fn grep_numbers_hits_with_context_and_merges_windows() {
        let lines = statements(SRC);
        assert_eq!(grep(&lines, "UNPACK", 0), ["1: float4 Unpack (float4 c) {", "--", "7: float d = Unpack (x).z;"]);
        assert_eq!(
            grep(&lines, "mad", 1),
            ["2: const float m = 0.5;", "3: c = mad (c, 2, - m);", "4: return c;"]
        );
        assert_eq!(grep(&lines, "return", 2).len(), 5, "windows 2..6 and none overlapping: one block");
        assert!(grep(&lines, "absent", 3).is_empty());
    }

    /// On the installed game: the bins list, an effect decodes and greps, a
    /// search finds a function's definition across includes, and a family's
    /// pixel entry reflects with its G-buffer outputs.
    #[test]
    #[ignore = "needs Data.p4k"]
    fn tools_answer_on_real_data() {
        let Ok((p4k, _)) = starbreaker_p4k::find_p4k() else {
            eprintln!("no Data.p4k; skipping");
            return;
        };
        let s = Shaders::open(&p4k).unwrap();
        let list: serde_json::Value = serde_json::from_str(&s.list(Some("decal"))).unwrap();
        assert!(list["effects"].as_array().unwrap().iter().any(|e| e == "MeshDecal"), "{list}");
        let src = s.source(&ShaderSourceRequest { name: "meshdecal".into(), grep: Some("SetGBufferBlending".into()), context: Some(1), start: None, max_lines: None });
        assert!(src.starts_with("// MeshDecal.cfxb:"), "{src}");
        assert!(src.contains("SetGBufferBlending ("), "{src}");
        let hits = s.search("void SetGBufferBlending", Some(5));
        assert!(hits.contains("common:"), "{hits}");
        let entries = s.reflect(&ShaderReflectRequest { family: "MeshDecal".into(), entry: None, variant: None, limit: None });
        assert!(entries.contains("MeshDecalZPassPS"), "{entries}");
        let one = s.reflect(&ShaderReflectRequest { family: "HardSurface".into(), entry: Some("HardSurfaceZPassPS".into()), variant: None, limit: Some(2) });
        let v: serde_json::Value = serde_json::from_str(&one).unwrap();
        assert!(v["reflection"]["outputs"].as_array().is_some_and(|o| o.len() >= 3), "{one}");
    }

    #[test]
    fn window_is_one_based_and_clamped() {
        let lines = statements(SRC);
        assert_eq!(window(&lines, 7, 5), ["7: float d = Unpack (x).z;", "8: #endif"]);
        assert!(window(&lines, 20, 5).is_empty());
        assert_eq!(window(&lines, 0, 1), ["1: float4 Unpack (float4 c) {"], "0 reads as 1");
    }
}
