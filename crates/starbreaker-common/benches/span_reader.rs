use criterion::{Criterion, black_box, criterion_group, criterion_main};
use starbreaker_common::SpanReader;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

// ── Simulated chunk structures (mirrors real IVO/CrCh layouts) ──────────────

#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable, IntoBytes, PartialEq)]
#[repr(C, packed)]
struct ChunkTableEntry {
    chunk_type: u32,
    version: u32,
    offset: u64,
}

#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable, IntoBytes, PartialEq)]
#[repr(C, packed)]
struct Vertex {
    position: [f32; 3],
    normal: [u8; 4],
    uv: [f32; 2],
}

#[derive(Debug, Clone, Copy, FromBytes, KnownLayout, Immutable, IntoBytes, PartialEq)]
#[repr(C, packed)]
struct MeshHeader {
    flags: u32,
    vertex_count: u32,
    index_count: u32,
    material_id: u32,
    bbox_min: [f32; 3],
    bbox_max: [f32; 3],
}

const MAGIC: u32 = 0x6F766923;
const VERSION: u32 = 0x900;

// ── Data builders ───────────────────────────────────────────────────────────

/// Build a fake IVO-like buffer: magic, version, chunk_count, table entries,
/// then for each chunk: MeshHeader + vertex slice + index slice.
fn build_ivo_buffer(num_chunks: usize, verts_per_chunk: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let push = |buf: &mut Vec<u8>, bytes: &[u8]| buf.extend_from_slice(bytes);

    // Header
    push(&mut buf, &MAGIC.to_le_bytes());
    push(&mut buf, &VERSION.to_le_bytes());
    push(&mut buf, &(num_chunks as u32).to_le_bytes());
    push(&mut buf, &0u32.to_le_bytes()); // table offset placeholder

    // Chunk table
    let table_start = buf.len();
    let entry_size = size_of::<ChunkTableEntry>();
    let data_start = table_start + entry_size * num_chunks;

    let chunk_data_size =
        size_of::<MeshHeader>() + size_of::<Vertex>() * verts_per_chunk + 2 * verts_per_chunk * 3;

    for i in 0..num_chunks {
        let offset = (data_start + i * chunk_data_size) as u64;
        let entry = ChunkTableEntry {
            chunk_type: 0x1001 + i as u32,
            version: 1,
            offset,
        };
        push(&mut buf, entry.as_bytes());
    }

    // Chunk data
    for _ in 0..num_chunks {
        let header = MeshHeader {
            flags: 0x01,
            vertex_count: verts_per_chunk as u32,
            index_count: (verts_per_chunk * 3) as u32,
            material_id: 42,
            bbox_min: [-1.0, -1.0, -1.0],
            bbox_max: [1.0, 1.0, 1.0],
        };
        push(&mut buf, header.as_bytes());

        for v in 0..verts_per_chunk {
            let vert = Vertex {
                position: [v as f32 * 0.1, v as f32 * 0.2, v as f32 * 0.3],
                normal: [127, 127, 255, 0],
                uv: [v as f32 / verts_per_chunk as f32, 0.5],
            };
            push(&mut buf, vert.as_bytes());
        }

        for idx in 0..(verts_per_chunk * 3) {
            push(&mut buf, &(idx as u16).to_le_bytes());
        }
    }

    buf
}

// ── Benchmarks ──────────────────────────────────────────────────────────────

/// Parse the IVO header + chunk table (primitives + read_slice of structs).
fn parse_header_and_table(data: &[u8]) -> (u32, u32, &[ChunkTableEntry]) {
    let mut r = SpanReader::new(data);
    let magic = r.read_u32().unwrap();
    let version = r.read_u32().unwrap();
    let chunk_count = r.read_u32().unwrap();
    let _table_offset = r.read_u32().unwrap();
    let entries = r.read_slice::<ChunkTableEntry>(chunk_count as usize).unwrap();
    (magic, version, entries)
}

/// For each chunk, read MeshHeader + vertex slice + index slice.
fn parse_all_chunks(data: &[u8], entries: &[ChunkTableEntry], verts_per_chunk: usize) -> f32 {
    let mut sum = 0.0f32;
    for entry in entries {
        let mut r = SpanReader::new_at(data, entry.offset as usize);
        let header = r.read_type::<MeshHeader>().unwrap();
        let verts = r.read_slice::<Vertex>(header.vertex_count as usize).unwrap();
        let _indices = r.read_slice::<u16>(header.index_count as usize).unwrap();

        // Accumulate to prevent dead-code elimination
        for v in verts.iter().take(verts_per_chunk.min(4)) {
            sum += v.position[0] + v.position[1] + v.position[2];
        }
        let _ = header.flags;
    }
    sum
}

/// Sequential primitive reads — simulates reading many small fields.
fn sequential_primitives(data: &[u8]) -> u64 {
    let mut r = SpanReader::new(data);
    let mut acc: u64 = 0;
    while r.remaining() >= 4 {
        acc = acc.wrapping_add(r.read_u32().unwrap() as u64);
    }
    acc
}

/// Mixed reads — interleave read_type, primitives, read_bytes, advance.
fn mixed_reads(data: &[u8]) -> u64 {
    let mut r = SpanReader::new(data);
    let mut acc: u64 = 0;
    while r.remaining() >= 28 {
        acc = acc.wrapping_add(r.read_u32().unwrap() as u64);
        acc = acc.wrapping_add(r.read_u16().unwrap() as u64);
        acc = acc.wrapping_add(r.read_u8().unwrap() as u64);
        r.advance(1).unwrap();
        let chunk = r.read_bytes(8).unwrap();
        acc = acc.wrapping_add(chunk[0] as u64);
        let v = r.read_type::<Vertex>().unwrap();
        acc = acc.wrapping_add(v.normal[0] as u64);
    }
    acc
}

/// split_off pattern — parse sub-sections.
fn split_off_pattern(data: &[u8], section_size: usize) -> u64 {
    let mut r = SpanReader::new(data);
    let mut acc: u64 = 0;
    while r.remaining() >= section_size {
        let mut sub = r.split_off(section_size).unwrap();
        acc = acc.wrapping_add(sub.read_u32().unwrap() as u64);
        acc = acc.wrapping_add(sub.read_u16().unwrap() as u64);
    }
    acc
}

fn bench_span_reader(c: &mut Criterion) {
    let small_buf = build_ivo_buffer(8, 64);
    let large_buf = build_ivo_buffer(32, 1024);
    // 1MB of random-ish data for raw sequential reads
    let raw_buf: Vec<u8> = (0..1_048_576u32).map(|i| (i.wrapping_mul(2654435761)) as u8).collect();

    c.bench_function("header_table_8x64", |b| {
        b.iter(|| parse_header_and_table(black_box(&small_buf)))
    });

    c.bench_function("header_table_32x1024", |b| {
        b.iter(|| parse_header_and_table(black_box(&large_buf)))
    });

    // Parse chunk table then all chunk bodies
    let (_, _, entries_small) = parse_header_and_table(&small_buf);
    c.bench_function("full_parse_8x64", |b| {
        b.iter(|| parse_all_chunks(black_box(&small_buf), entries_small, 64))
    });

    let (_, _, entries_large) = parse_header_and_table(&large_buf);
    c.bench_function("full_parse_32x1024", |b| {
        b.iter(|| parse_all_chunks(black_box(&large_buf), entries_large, 1024))
    });

    c.bench_function("sequential_u32_1mb", |b| {
        b.iter(|| sequential_primitives(black_box(&raw_buf)))
    });

    c.bench_function("mixed_reads_1mb", |b| {
        b.iter(|| mixed_reads(black_box(&raw_buf)))
    });

    c.bench_function("split_off_64b_1mb", |b| {
        b.iter(|| split_off_pattern(black_box(&raw_buf), 64))
    });
}

criterion_group!(benches, bench_span_reader);
criterion_main!(benches);
