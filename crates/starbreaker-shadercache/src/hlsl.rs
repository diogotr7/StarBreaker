//! Render a decoded `.cfxb` token stream back to readable HLSL.
//!
//! The token stream carries no whitespace: token id `0` marks a line break and
//! everything else is a keyword (from the [`Grammar`]) or an identifier /
//! literal (from the file's own string table). The printer only inserts
//! whitespace and line breaks; it never changes token order.
//!
//! [`decode`] is the safe entry point: it picks the grammar by the bin's cache
//! version and refuses streams it cannot vouch for. [`render`] and [`validate`]
//! take an explicit grammar.

use std::collections::HashMap;

use crate::cfxb::ShaderBin;
use crate::error::ShaderCacheError;
use crate::grammar::Grammar;

const COMPOUND: &[(char, char, &str)] = &[
    ('&', '&', "&&"),
    ('|', '|', "||"),
    ('>', '>', ">>"),
    ('<', '<', "<<"),
];

fn compound(prev: &str, cur: &str) -> Option<&'static str> {
    let mut p = prev.chars();
    let mut c = cur.chars();
    let (Some(a), Some(b), None, None) = (p.next(), c.next(), p.next(), c.next()) else {
        return None;
    };
    // `>>=` / `<<=` arrive as (`>>`, `=`)
    if let Some(win) = COMPOUND.iter().find(|(x, y, _)| *x == a && *y == b) {
        return Some(win.2);
    }
    match (a, b) {
        ('=', '=') => Some("=="),
        ('!', '=') => Some("!="),
        ('<', '=') => Some("<="),
        ('>', '=') => Some(">="),
        ('*', '=') => Some("*="),
        ('/', '=') => Some("/="),
        ('+', '=') => Some("+="),
        ('-', '=') => Some("-="),
        ('&', '=') => Some("&="),
        ('|', '=') => Some("|="),
        ('^', '=') => Some("^="),
        ('+', '+') => Some("++"),
        ('-', '-') => Some("--"),
        _ => None,
    }
}

fn is_word(s: &str) -> bool {
    let s = s.trim_start_matches('%');
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

const TYPES: &[&str] = &[
    "float", "float2", "float3", "float4", "float4x4", "float3x4", "float3x3", "float2x4",
    "float4x2", "half", "int", "uint", "bool", "void", "struct", "return", "static", "const",
    "Texture2D", "Texture3D", "TextureCube", "SamplerState", "matrix",
];

/// Render tokens to text.
pub fn render(tokens: &[u32], grammar: &Grammar, table: &HashMap<u32, String>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<String> = None;
    let mut prev_word = false;
    let mut directive: Option<String> = None;

    for &token in tokens {
        if token == 0 {
            out.push("\n".to_owned());
            prev = None;
            prev_word = false;
            directive = None;
            continue;
        }
        let text = grammar
            .get(token)
            .map(str::to_owned)
            .or_else(|| table.get(&token).cloned())
            .unwrap_or_else(|| format!("<tok:{token:#x}>"));

        if text.starts_with('#') {
            if prev.as_deref().is_some_and(|p| p != "\n") {
                out.push("\n".to_owned());
            }
            out.push(text.clone());
            let is_cond = matches!(
                text.as_str(),
                "#if" | "#ifdef" | "#ifndef" | "#elif" | "#else" | "#endif"
            );
            directive = is_cond.then(|| text.clone());
            prev = Some(text.clone());
            prev_word = false;
            continue;
        }

        let mut emitted_newline = false;
        if let Some(dir) = directive.clone() {
            let breaks = if dir == "#else" || dir == "#endif" {
                is_word(&text)
            } else {
                (is_word(&text) && prev_word) || TYPES.contains(&text.as_str())
            };
            if breaks {
                out.push("\n".to_owned());
                directive = None;
                prev = None;
                emitted_newline = true;
            }
        }

        if let Some(p) = prev.as_deref()
            && !emitted_newline {
                if let Some(joined) = compound(p, &text) {
                    out.pop();
                    out.push(joined.to_owned());
                    prev = Some(joined.to_owned());
                    continue;
                }
                let need_space = !matches!(text.as_str(), "," | ";" | ")" | "]" | "." | ":" | ">")
                    && !matches!(p, "(" | "[" | "." | "!" | "<" | "~");
                if need_space {
                    out.push(" ".to_owned());
                }
            }
        out.push(text.clone());
        prev_word = is_word(&text);
        prev = Some(text);
    }

    let mut text = out.concat();
    while text.contains("\n\n") {
        text = text.replace("\n\n", "\n");
    }
    text
}

/// Decode a shader bin to HLSL with the grammar shipped for its cache version.
///
/// Refuses, rather than guessing, when the bin's cache version has no shipped
/// grammar ([`ShaderCacheError::UnsupportedCacheVersion`]) or when the stream
/// does not hold up under it ([`ShaderCacheError::UntrustedDecode`]).
pub fn decode(bin: &ShaderBin) -> Result<String, ShaderCacheError> {
    let grammar = crate::grammar::for_cache_version(bin.version)?;
    validate(&bin.tokens, grammar, &bin.table).map_err(ShaderCacheError::UntrustedDecode)?;
    Ok(render(&bin.tokens, grammar, &bin.table))
}

/// Count tokens that resolve through neither the grammar nor the string table.
///
/// A non-zero count means the grammar does not match this stream.
pub fn unresolved(tokens: &[u32], grammar: &Grammar, table: &HashMap<u32, String>) -> usize {
    tokens
        .iter()
        .filter(|&&t| t != 0 && grammar.get(t).is_none() && !table.contains_key(&t))
        .count()
}

/// Why a decoded token stream cannot be trusted with a given grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeIssue {
    /// Token ids that resolve to neither a keyword nor a table string.
    Unresolved(usize),
    /// `#if`-family directives are not closed by matching `#endif`s.
    UnbalancedConditionals,
}

impl std::fmt::Display for DecodeIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unresolved(n) => write!(f, "{n} unresolved token(s)"),
            Self::UnbalancedConditionals => write!(f, "#if/#endif do not balance"),
        }
    }
}

/// Sanity-check a token stream under a grammar.
///
/// This is a backstop, not the version check — the grammar is chosen by the
/// bin's cache version ([`crate::grammar::for_cache_version`]). Only properties
/// that hold for every well-formed source are checked: every id resolves, and
/// `#if`-family directives balance with `#endif`.
///
/// Deliberately *not* checked, because real sources break them: brace balance
/// (the stream still contains every preprocessor branch, so `#if … { #else … {
/// #endif` counts twice), and `#include` names coming from the string table (an
/// include file named like a keyword, e.g. `ProjectedEffect`, is tokenized as
/// that keyword).
pub fn validate(
    tokens: &[u32],
    grammar: &Grammar,
    table: &HashMap<u32, String>,
) -> Result<(), DecodeIssue> {
    let missing = unresolved(tokens, grammar, table);
    if missing > 0 {
        return Err(DecodeIssue::Unresolved(missing));
    }

    let mut open = 0i32;
    for text in tokens.iter().filter_map(|&t| grammar.get(t)) {
        match text {
            "#if" | "#ifdef" | "#ifndef" | "#ifcvar" | "#ifncvar" => open += 1,
            "#endif" => open -= 1,
            _ => {}
        }
        if open < 0 {
            return Err(DecodeIssue::UnbalancedConditionals);
        }
    }
    if open != 0 {
        return Err(DecodeIssue::UnbalancedConditionals);
    }
    Ok(())
}

