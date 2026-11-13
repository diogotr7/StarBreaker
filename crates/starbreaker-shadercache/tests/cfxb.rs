//! Shader-bin (`FXB1`) parsing, rendering, validation and the cache-version-keyed
//! grammar, on synthetic token streams.
//!
//! Streams that use the shipped grammar look their keyword ids up by text, so no
//! id value is copied into the tests.

use std::collections::HashMap;

use starbreaker_shadercache::hlsl::{self, DecodeIssue};
use starbreaker_shadercache::{
    Grammar, SUPPORTED_CACHE_VERSION, ShaderCacheError, cfxb, grammar,
};

/// Build a minimal `FXB1` file with explicit tokens and string table.
fn synthetic_bin(version: u32, tokens: &[u32], table: &[(u32, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"FXB1");
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let string_table_offset = 28 + tokens.len() * 4;
    out.extend_from_slice(&(string_table_offset as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(tokens.len() as u32).to_le_bytes());
    out.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    for t in tokens {
        out.extend_from_slice(&t.to_le_bytes());
    }
    for (id, text) in table {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }
    let end = out.len() as u32;
    out[16..20].copy_from_slice(&end.to_le_bytes());
    out
}

// Toy grammar: 1 = `#include`, 2 = `float4`, 3 = `(`, 4 = `)`, 5 = `{`, 6 = `}`, 7 = `;`,
// 8 = `#if`, 9 = `#else`, 10 = `#endif`
const TOKENS: &[u32] = &[1, 0x1234_5678, 0, 2, 0xAAAA_0001, 3, 4, 7, 0, 5, 6, 0];

fn table() -> HashMap<u32, String> {
    [(0x1234_5678u32, "common".to_owned()), (0xAAAA_0001, "x".to_owned())]
        .into_iter()
        .collect()
}

fn toy_grammar() -> Grammar {
    Grammar::parse(
        "1\t#include\n2\tfloat4\n3\t(\n4\t)\n5\t{\n6\t}\n7\t;\n8\t#if\n9\t#else\n10\t#endif\n",
    )
    .expect("grammar")
}

/// Id of a keyword in the shipped grammar, found by its text.
fn shipped_id(text: &str) -> u32 {
    let g = grammar::shipped();
    g.ids()
        .into_iter()
        .find(|&id| g.get(id) == Some(text))
        .unwrap_or_else(|| panic!("shipped grammar has no {text:?}"))
}

#[test]
fn parses_tokens_and_string_table() {
    let bin = synthetic_bin(0x0102_0304, TOKENS, &[(0x1234_5678, "common")]);
    let parsed = cfxb::parse(&bin).expect("parses");

    assert_eq!(parsed.version, 0x0102_0304);
    assert_eq!(parsed.tokens, TOKENS);
    assert_eq!(parsed.table.get(&0x1234_5678).map(String::as_str), Some("common"));
}

#[test]
fn renders_and_validates() {
    let grammar = toy_grammar();
    assert!(hlsl::validate(TOKENS, &grammar, &table()).is_ok());
    let text = hlsl::render(TOKENS, &grammar, &table());
    assert!(text.contains("#include common"), "got: {text}");
    assert!(text.contains("float4 x"), "got: {text}");
}

#[test]
fn rejects_unresolved_ids() {
    // Every id shifted by one: `#include` lands on 2, and 11 resolves to nothing.
    let shifted = Grammar::parse("2\t#include\n3\tfloat4\n4\t(\n5\t)\n6\t{\n7\t}\n8\t;\n")
        .expect("grammar");
    let err = hlsl::validate(TOKENS, &shifted, &table()).expect_err("must fail");
    assert!(matches!(err, DecodeIssue::Unresolved(_)), "got {err:?}");
}

#[test]
fn rejects_unbalanced_conditionals() {
    let tokens = [8, 0xAAAA_0001, 0, 2, 0xAAAA_0001, 7, 0];
    let err = hlsl::validate(&tokens, &toy_grammar(), &table()).expect_err("must fail");
    assert_eq!(err, DecodeIssue::UnbalancedConditionals);
}

#[test]
fn include_name_may_be_a_keyword() {
    // An include file named like a keyword (e.g. `ProjectedEffect`) is tokenized
    // as that keyword; the stream is still well formed.
    let tokens = [1, 2, 0, 5, 6, 0];
    assert_eq!(hlsl::validate(&tokens, &toy_grammar(), &table()), Ok(()));
}

#[test]
fn preprocessor_branches_may_leave_braces_unbalanced() {
    // `#if x {  #else {  #endif  }` — each branch opens one brace.
    let tokens = [8, 0xAAAA_0001, 0, 5, 0, 9, 0, 5, 0, 10, 0, 6, 0];
    assert_eq!(hlsl::validate(&tokens, &toy_grammar(), &table()), Ok(()));
}

#[test]
fn shipped_grammar_is_complete_and_keyed() {
    let g = grammar::for_cache_version(SUPPORTED_CACHE_VERSION).expect("supported version");
    assert_eq!(g.get(1), Some("#include"));
    let ids = g.ids();
    let max = *ids.last().expect("non-empty");
    // Dense from 1, except the engine's unregistered technique-range end marker.
    assert_eq!(max as usize - ids.len(), 1, "exactly one empty slot expected");
}

#[test]
fn other_cache_versions_are_refused() {
    let other = SUPPORTED_CACHE_VERSION ^ 0x0100_0000;
    let err = grammar::for_cache_version(other).expect_err("must refuse");
    assert!(
        matches!(
            err,
            ShaderCacheError::UnsupportedCacheVersion { found, supported }
                if found == other && supported == SUPPORTED_CACHE_VERSION
        ),
        "got {err:?}"
    );

    let bin = cfxb::parse(&synthetic_bin(other, TOKENS, &[])).expect("parses");
    assert!(matches!(
        hlsl::decode(&bin),
        Err(ShaderCacheError::UnsupportedCacheVersion { .. })
    ));
}

#[test]
fn decode_uses_the_shipped_grammar_for_its_version() {
    let name = 0x7000_0001;
    let tokens = [
        shipped_id("#include"),
        name,
        0,
        shipped_id("float4"),
        name,
        shipped_id(";"),
        0,
    ];
    let bin = cfxb::parse(&synthetic_bin(SUPPORTED_CACHE_VERSION, &tokens, &[(name, "Common")]))
        .expect("parses");
    let text = hlsl::decode(&bin).expect("decodes");
    assert!(text.contains("#include Common"), "got: {text}");
    assert!(text.contains("float4 Common;"), "got: {text}");
}

#[test]
fn decode_refuses_an_untrusted_stream() {
    let tokens = [shipped_id("#if"), 0x7000_0001, 0];
    let bin = cfxb::parse(&synthetic_bin(
        SUPPORTED_CACHE_VERSION,
        &tokens,
        &[(0x7000_0001, "X")],
    ))
    .expect("parses");
    assert!(matches!(
        hlsl::decode(&bin),
        Err(ShaderCacheError::UntrustedDecode(DecodeIssue::UnbalancedConditionals))
    ));
}
