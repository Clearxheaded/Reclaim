//! # Cryptographic BLAKE3 Hashing Micro-Benchmarks
//!
//! Measures streaming cryptographic BLAKE3 hashing throughput across various
//! payload sizes (64 KiB, 1 MiB, 10 MiB, 50 MiB), reporting throughput in GB/s.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use reclaim_core::mutation::compute_file_blake3_hash;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

fn bench_blake3_in_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_in_memory");

    for size in &[64 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let buffer = vec![0x5au8; *size];
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let mut hasher = blake3::Hasher::new();
                hasher.update(black_box(&buffer));
                black_box(hasher.finalize())
            });
        });
    }

    group.finish();
}

fn bench_blake3_streaming_filesystem(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let mut group = c.benchmark_group("blake3_streaming_disk");

    for size in &[64 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let file_path = temp_dir.path().join(format!("payload_{}.bin", size));
        let buffer = vec![0xa5u8; *size];
        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(&buffer).unwrap();
            f.sync_all().unwrap();
        }

        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &file_path, |b, path| {
            b.iter(|| {
                let hash = compute_file_blake3_hash(black_box(path)).unwrap();
                black_box(hash)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_blake3_in_memory,
    bench_blake3_streaming_filesystem
);
criterion_main!(benches);
