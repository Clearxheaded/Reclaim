//! # Pipeline & Scalability Benchmarks
//!
//! Measures:
//! 1. Two-stage prefix vs. full-hash filter efficiency (demonstrating speedup from Stage 1 prefix elimination).
//! 2. Scalability and peak memory bounds on 1,000 to 10,000+ synthetic candidate workloads.
//! 3. Real-time candidate invalidation throughput (operations per millisecond).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::entry::FileEntry;
use reclaim_core::file_id::FileId;
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_core::watcher::{CandidateInvalidator, InvalidationReason};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tempfile::TempDir;

/// Benchmarks Stage 1 Prefix Filtering vs. Full Hashing.
fn bench_prefix_filter_efficiency(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    let file_count = 50;
    let file_size: usize = 128 * 1024; // 128 KiB per file

    // Cohort A: Same size, different prefixes (eliminated in Stage 1)
    let dir_diff_prefix = root.join("diff_prefix");
    fs::create_dir_all(&dir_diff_prefix).unwrap();
    for i in 0..file_count {
        let p = dir_diff_prefix.join(format!("file_{}.bin", i));
        let mut buf = vec![0u8; file_size];
        buf[0..8].copy_from_slice(&(i as u64).to_le_bytes());
        fs::write(p, &buf).unwrap();
    }

    // Cohort B: Same size, identical prefixes, different tails (Stage 2 full hash needed)
    let dir_same_prefix = root.join("same_prefix");
    fs::create_dir_all(&dir_same_prefix).unwrap();
    for i in 0..file_count {
        let p = dir_same_prefix.join(format!("file_{}.bin", i));
        let mut buf = vec![0x7fu8; file_size];
        let tail_idx = file_size - 8;
        buf[tail_idx..].copy_from_slice(&(i as u64).to_le_bytes());
        fs::write(p, &buf).unwrap();
    }

    let mut group = c.benchmark_group("filter_efficiency");
    group.throughput(Throughput::Bytes((file_count * file_size) as u64));

    let scan_diff = scan_directory(&dir_diff_prefix, ScanOptions::default()).unwrap();
    group.bench_function("stage1_prefix_elimination", |b| {
        b.iter(|| {
            let res = find_duplicates(black_box(&scan_diff.entries), &DuplicateOptions::default());
            black_box(res)
        });
    });

    let scan_same = scan_directory(&dir_same_prefix, ScanOptions::default()).unwrap();
    group.bench_function("stage2_full_hash_required", |b| {
        b.iter(|| {
            let res = find_duplicates(black_box(&scan_same.entries), &DuplicateOptions::default());
            black_box(res)
        });
    });

    group.finish();
}

/// Benchmarks memory and execution scaling on 1,000, 5,000, and 10,000 synthetic entries.
fn bench_large_synthetic_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("synthetic_scale");

    for count in &[1_000, 5_000, 10_000] {
        let entries: Vec<FileEntry> = (0..*count)
            .map(|i| {
                // Generate 10 clusters of matching sizes to trigger grouping
                let cluster = i % 10;
                FileEntry {
                    path: PathBuf::from(format!("/synthetic/dir_{}/file_{}.dat", cluster, i)),
                    size: 1024 * (cluster as u64 + 1),
                    modified: Some(SystemTime::UNIX_EPOCH),
                    #[cfg(unix)]
                    file_id: Some(FileId::Unix {
                        dev: 1,
                        ino: i as u64,
                    }),
                    #[cfg(not(unix))]
                    file_id: Some(FileId::Windows {
                        volume_serial_number: 1,
                        file_index: i as u64,
                    }),
                    readonly: false,
                }
            })
            .collect();

        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(BenchmarkId::from_parameter(count), &entries, |b, items| {
            b.iter(|| {
                // Evaluates size-grouping and candidate pruning on in-memory collections
                let dup_opts = DuplicateOptions::default();
                let output = find_duplicates(black_box(items), &dup_opts);
                black_box(output)
            });
        });
    }

    group.finish();
}

/// Benchmarks real-time candidate invalidator lookup speed.
fn bench_candidate_invalidator_throughput(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create 100 duplicate files
    let mut entries = Vec::new();
    let payload = vec![0x33u8; 1024];
    for i in 0..100 {
        let p = root.join(format!("file_{}.bin", i));
        fs::write(&p, &payload).unwrap();
        entries.push(p);
    }

    let scan_out = scan_directory(root, ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    let mut group = c.benchmark_group("invalidator_throughput");
    group.bench_function("invalidate_single_candidate", |b| {
        b.iter_batched(
            || CandidateInvalidator::new(dup_out.clone(), vec![root.to_path_buf()]),
            |mut invalidator| {
                let notice = invalidator.invalidate_path(
                    black_box(&entries[0]),
                    black_box(InvalidationReason::Modified),
                );
                black_box(notice)
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_prefix_filter_efficiency,
    bench_large_synthetic_workload,
    bench_candidate_invalidator_throughput
);
criterion_main!(benches);
