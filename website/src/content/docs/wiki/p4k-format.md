---
title: P4k Format
description: Anatomy of the Star Citizen Data.p4k archive.
---

`Data.p4k` is a Zip64-based archive with CIG extensions. Entries are stored,
deflate-compressed, or ZStandard-compressed (method 100). Encryption is per
entry: an encrypted entry's compressed data is AES-128-CBC encrypted, so
reading it means decrypting first and then decompressing with zstd.

This page is a placeholder — fill in once we have a stable explainer to
publish.
