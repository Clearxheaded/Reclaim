use std::fmt;
use std::io;
use std::path::PathBuf;

/// High-level classification of an error encountered during directory traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanErrorKind {
    /// Read or traverse permissions were denied.
    PermissionDenied,
    /// Path was unexpectedly not found (e.g. deleted while scanning).
    NotFound,
    /// A potential directory loop or cycle was detected.
    LoopDetected,
    /// Other operating-system I/O error.
    Io,
}

impl From<&io::Error> for ScanErrorKind {
    fn from(err: &io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::PermissionDenied => ScanErrorKind::PermissionDenied,
            io::ErrorKind::NotFound => ScanErrorKind::NotFound,
            _ => ScanErrorKind::Io,
        }
    }
}

/// An error encountered while processing a specific path during scanning.
///
/// These errors do not stop the overall scan; instead, they are collected
/// and reported in the final `ScanOutput` so the caller has full visibility
/// into unreadable or skipped areas of the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// Path where the error occurred.
    pub path: PathBuf,
    /// Category of the error.
    pub kind: ScanErrorKind,
    /// Human-readable message from the OS or scanner.
    pub message: String,
}

impl ScanError {
    pub fn new(path: PathBuf, err: io::Error) -> Self {
        Self {
            kind: ScanErrorKind::from(&err),
            message: err.to_string(),
            path,
        }
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{:?}] {}: {}",
            self.kind,
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for ScanError {}

/// Fatal errors that prevent a scan from starting.
#[derive(Debug)]
pub enum RootScanError {
    /// The specified root path does not exist.
    NotFound(PathBuf),
    /// The specified root path is not a directory.
    NotADirectory(PathBuf),
    /// The specified root path is a symbolic link (safety invariant: never traverse symlinks).
    IsSymbolicLink(PathBuf),
    /// The specified root path is a Windows reparse point or junction.
    IsReparsePoint(PathBuf),
    /// An I/O error occurred while inspecting the root path.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for RootScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootScanError::NotFound(p) => write!(f, "path does not exist: {}", p.display()),
            RootScanError::NotADirectory(p) => {
                write!(f, "path is not a directory: {}", p.display())
            }
            RootScanError::IsSymbolicLink(p) => write!(
                f,
                "refusing to scan root because it is a symbolic link: {}",
                p.display()
            ),
            RootScanError::IsReparsePoint(p) => write!(
                f,
                "refusing to scan root because it is a Windows reparse point / junction: {}",
                p.display()
            ),
            RootScanError::Io { path, source } => {
                write!(f, "failed to access root {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for RootScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RootScanError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
