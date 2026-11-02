use crate::file_id::FileId;
use std::path::PathBuf;
use std::time::SystemTime;

/// Represents a regular file discovered during a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Full path to the file.
    pub path: PathBuf,
    /// Size of the file in bytes.
    pub size: u64,
    /// Last modified time, if reported by the filesystem.
    pub modified: Option<SystemTime>,
    /// Filesystem identity (device/inode or volume/file index), if supported.
    pub file_id: Option<FileId>,
    /// Whether the file is marked read-only.
    pub readonly: bool,
}

/// The reason a filesystem entry was deliberately skipped by the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// Symbolic link (Reclaim never traverses symlinks to avoid directory loops and unexpected mutations).
    SymbolicLink,
    /// Windows reparse point (such as a directory junction, mount point, or app execution alias).
    ReparsePoint,
    /// Named pipe / FIFO.
    Fifo,
    /// Unix domain socket.
    Socket,
    /// Block device.
    BlockDevice,
    /// Character device.
    CharacterDevice,
    /// Other unhandled or non-standard file type.
    UnknownFileType,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::SymbolicLink => write!(f, "symbolic link"),
            SkipReason::ReparsePoint => write!(f, "reparse point / junction"),
            SkipReason::Fifo => write!(f, "fifo / named pipe"),
            SkipReason::Socket => write!(f, "unix domain socket"),
            SkipReason::BlockDevice => write!(f, "block device"),
            SkipReason::CharacterDevice => write!(f, "character device"),
            SkipReason::UnknownFileType => write!(f, "unknown file type"),
        }
    }
}

/// An entry skipped by the scanner with its path and skip rationale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    /// Path of the skipped entry.
    pub path: PathBuf,
    /// Why the entry was skipped.
    pub reason: SkipReason,
}

