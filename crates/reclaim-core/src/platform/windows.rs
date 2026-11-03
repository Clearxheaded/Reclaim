use crate::entry::SkipReason;
use std::fs::FileType;
use std::os::windows::fs::FileTypeExt;

/// Checks if a Windows filesystem entry is a reparse point (junction, mount point, or redirect).
pub fn is_reparse_point(ft: &FileType) -> bool {
    ft.is_symlink() || ft.is_symlink_dir() || ft.is_symlink_file()
}

/// Classifies Windows-specific non-regular file types that should not be scanned.
pub fn classify_special_file_type(ft: &FileType) -> Option<SkipReason> {
    if is_reparse_point(ft) {
        Some(SkipReason::ReparsePoint)
    } else {
        None
    }
}
