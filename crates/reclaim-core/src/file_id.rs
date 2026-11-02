use std::fmt;
use std::fs::Metadata;

/// A cross-platform identifier for a file on disk.
///
/// On Unix systems, this combines the device number (`st_dev`) and inode number (`st_ino`).
/// On Windows systems, this combines the volume serial number and the 64-bit file index.
///
/// This identity allows distinguishing distinct physical files from hard links pointing to
/// the same underlying storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    pub fn from_metadata(meta: &Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(FileId::Unix {
                dev: meta.dev(),
                ino: meta.ino(),
            })
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            let volume = meta.volume_serial_number()?;
            let index = meta.file_index()?;
            Some(FileId::Windows {
                volume_serial_number: volume,
                file_index: index,
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = meta;
            None
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_file_id_display() {
        let unix_id = FileId::Unix { dev: 42, ino: 1001 };
        assert_eq!(unix_id.to_string(), "unix(42:1001)");

        let win_id = FileId::Windows {
            volume_serial_number: 0x1234ABCD,
            file_index: 9999,
        };
        assert_eq!(win_id.to_string(), "win(0x1234ABCD:9999)");
    }

    #[test]
    fn test_from_metadata_on_real_file() {
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("reclaim_test_file_id.tmp");
        let _file = File::create(&test_path).expect("failed to create temp file");

        let meta = std::fs::symlink_metadata(&test_path).expect("failed to get metadata");
        let file_id = FileId::from_metadata(&meta);

        #[cfg(any(unix, windows))]
        assert!(file_id.is_some());

        let _ = std::fs::remove_file(&test_path);
    }
}
