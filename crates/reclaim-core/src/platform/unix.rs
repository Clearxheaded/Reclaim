use crate::entry::SkipReason;
use std::fs::FileType;
use std::os::unix::fs::FileTypeExt;

/// Classifies Unix-specific non-regular file types that should not be scanned.
pub fn classify_special_file_type(ft: &FileType) -> Option<SkipReason> {
    if ft.is_fifo() {
        Some(SkipReason::Fifo)
    } else if ft.is_socket() {
        Some(SkipReason::Socket)
    } else if ft.is_block_device() {
        Some(SkipReason::BlockDevice)
    } else if ft.is_char_device() {
        Some(SkipReason::CharacterDevice)
    } else {
        None
    }
}

