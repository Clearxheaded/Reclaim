# Reclaim

[![CI](https://github.com/reclaim-app/reclaim/actions/workflows/ci.yml/badge.svg)](https://github.com/reclaim-app/reclaim/actions/workflows/ci.yml)
[![Release Packaging](https://github.com/reclaim-app/reclaim/actions/workflows/release.yml/badge.svg)](https://github.com/reclaim-app/reclaim/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Reclaim** is a cross-platform, local-first desktop application and CLI tool built in **Rust** and **Tauri 2** for finding duplicate files and performing safe, reviewable, recoverable filesystem cleanup.

Reclaim is engineered from first principles with a non-negotiable directive: **filesystem safety and data integrity take absolute precedence over convenience or raw execution speed.**

---

## Key Features

- **Read-Only Discovery**: Directory traversal and duplicate detection never alter, move, or touch user files.
- **Two-Stage Content Verification**: Fast candidate pre-filtering by file size and streaming 64 KB prefix hashes, followed by full **BLAKE3** cryptographic hashing.
- **Physical Hard-Link Awareness**: Queries physical filesystem file identity (`st_ino`/`st_dev` on Unix; `nFileIndexHigh`/`nFileIndexLow` on Windows) to correctly classify hard links and prevent phantom duplicate reporting (accounting for 0 B reclaimed).
- **Prospective, Immutable Planning**: Proposes quarantine actions with frozen preconditions (file sizes, `FileId`, cryptographic hashes) before any mutation is approved.
- **Durable SQLite State Machine**: Every state transition (`Planned` $\rightarrow$ `Validating` $\rightarrow$ `Executing` $\rightarrow$ `Completed` / `Failed` $\rightarrow$ `Reverted`) is recorded with microsecond timestamps in a local SQLite WAL journal with an append-only audit log.
- **Zero Silent Overwrites**: Low-level mutations use kernel-level atomic non-replacing syscalls (`libc::renameatx_np` with `RENAME_EXCL` on macOS; `libc::renameat2` with `RENAME_NOREPLACE` on Linux; `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on Windows). Never relies on check-then-rename races.
- **Crash Reconciliation & Auto-Recovery**: If interrupted mid-operation (power cut, SIGKILL, kernel panic), Reclaim inspects the journal and physical disk state on startup, resolves dangling in-flight mutations, and prevents partially executed operations from being blindly repeated.
- **Conditional Undo Engine**: Completed quarantine actions can be safely reversed. If a destination path has been re-occupied or the preserved keeper is missing, Reclaim halts and logs a transparent conflict rather than overwriting data.
- **Native Desktop Interface**: Built with Tauri 2, React, and TypeScript. Native typography, high-density file inspection, live progress event streaming, side-by-side conflict cards, and Finder/Explorer integration.

---

## Architectural Layers

Reclaim strictly separates concerns into 6 decoupled architectural layers:

```text
[ Layer F: Desktop Interface (Tauri 2 + React + TypeScript) ]
                          |
[ Layer D: Safe Executor ] <---> [ Layer E: Recovery & Conditional Undo ]
           |                                  |
           +----------------+-----------------+
                            |
           [ Layer C: Prospective Cleanup Planner ]
                            |
           [ Layer B: Exact Duplicate Detection ]
                            |
           [ Layer A: Read-Only Filesystem Scanner ]
                            |
              [ Physical Operating System / Storage ]
     macOS (APFS/HFS+) | Windows (NTFS) | Linux (ext4/btrfs)
```

1. **Layer A — Read-Only Filesystem Scanner (`reclaim-core::scanner`)**: Safe directory walker collecting file metadata, file size, timestamps, and physical file identity (`FileId`).
2. **Layer B — Exact Duplicate Detection (`reclaim-core::duplicates`)**: Size grouping, 64 KB prefix filtering, streaming BLAKE3 hashing, and hard-link alias deduplication.
3. **Layer C — Prospective Cleanup Planner (`reclaim-core::planner`)**: Deterministic keeper selection policies (`ShortestPath`, `PreferredDirectory`, `NewestModified`, `OldestModified`), prospective relative quarantine path generation, and TOCTOU precondition capture.
4. **Layer D — Safe Executor (`reclaim-storage::executor`)**: Revalidates preconditions immediately before execution, enforces volume/filesystem boundaries, writes intent to SQLite journal before mutation, and executes kernel atomic non-replacing moves.
5. **Layer E — Recovery & Conditional Undo (`reclaim-storage::recovery` & `reclaim-storage::undo`)**: Reconciles interrupted sessions on startup against physical storage; provides conditional reversal with destination vacancy checks and cryptographic verification.
6. **Layer F — Desktop Interface (`apps/desktop`)**: Tauri 2 IPC commands, React state machine, real-time progress monitors, duplicate inspector, and explicit approval review modals.

---

## The 10 Filesystem Safety Invariants

Reclaim operates under ten strictly verified safety invariants:

| # | Invariant | Enforcement Mechanism |
|---|---|---|
| **1** | **No Mutation Without Approval** | Scanner, hasher, and planner are purely read-only; mutations require explicit user approval. |
| **2** | **No Silent Overwrites** | Kernel-level atomic non-replace syscalls (`RENAME_EXCL`, `RENAME_NOREPLACE`). |
| **3** | **No Unexpected Path Traversal** | Symlink-aware metadata (`symlink_metadata`); symbolic links and junctions are rejected during mutation. |
| **4** | **No Trust in Stale Metadata** | Preconditions (file size, `FileId`, BLAKE3 hash) re-checked immediately before syscall execution. |
| **5** | **No Unsafe Deletion** | Reversible quarantine moves only; permanent deletion is strictly out of scope. |
| **6** | **Preserve Verified Content** | Candidate duplicate is never quarantined unless the keeper exists, is accessible, and content hash matches. |
| **7** | **Honest Undo Guarantees** | Reversal is conditional; if a destination path is occupied, Reclaim fails closed and retains the quarantine copy. |
| **8** | **Crash Recovery** | SQLite intent is committed before mutation; startup reconciliation inspects disk state after unexpected termination. |
| **9** | **Honest Durability Claims** | SQLite WAL mode durability; clear distinction between filesystem state and database transactions. |
| **10** | **Fail Closed** | If an OS capability is unsupported or an access check fails, Reclaim aborts rather than falling back to weaker checks. |

---

## Verification & Test Suite

The workspace is protected by a deterministic test suite containing **100 automated unit, integration, recovery, and fault-injection tests**:

```bash
cargo test --workspace
```

### Test Coverage Highlights
- **Scanner Tests (10 tests)**: Nested directories, unicode filenames, symlink defense, permission-denied directories, cancellation.
- **Duplicate Detection Tests (10 tests)**: Size grouping, prefix elimination, hard-link alias deduplication (0 B accounted), large file streaming.
- **Planner Tests (11 tests)**: Keeper policy determinism, relative path quarantine structure, precondition capture, collision detection.
- **Mutation & Atomic Move Tests (10 tests)**: `atomic_move_no_replace`, destination collisions, volume boundary verification, symlink replacement defense.
- **Durable Journal Tests (12 tests)**: SQLite WAL mode, foreign key cascades, transactional rollbacks, state machine transitions, audit trail logs.
- **Startup Recovery Tests (6 tests)**: Clean journal inspection, in-flight action reconciliation, physical disk inspection.
- **Conditional Undo Tests (8 tests)**: Happy-path restoration, occupied destination conflicts, missing keeper conflicts, tampered quarantine file rejection.
- **CLI Integration Tests (3 tests)**: JSON output formatting, CLI execution, CLI conditional undo.
- **Fault-Injection & Reliability Tests (15 tests)**: Process crash before syscall, crash after syscall before commit, batch crash reconciliation, read-only file/directory permission drops, unwritable quarantine paths, disk corruption recovery, double-undo idempotency.

---

## Continuous Integration & Release Packaging

Dual-platform CI and automated release packaging workflows are configured in `.github/workflows`:

- **`ci.yml`**: Matrix testing across **macOS**, **Windows**, and **Linux**:
  - `cargo fmt --all -- --check`
  - `cargo check --workspace --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace --verbose`
  - Desktop frontend compilation (`tsc && vite build`)
- **`release.yml`**: Native desktop packaging via Tauri 2:
  - **macOS**: Native `.dmg` bundle with custom layout and universal binary support.
  - **Windows**: Native `.msi` (WiX) and `.exe` (NSIS) installers with currentUser installation mode.

---

## Building from Source

### Prerequisites
- **Rust** (stable toolchain: `rustup update stable`)
- **Node.js** (v20 or v22) & **npm**

### CLI Tool
```bash
# Build the Reclaim CLI
cargo build --release --bin reclaim

# Scan a directory and find duplicates (read-only)
./target/release/reclaim scan /path/to/folder

# Review past operations and sessions
./target/release/reclaim history
```

### Desktop Application
```bash
# Install frontend dependencies
cd apps/desktop
npm install

# Run in development mode (hot-reloading Vite + native Tauri window)
npm run tauri dev

# Build release bundle
npm run tauri build
```

---

## License

Reclaim is licensed under the [MIT License](LICENSE).
