## Description
Summary of the changes introduced in this pull request and the rationale behind them.

## Safety & Invariant Checklist
- [ ] Scanning and duplicate detection remain read-only.
- [ ] No silent overwrites: mutations use atomic non-replace primitives (`renameatx_np(RENAME_EXCL)` on macOS, `MoveFileExW` on Windows).
- [ ] Preconditions are verified immediately before syscalls (TOCTOU defense).
- [ ] All action transitions are durably recorded in the SQLite action journal before mutation.
- [ ] No regression in undo guarantees: occupied destinations fail closed with explicit conflicts.

## Verification
- [ ] `cargo fmt --all -- --check` passes with zero differences.
- [ ] `cargo check --workspace --all-targets` compiles cleanly.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` reports zero warnings.
- [ ] `cargo test --workspace` passes all tests.
- [ ] Added automated tests for new functionality or edge cases.

