# Shader keyword grammar

`cache-<version>.txt` maps `FXB1` keyword token ids to their text for shader bins
(`.cfxb` / `.cfib` in `Engine\ShaderCache_Vulkan.pak`) whose header carries that
cache version word. It is a format table — keyword spellings and their ids — not
game content. Decoded shader source is game content and is never written into the
repository.

## Why exactly one, keyed by cache version

Keyword ids are positions in the engine's parser token list, so one inserted
keyword shifts every id after it. Across 95 client builds (3.0 to 4.10.2) there
were six distinct grammars. Every grammar change shifted dozens to hundreds of ids,
and every one coincided with a change of the cache version word. A stale grammar
still resolves almost every id, so it decodes into plausible but wrong text, and no
structural check reliably catches that. The crate therefore ships only the latest
known grammar and refuses any other cache version
(`ShaderCacheError::UnsupportedCacheVersion`).

## Provenance

`cache-00b222be.txt`: Star Citizen 4.10.2 client, P4 change 12913470. It is the
table the client's shader parser registers at start-up (`CParserBin::Init`), id 1 =
`#include`. Id 416 is absent on purpose: it is the engine's technique-range end
marker, which is never registered.

## Replacing it for a new game build

When a new build's shader bins report a different cache version, the decoder
refuses them. To move forward, a maintainer recovers that build's table locally,
outside the repository, from that build's own client. Then they:

1. replace this file with `cache-<newversion>.txt` (same format, same header);
2. update `SUPPORTED_CACHE_VERSION` and the `include_str!` path in `src/grammar.rs`;
3. run `cargo test -p starbreaker-shadercache`, then decode every bin of the new
   build with `hlsl::decode` and confirm none is refused.

Keep only one grammar. The crate supports the current build, not history.
