use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::Metadata;
use std::path::Path;
use std::str::FromStr;

/// A cross-platform identifier for a file on disk.
///
/// On Unix systems, this combines the device number (`st_dev`) and inode number (`st_ino`).
/// On Windows systems, this combines the volume serial number and the 64-bit file index.
///
/// This identity allows distinguishing distinct physical files from hard links pointing to
/// the same underlying storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FileId {
    /// Unix device ID and inode number.
    Unix { dev: u64, ino: u64 },
    /// Windows volume serial number and 64-bit file index.
    Windows {
        volume_serial_number: u32,
        file_index: u64,
    },
}

impl FileId {
    /// Extracts a `FileId` from file `Metadata` where supported by the operating system.
    ///
    /// On Unix systems this reads `st_dev` and `st_ino` directly from `Metadata`.
    /// On Windows, `std::fs::Metadata` on stable Rust does not expose volume serial numbers
    /// or file indices; prefer [`FileId::from_path`] or [`FileId::from_path_and_metadata`]
    /// whenever a path is available.
    pub fn from_metadata(meta: &Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(FileId::Unix {
                dev: meta.dev(),
                ino: meta.ino(),
            })
        }

        #[cfg(not(unix))]
        {
            let _ = meta;
            None
        }
    }

    /// Extracts a `FileId` from a filesystem path without following symbolic links or
    /// Windows reparse points.
    pub fn from_path(path: &Path) -> Option<Self> {
        #[cfg(unix)]
        {
            std::fs::symlink_metadata(path)
                .ok()
                .and_then(|meta| Self::from_metadata(&meta))
        }

        #[cfg(windows)]
        {
            windows_file_id_from_path(path)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            None
        }
    }

    /// Extracts a `FileId` using `Metadata` when sufficient (Unix) and falling back to a
    /// non-following path handle query where `Metadata` lacks file identity (Windows).
    pub fn from_path_and_metadata(path: &Path, meta: &Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            let _ = path;
            Self::from_metadata(meta)
        }

        #[cfg(windows)]
        {
            let _ = meta;
            windows_file_id_from_path(path)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (path, meta);
            None
        }
    }
}

#[cfg(windows)]
fn windows_file_id_from_path(path: &Path) -> Option<FileId> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    #[allow(non_snake_case, dead_code)]
    struct FileTime {
        dwLowDateTime: u32,
        dwHighDateTime: u32,
    }

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    #[allow(non_snake_case, dead_code)]
    struct ByHandleFileInformation {
        dwFileAttributes: u32,
        ftCreationTime: FileTime,
        ftLastAccessTime: FileTime,
        ftLastWriteTime: FileTime,
        dwVolumeSerialNumber: u32,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        nNumberOfLinks: u32,
        nFileIndexHigh: u32,
        nFileIndexLow: u32,
    }

    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *const c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: *mut c_void,
        ) -> *mut c_void;
        fn GetFileInformationByHandle(
            hFile: *mut c_void,
            lpFileInformation: *mut ByHandleFileInformation,
        ) -> i32;
        fn CloseHandle(hObject: *mut c_void) -> i32;
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.is_empty() || wide.contains(&0) {
        return None;
    }
    wide.push(0);

    // Desire 0 opens for metadata-only queries without needing file read/write permissions.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };

    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut info = ByHandleFileInformation::default();
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe {
        CloseHandle(handle);
    }

    if ok == 0 {
        return None;
    }

    let file_index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
    Some(FileId::Windows {
        volume_serial_number: info.dwVolumeSerialNumber,
        file_index,
    })
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileId::Unix { dev, ino } => write!(f, "unix({}:{})", dev, ino),
            FileId::Windows {
                volume_serial_number,
                file_index,
            } => write!(f, "win({:#X}:{})", volume_serial_number, file_index),
        }
    }
}

/// Error returned when parsing a [`FileId`] from its display string fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFileIdError;

impl fmt::Display for ParseFileIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid FileId string")
    }
}

impl std::error::Error for ParseFileIdError {}

impl FromStr for FileId {
    type Err = ParseFileIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(inner) = s.strip_prefix("unix(").and_then(|r| r.strip_suffix(')')) {
            let (dev_str, ino_str) = inner.split_once(':').ok_or(ParseFileIdError)?;
            let dev = dev_str.parse::<u64>().map_err(|_| ParseFileIdError)?;
            let ino = ino_str.parse::<u64>().map_err(|_| ParseFileIdError)?;
            return Ok(FileId::Unix { dev, ino });
        }

        if let Some(inner) = s.strip_prefix("win(").and_then(|r| r.strip_suffix(')')) {
            let (vol_str, idx_str) = inner.split_once(':').ok_or(ParseFileIdError)?;
            let hex = vol_str
                .strip_prefix("0x")
                .or_else(|| vol_str.strip_prefix("0X"));
            let volume_serial_number = if let Some(hex) = hex {
                u32::from_str_radix(hex, 16).map_err(|_| ParseFileIdError)?
            } else {
                vol_str.parse::<u32>().map_err(|_| ParseFileIdError)?
            };
            let file_index = idx_str.parse::<u64>().map_err(|_| ParseFileIdError)?;
            return Ok(FileId::Windows {
                volume_serial_number,
                file_index,
            });
        }

        Err(ParseFileIdError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_file_id_display_and_parse() {
        let unix_id = FileId::Unix { dev: 42, ino: 1001 };
        assert_eq!(unix_id.to_string(), "unix(42:1001)");
        assert_eq!("unix(42:1001)".parse::<FileId>(), Ok(unix_id));

        let win_id = FileId::Windows {
            volume_serial_number: 0x1234ABCD,
            file_index: 9999,
        };
        assert_eq!(win_id.to_string(), "win(0x1234ABCD:9999)");
        assert_eq!("win(0x1234ABCD:9999)".parse::<FileId>(), Ok(win_id));
        assert!("invalid".parse::<FileId>().is_err());
    }

    #[test]
    fn test_from_metadata_on_real_file() {
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("reclaim_test_file_id.tmp");
        let _file = File::create(&test_path).expect("failed to create temp file");

        let meta = std::fs::symlink_metadata(&test_path).expect("failed to get metadata");
        let file_id = FileId::from_metadata(&meta);
        let path_file_id = FileId::from_path_and_metadata(&test_path, &meta);

        #[cfg(unix)]
        {
            assert!(file_id.is_some());
            assert_eq!(file_id, path_file_id);
        }

        #[cfg(windows)]
        {
            assert!(file_id.is_none());
            assert!(matches!(path_file_id, Some(FileId::Windows { .. })));
        }

        let _ = std::fs::remove_file(&test_path);
    }

    #[test]
    fn test_hard_link_file_id_matches_across_platforms() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let original = temp_dir.path().join("original.txt");
        let hard_link = temp_dir.path().join("hardlink.txt");
        let separate = temp_dir.path().join("separate.txt");

        std::fs::write(&original, b"hello").expect("write original");
        std::fs::hard_link(&original, &hard_link).expect("create hard link");
        std::fs::write(&separate, b"hello").expect("write separate");

        let id_orig = FileId::from_path(&original);
        let id_link = FileId::from_path(&hard_link);
        let id_sep = FileId::from_path(&separate);

        #[cfg(any(unix, windows))]
        {
            assert!(id_orig.is_some());
            assert_eq!(id_orig, id_link);
            assert_ne!(id_orig, id_sep);
        }
    }
}
