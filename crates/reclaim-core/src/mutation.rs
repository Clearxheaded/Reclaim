//! # Filesystem Mutation & Precondition Verification Module
//!
//! Provides platform-specific, atomic, non-replacing file movement and rigorous TOCTOU
//! precondition revalidation for Milestone 5 (Safe Quarantine Execution).
//!
//! ## Core Safety Invariants
//! - **No Silent Overwrites**: Filesystem mutation uses atomic no-replace primitives at the OS
//!   syscall level (`libc::renameatx_np` with `RENAME_EXCL` on macOS; `libc::renameat2` with
//!   `RENAME_NOREPLACE` on Linux; `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on Windows).
//!   Under NO circumstance does Reclaim fall back to ordinary check-then-rename logic.
//! - **Fail Closed**: If an atomic non-replacing primitive is unsupported on an operating system
//!   or environment, execution halts immediately with an explicit error.
//! - **Defense Against Stale Metadata (TOCTOU)**: Before any mutation is initiated, the candidate
//!   source file's size, cryptographic BLAKE3 content hash, and physical `FileId` are recomputed
//!   and compared against frozen plan preconditions.
//! - **Preserve Verified Content**: The preserved keeper file must be re-verified to still exist,
//!   be a regular file, and match expected size, `FileId`, and content hash before any duplicate
//!   can be moved to quarantine.
//! - **No Unexpected Path Traversal / Symlink Defense**: Source and keeper files are checked using
//!   symlink-aware metadata (`symlink_metadata`). If a file was replaced with a symlink or redirect,
//!   the action fails closed.
//! - **Same-Device / Volume Boundary Verification**: Source and quarantine destination must reside
//!   on the same filesystem/device (`st_dev` on Unix). Cross-device moves degrade to copy-then-delete
//!   and are strictly rejected.

use crate::file_id::FileId;
use crate::planner::QuarantineAction;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

pub mod cow;
pub use cow::{
    deduplicate_via_cow, default_cow_deduplicator, ApfsCloneDeduplicator, CowDeduplicationResult,
    CowError, CowStrategy, ExtentDeduplicator, LinuxFicloneDeduplicator, SyntheticCowDeduplicator,
    WindowsBlockCloneDeduplicator,
};

/// 64 KB buffer size for streaming cryptographic BLAKE3 hashing during precondition verification.
const HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Errors arising from low-level filesystem mutation operations.
#[derive(Debug)]
pub enum MutationError {
    /// The destination path already exists. Atomic non-replacing move refused overwrite.
    DestinationAlreadyExists(PathBuf),
    /// Source and destination reside on different filesystem devices or volumes.
    CrossDeviceMoveUnsupported {
        source: PathBuf,
        destination: PathBuf,
    },
    /// A path is invalid (contains null bytes, missing parent directory, etc.).
    InvalidPath(String),
    /// The platform does not support atomic no-replace semantics; failing closed.
    UnsupportedPlatform(String),
    /// I/O error inspecting source file metadata.
    SourceIo { path: PathBuf, error: io::Error },
    /// I/O error inspecting destination parent directory metadata.
    DestinationParentIo { path: PathBuf, error: io::Error },
    /// General underlying I/O error from the operating system.
    Io(io::Error),
}

impl fmt::Display for MutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MutationError::DestinationAlreadyExists(path) => {
                write!(
                    f,
                    "Destination already exists; refusing to overwrite: '{}'",
                    path.display()
                )
            }
            MutationError::CrossDeviceMoveUnsupported {
                source,
                destination,
            } => {
                write!(
                    f,
                    "Cross-device move unsupported: source '{}' and destination '{}' reside on different filesystems",
                    source.display(),
                    destination.display()
                )
            }
            MutationError::InvalidPath(msg) => write!(f, "Invalid path: {}", msg),
            MutationError::UnsupportedPlatform(msg) => write!(f, "Unsupported platform: {}", msg),
            MutationError::SourceIo { path, error } => {
                write!(
                    f,
                    "Failed to read source file metadata for '{}': {}",
                    path.display(),
                    error
                )
            }
            MutationError::DestinationParentIo { path, error } => {
                write!(
                    f,
                    "Failed to read destination parent directory metadata for '{}': {}",
                    path.display(),
                    error
                )
            }
            MutationError::Io(err) => write!(f, "Filesystem I/O error: {}", err),
        }
    }
}

impl std::error::Error for MutationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MutationError::Io(err) => Some(err),
            MutationError::SourceIo { error, .. } => Some(error),
            MutationError::DestinationParentIo { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Errors detected during TOCTOU precondition revalidation prior to mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreconditionError {
    /// The source duplicate file was deleted or cannot be found.
    SourceMissing(PathBuf),
    /// The source file is a symbolic link or redirect; refusing to follow.
    SourceIsSymlink(PathBuf),
    /// The source file is not a regular file (e.g. socket, FIFO, device).
    SourceNotRegularFile(PathBuf),
    /// Source file size changed between planning and execution.
    SourceSizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    /// Source file content hash changed between planning and execution.
    SourceHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// Source physical file identity changed between planning and execution.
    SourceFileIdMismatch {
        path: PathBuf,
        expected: FileId,
        actual: FileId,
    },
    /// The preserved keeper file was deleted or cannot be found.
    KeeperMissing(PathBuf),
    /// The preserved keeper file is a symbolic link; refusing to accept.
    KeeperIsSymlink(PathBuf),
    /// The preserved keeper file is not a regular file.
    KeeperNotRegularFile(PathBuf),
    /// Preserved keeper file size changed.
    KeeperSizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    /// Preserved keeper file content hash changed.
    KeeperHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// Preserved keeper physical identity changed.
    KeeperFileIdMismatch {
        path: PathBuf,
        expected: FileId,
        actual: FileId,
    },
    /// The target quarantine destination file already exists.
    DestinationAlreadyExists(PathBuf),
    /// The source and quarantine destination are on different filesystem devices.
    CrossDeviceMove {
        source: PathBuf,
        destination: PathBuf,
    },
    /// Filesystem I/O error occurred during validation.
    Io { path: PathBuf, message: String },
}

impl fmt::Display for PreconditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreconditionError::SourceMissing(path) => {
                write!(f, "Source duplicate file missing: '{}'", path.display())
            }
            PreconditionError::SourceIsSymlink(path) => {
                write!(
                    f,
                    "Source path is a symlink (unexpected redirect): '{}'",
                    path.display()
                )
            }
            PreconditionError::SourceNotRegularFile(path) => {
                write!(f, "Source is not a regular file: '{}'", path.display())
            }
            PreconditionError::SourceSizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Source file size mismatch for '{}': expected {} bytes, actual {} bytes",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::SourceHashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Source content hash mismatch for '{}': expected {}, actual {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::SourceFileIdMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Source file identity (FileId) changed for '{}': expected {}, actual {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::KeeperMissing(path) => {
                write!(f, "Protected keeper file missing: '{}'", path.display())
            }
            PreconditionError::KeeperIsSymlink(path) => {
                write!(
                    f,
                    "Protected keeper path is a symlink: '{}'",
                    path.display()
                )
            }
            PreconditionError::KeeperNotRegularFile(path) => {
                write!(
                    f,
                    "Protected keeper is not a regular file: '{}'",
                    path.display()
                )
            }
            PreconditionError::KeeperSizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Protected keeper size mismatch for '{}': expected {} bytes, actual {} bytes",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::KeeperHashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Protected keeper content hash mismatch for '{}': expected {}, actual {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::KeeperFileIdMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Protected keeper identity (FileId) changed for '{}': expected {}, actual {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            PreconditionError::DestinationAlreadyExists(path) => {
                write!(
                    f,
                    "Quarantine destination path already exists: '{}'",
                    path.display()
                )
            }
            PreconditionError::CrossDeviceMove {
                source,
                destination,
            } => {
                write!(
                    f,
                    "Source '{}' and quarantine destination '{}' are on different filesystem volumes",
                    source.display(),
                    destination.display()
                )
            }
            PreconditionError::Io { path, message } => {
                write!(f, "I/O error verifying '{}': {}", path.display(), message)
            }
        }
    }
}

impl std::error::Error for PreconditionError {}

/// Computes the cryptographic BLAKE3 content hash of a file by streaming its bytes.
pub fn compute_file_blake3_hash(path: &Path) -> Result<String, io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(HASH_BUFFER_SIZE, file);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; HASH_BUFFER_SIZE];

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Atomically moves a file from `source` to `destination`, guaranteeing that `destination`
/// will NOT be overwritten if it already exists.
///
/// ## Operating System Implementations
/// - **macOS (Darwin)**: Uses `libc::renameatx_np(libc::AT_FDCWD, src, libc::AT_FDCWD, dst, libc::RENAME_EXCL)`.
///   Fails with `EEXIST` if destination exists.
/// - **Linux**: Uses `libc::renameat2(libc::AT_FDCWD, src, libc::AT_FDCWD, dst, libc::RENAME_NOREPLACE)`.
///   Fails with `EEXIST` if destination exists.
/// - **Windows**: Uses Win32 `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
///   Fails with `ERROR_ALREADY_EXISTS` or `ERROR_FILE_EXISTS`.
/// - **Other / Fallback**: Refuses execution and fails closed with `UnsupportedPlatform`.
pub fn atomic_move_no_replace(source: &Path, destination: &Path) -> Result<(), MutationError> {
    #[cfg(target_os = "macos")]
    {
        atomic_move_macos(source, destination)
    }

    #[cfg(target_os = "linux")]
    {
        atomic_move_linux(source, destination)
    }

    #[cfg(windows)]
    {
        atomic_move_windows(source, destination)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (source, destination);
        Err(MutationError::UnsupportedPlatform(
            "Atomic non-replacing move is unsupported on this platform; refusing to fall back to unsafe overwrite"
                .into(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn atomic_move_macos(source: &Path, destination: &Path) -> Result<(), MutationError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src_c = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| MutationError::InvalidPath("Source path contains null byte".into()))?;
    let dst_c = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| MutationError::InvalidPath("Destination path contains null byte".into()))?;

    // Darwin libc renameatx_np with RENAME_EXCL (0x00000004)
    let ret = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            src_c.as_ptr(),
            libc::AT_FDCWD,
            dst_c.as_ptr(),
            libc::RENAME_EXCL,
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EEXIST) => Err(MutationError::DestinationAlreadyExists(
                destination.to_path_buf(),
            )),
            Some(libc::EXDEV) => Err(MutationError::CrossDeviceMoveUnsupported {
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
            }),
            _ => Err(MutationError::Io(err)),
        }
    }
}

#[cfg(target_os = "linux")]
fn atomic_move_linux(source: &Path, destination: &Path) -> Result<(), MutationError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src_c = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| MutationError::InvalidPath("Source path contains null byte".into()))?;
    let dst_c = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| MutationError::InvalidPath("Destination path contains null byte".into()))?;

    // Linux renameat2 with RENAME_NOREPLACE (1)
    let ret = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            src_c.as_ptr(),
            libc::AT_FDCWD,
            dst_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EEXIST) => Err(MutationError::DestinationAlreadyExists(
                destination.to_path_buf(),
            )),
            Some(libc::EXDEV) => Err(MutationError::CrossDeviceMoveUnsupported {
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
            }),
            _ => Err(MutationError::Io(err)),
        }
    }
}

#[cfg(windows)]
fn atomic_move_windows(source: &Path, destination: &Path) -> Result<(), MutationError> {
    use std::os::windows::ffi::OsStrExt;

    let mut src_wide: Vec<u16> = source.as_os_str().encode_wide().collect();
    src_wide.push(0);

    let mut dst_wide: Vec<u16> = destination.as_os_str().encode_wide().collect();
    dst_wide.push(0);

    // MOVEFILE_WRITE_THROUGH forces flush to disk.
    // MOVEFILE_REPLACE_EXISTING (0x1) is deliberately omitted to prevent silent overwrites.
    const MOVEFILE_WRITE_THROUGH: u32 = 0x00000008;

    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    let success =
        unsafe { MoveFileExW(src_wide.as_ptr(), dst_wide.as_ptr(), MOVEFILE_WRITE_THROUGH) };

    if success != 0 {
        Ok(())
    } else {
        let error_code = unsafe { GetLastError() };
        // ERROR_ALREADY_EXISTS = 183, ERROR_FILE_EXISTS = 80
        if error_code == 183 || error_code == 80 {
            Err(MutationError::DestinationAlreadyExists(
                destination.to_path_buf(),
            ))
        } else {
            Err(MutationError::Io(io::Error::from_raw_os_error(
                error_code as i32,
            )))
        }
    }
}

/// Verifies that `source` and `destination` reside on the same filesystem / volume boundary.
///
/// Renames across filesystem devices (`EXDEV`) cannot be atomic because they require copy-and-delete.
/// Resolves the nearest existing ancestor of `destination` to verify volume compatibility
/// without prematurely creating directories on disk.
pub fn verify_same_filesystem(source: &Path, destination: &Path) -> Result<(), MutationError> {
    let src_meta = fs::symlink_metadata(source).map_err(|e| MutationError::SourceIo {
        path: source.to_path_buf(),
        error: e,
    })?;

    // Find the nearest existing ancestor directory of the destination
    let mut current = destination.parent();
    let existing_parent = loop {
        match current {
            Some(p) if p.exists() => break p,
            Some(p) => current = p.parent(),
            None => {
                return Err(MutationError::InvalidPath(
                    "Destination has no existing ancestor directory".into(),
                ));
            }
        }
    };

    let dst_parent_meta =
        fs::metadata(existing_parent).map_err(|e| MutationError::DestinationParentIo {
            path: existing_parent.to_path_buf(),
            error: e,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if src_meta.dev() != dst_parent_meta.dev() {
            return Err(MutationError::CrossDeviceMoveUnsupported {
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let src_vol = src_meta.volume_serial_number();
        let dst_vol = dst_parent_meta.volume_serial_number();
        match (src_vol, dst_vol) {
            (Some(v1), Some(v2)) if v1 != v2 => {
                return Err(MutationError::CrossDeviceMoveUnsupported {
                    source: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                });
            }
            _ => {}
        }
    }

    Ok(())
}

/// Rigorously validates all preconditions for a proposed `QuarantineAction` before execution.
///
/// Defends against Time-of-Check / Time-of-Use (TOCTOU) races:
/// 1. Verifies that the preserved keeper file still exists, is a regular file (not a symlink),
///    and its size, `FileId`, and content hash match expectations.
/// 2. Verifies that the source duplicate file still exists, is a regular file (not a symlink),
///    and its size, `FileId`, and content hash match expectations.
/// 3. Ensures the quarantine parent directory exists (creating it safely if needed).
/// 4. Verifies that source and quarantine destination reside on the same device/volume.
/// 5. Verifies that the quarantine destination path does not already exist.
pub fn validate_action_preconditions(action: &QuarantineAction) -> Result<(), PreconditionError> {
    // -------------------------------------------------------------
    // 1. Revalidate Protected Keeper File
    // -------------------------------------------------------------
    let keeper_meta = match fs::symlink_metadata(&action.keeper_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(PreconditionError::KeeperMissing(action.keeper_path.clone()));
        }
        Err(e) => {
            return Err(PreconditionError::Io {
                path: action.keeper_path.clone(),
                message: e.to_string(),
            });
        }
    };

    if keeper_meta.file_type().is_symlink() {
        return Err(PreconditionError::KeeperIsSymlink(
            action.keeper_path.clone(),
        ));
    }

    if !keeper_meta.is_file() {
        return Err(PreconditionError::KeeperNotRegularFile(
            action.keeper_path.clone(),
        ));
    }

    if keeper_meta.len() != action.preconditions.expected_size {
        return Err(PreconditionError::KeeperSizeMismatch {
            path: action.keeper_path.clone(),
            expected: action.preconditions.expected_size,
            actual: keeper_meta.len(),
        });
    }

    if let Some(expected_fid) = action.preconditions.keeper_file_id {
        if let Some(actual_fid) = FileId::from_metadata(&keeper_meta) {
            if actual_fid != expected_fid {
                return Err(PreconditionError::KeeperFileIdMismatch {
                    path: action.keeper_path.clone(),
                    expected: expected_fid,
                    actual: actual_fid,
                });
            }
        }
    }

    // Recompute keeper content hash to ensure content preservation invariant
    let keeper_hash =
        compute_file_blake3_hash(&action.keeper_path).map_err(|e| PreconditionError::Io {
            path: action.keeper_path.clone(),
            message: format!("Failed to compute keeper content hash: {}", e),
        })?;

    if keeper_hash != action.preconditions.expected_hash {
        return Err(PreconditionError::KeeperHashMismatch {
            path: action.keeper_path.clone(),
            expected: action.preconditions.expected_hash.clone(),
            actual: keeper_hash,
        });
    }

    // -------------------------------------------------------------
    // 2. Revalidate Candidate Source Duplicate File
    // -------------------------------------------------------------
    let src_meta = match fs::symlink_metadata(&action.source_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(PreconditionError::SourceMissing(action.source_path.clone()));
        }
        Err(e) => {
            return Err(PreconditionError::Io {
                path: action.source_path.clone(),
                message: e.to_string(),
            });
        }
    };

    if src_meta.file_type().is_symlink() {
        return Err(PreconditionError::SourceIsSymlink(
            action.source_path.clone(),
        ));
    }

    if !src_meta.is_file() {
        return Err(PreconditionError::SourceNotRegularFile(
            action.source_path.clone(),
        ));
    }

    if src_meta.len() != action.preconditions.expected_size {
        return Err(PreconditionError::SourceSizeMismatch {
            path: action.source_path.clone(),
            expected: action.preconditions.expected_size,
            actual: src_meta.len(),
        });
    }

    if let Some(expected_fid) = action.preconditions.source_file_id {
        if let Some(actual_fid) = FileId::from_metadata(&src_meta) {
            if actual_fid != expected_fid {
                return Err(PreconditionError::SourceFileIdMismatch {
                    path: action.source_path.clone(),
                    expected: expected_fid,
                    actual: actual_fid,
                });
            }
        }
    }

    // Recompute source content hash
    let src_hash =
        compute_file_blake3_hash(&action.source_path).map_err(|e| PreconditionError::Io {
            path: action.source_path.clone(),
            message: format!("Failed to compute source content hash: {}", e),
        })?;

    if src_hash != action.preconditions.expected_hash {
        return Err(PreconditionError::SourceHashMismatch {
            path: action.source_path.clone(),
            expected: action.preconditions.expected_hash.clone(),
            actual: src_hash,
        });
    }

    // -------------------------------------------------------------
    // 3. Quarantine Path & Boundary Verification
    // -------------------------------------------------------------
    if action.quarantine_path.parent().is_none() {
        return Err(PreconditionError::Io {
            path: action.quarantine_path.clone(),
            message: "Quarantine path has no parent directory".into(),
        });
    }

    // Check same-device / volume boundary (resolves nearest existing ancestor without disk mutation)
    if let Err(err) = verify_same_filesystem(&action.source_path, &action.quarantine_path) {
        match err {
            MutationError::CrossDeviceMoveUnsupported {
                source,
                destination,
            } => {
                return Err(PreconditionError::CrossDeviceMove {
                    source,
                    destination,
                });
            }
            other => {
                return Err(PreconditionError::Io {
                    path: action.quarantine_path.clone(),
                    message: other.to_string(),
                });
            }
        }
    }

    // -------------------------------------------------------------
    // 4. Destination Collision Pre-Check
    // -------------------------------------------------------------
    // Note: This pre-check provides immediate user feedback, while the atomic syscall
    // (`RENAME_EXCL`) remains the non-negotiable kernel-level guarantee against TOCTOU races.
    if fs::symlink_metadata(&action.quarantine_path).is_ok() {
        return Err(PreconditionError::DestinationAlreadyExists(
            action.quarantine_path.clone(),
        ));
    }

    Ok(())
}
