use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptionsDto {
    #[serde(default)]
    pub include_empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgressDto {
    pub directories_visited: u64,
    pub files_discovered: u64,
    pub total_file_bytes: u64,
    pub current_directory: String,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub bytes_scanned: u64,
    pub current_path: String,
    pub symlinks_skipped: u64,
    pub entries_scanned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DuplicateStageDto {
    GroupingBySize,
    PrefixHashing,
    FullHashing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateProgressDto {
    pub stage: DuplicateStageDto,
    pub current_file: u64,
    pub total_candidates: u64,
    pub current_path: String,
    pub current_step: u64,
    pub total_steps: u64,
    pub bytes_processed: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalCopyDto {
    pub file_id: Option<String>,
    pub primary_path: String,
    pub all_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroupDto {
    pub size: u64,
    pub blake3_hash: String,
    pub physical_copies: Vec<PhysicalCopyDto>,
    pub total_physical_copies: usize,
    pub total_path_count: usize,
    pub reclaimable_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateSummaryDto {
    pub total_files_scanned: u64,
    pub duplicate_groups_count: usize,
    pub duplicate_files_count: usize,
    pub reclaimable_bytes: u64,
    pub total_duplicate_bytes: u64,
    pub hard_link_savings_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CowDeduplicationResultDto {
    pub source: String,
    pub target: String,
    pub bytes_reclaimed: u64,
    pub strategy: String,
    pub verified_hash: String,
    pub already_shared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateOutputDto {
    pub groups: Vec<DuplicateGroupDto>,
    pub summary: DuplicateSummaryDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum KeeperPolicyDto {
    ShortestPath,
    OldestModified,
    NewestModified,
    PreferredDirectory { directory: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPreconditionsDto {
    pub expected_size: u64,
    pub expected_blake3_hash: String,
    pub source_file_id: Option<String>,
    pub keeper_file_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineActionDto {
    pub source_path: String,
    pub quarantine_path: String,
    pub keeper_path: String,
    pub preconditions: ActionPreconditionsDto,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPlanSummaryDto {
    pub total_actions: usize,
    pub total_bytes_to_quarantine: u64,
    pub protected_keepers_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPlanDto {
    pub plan_id: String,
    pub root_dir: String,
    pub quarantine_dir: String,
    pub created_at: String,
    pub policy: KeeperPolicyDto,
    pub actions: Vec<QuarantineActionDto>,
    pub summary: CleanupPlanSummaryDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOptionsDto {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_halt_on_error")]
    pub halt_on_error: bool,
    pub journal_path: Option<String>,
}

fn default_halt_on_error() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResultDto {
    pub action_id: String,
    pub source_path: String,
    pub quarantine_path: String,
    pub status: String,
    pub bytes_freed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReportDto {
    pub session_id: String,
    pub plan_id: String,
    pub total_actions: usize,
    pub completed_actions: usize,
    pub failed_actions: usize,
    pub skipped_actions: usize,
    pub bytes_quarantined: u64,
    pub duration_ms: u64,
    pub dry_run: bool,
    pub results: Vec<ActionResultDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProgressDto {
    pub action_id: String,
    pub source_path: String,
    pub quarantine_path: String,
    pub state: String,
    pub error_message: Option<String>,
    pub completed_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoConflictDetailDto {
    pub action_id: String,
    pub source_path: String,
    pub quarantine_path: String,
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoOptionsDto {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub halt_on_error: bool,
    pub journal_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoReportDto {
    pub session_id: Option<String>,
    pub restored_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub restored_bytes: u64,
    pub duration_ms: u64,
    pub dry_run: bool,
    pub conflicts: Vec<UndoConflictDetailDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoProgressDto {
    pub action_id: String,
    pub source_path: String,
    pub quarantine_path: String,
    pub status: String,
    pub reason: Option<String>,
    pub completed_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHistoryDto {
    pub session_id: String,
    pub plan_ids: Vec<String>,
    pub root_path: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub total_actions: usize,
    pub completed_actions: usize,
    pub reverted_actions: usize,
    pub failed_actions: usize,
    pub quarantined_bytes: u64,
    pub restored_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryReportDto {
    pub is_clean: bool,
    pub interrupted_sessions_found: usize,
    pub reconciled_sessions_count: usize,
    pub completed_at_quarantine_count: usize,
    pub unmoved_at_source_count: usize,
    pub corrupt_or_missing_count: usize,
}
