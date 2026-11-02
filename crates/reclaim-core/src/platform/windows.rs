use crate::entry::SkipReason;
use std::fs::FileType;
use std::os::windows::fs::FileTypeExt;

/// Checks if a Windows filesystem entry is a reparse point (junction, mount point, or redirect).
pub fn is_reparse_point(ft: &FileType) -> bool {
    ft.is_reparse_point()
}

/// Classifies Windows-specific non-regular file types that should not be scanned.
pub fn classify_special_file_type(ft: &FileType) -> Option<SkipReason> {
    if ft.is_reparse_point() {
        Some(SkipReason::ReparsePoint)
    } else {
        None
    }
}
