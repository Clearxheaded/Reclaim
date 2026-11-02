use crate::entry::SkipReason;
use std::fs::FileType;

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod windows;

/// Inspects a `FileType` for operating-system specific special types or redirects.
pub fn classify_special_type(file_type: &FileType) -> Option<SkipReason> {
    #[cfg(unix)]
    {
        unix::classify_special_file_type(file_type)
    }

    #[cfg(windows)]
    {
        windows::classify_special_file_type(file_type)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = file_type;
        None
    }
}

/// Checks if a `FileType` is a Windows reparse point. Always returns false on non-Windows.
pub fn is_reparse_point(file_type: &FileType) -> bool {
    #[cfg(windows)]
    {
        windows::is_reparse_point(file_type)
    }

    #[cfg(not(windows))]
    {
        let _ = file_type;
        false
    }
}

