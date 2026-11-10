// Domain Types and IPC Contracts for Reclaim Desktop

export interface ScanProgressEvent {
  entries_scanned?: number;
  files_scanned?: number;
  directories_scanned?: number;
  symlinks_skipped?: number;
  bytes_scanned?: number;
  current_path?: string;

  // Backend DTO fields
  directories_visited?: number;
  files_discovered?: number;
  total_file_bytes?: number;
  current_directory?: string;
}

export type DuplicateStage =
  | 'GroupingBySize'
  | 'SizeGrouping'
  | 'PrefixHashing'
  | 'FullHashing';

export interface DuplicateProgressEvent {
  stage: DuplicateStage;
  current_file?: string | number;
  current_step?: number;
  total_candidates?: number;
  total_steps?: number;
  bytes_processed?: number;
  total_bytes?: number;
  current_path?: string;
}

export interface ExecutionProgressEventPayload {
  action_id: string;
  source_path: string;
  quarantine_path: string;
  state: 'Validating' | 'Executing' | 'Completed' | 'Failed';
  error_message?: string;
  completed_count: number;
  total_count: number;
}

export interface UndoProgressEventPayload {
  action_id: string;
  source_path: string;
  quarantine_path: string;
  status: 'Validating' | 'Restoring' | 'Restored' | 'Conflict' | 'Skipped';
  reason?: string;
  completed_count: number;
  total_count: number;
}

export interface FileEntryPayload {
  path: string;
  size: number;
  modified: number; // Unix timestamp seconds
  is_symlink: boolean;
  device_id?: number;
  inode?: number;
  nlink?: number;
}

export interface PhysicalCopyPayload {
  device_id?: number;
  inode?: number;
  file_id?: string;
  primary_path: string;
  all_paths: string[];
}

export interface DuplicateGroupPayload {
  size: number;
  blake3_hash: string;
  physical_copies: PhysicalCopyPayload[];
  total_physical_copies: number;
  total_path_count: number;
  reclaimable_bytes: number;
}

export interface DuplicateSummaryPayload {
  total_files_scanned: number;
  duplicate_groups_count: number;
  duplicate_files_count: number;
  reclaimable_bytes: number;
  total_duplicate_bytes: number;
  hard_link_savings_bytes: number;
}

export interface DuplicateOutputPayload {
  groups: DuplicateGroupPayload[];
  summary: DuplicateSummaryPayload;
}

export type KeeperPolicy =
  | { type: 'ShortestPath' }
  | { type: 'OldestModified' }
  | { type: 'NewestModified' }
  | { type: 'PreferredDirectory'; directory: string };

export interface ActionPreconditionsPayload {
  expected_size: number;
  expected_blake3_hash: string;
  expected_device_id?: number;
  expected_inode?: number;
  expected_nlink?: number;
  expected_modified?: number;
  source_file_id?: string;
  keeper_file_id?: string;
  source_modified?: string;
  keeper_modified?: string;
}

export interface QuarantineActionPayload {
  action_id?: string;
  source_path: string;
  quarantine_path: string;
  keeper_path: string;
  preconditions: ActionPreconditionsPayload;
  rationale?: string;
}

export interface CleanupPlanSummaryPayload {
  total_actions: number;
  total_bytes_to_quarantine: number;
  protected_keepers_count: number;
}

export interface CleanupPlanPayload {
  plan_id: string;
  root_dir: string;
  quarantine_dir: string;
  created_at: string;
  actions: QuarantineActionPayload[];
  summary: CleanupPlanSummaryPayload;
}

export interface ExecutionOptionsPayload {
  dry_run: boolean;
  halt_on_error: boolean;
  journal_path?: string;
}

export interface ActionResultPayload {
  action_id: string;
  source_path: string;
  quarantine_path: string;
  status: 'Completed' | 'Failed' | 'Skipped';
  error?: string;
}

export interface ExecutionReportPayload {
  session_id: string;
  plan_id: string;
  total_actions: number;
  completed_actions: number;
  failed_actions: number;
  bytes_quarantined: number;
  duration_ms: number;
  dry_run: boolean;
  results: ActionResultPayload[];
}

export interface UndoConflictDetail {
  action_id: string;
  source_path: string;
  quarantine_path: string;
  reason:
    | 'DestinationOccupied'
    | 'QuarantineFileMissing'
    | 'QuarantineFileTampered'
    | 'KeeperMissing'
    | 'KeeperTampered'
    | 'CrossDeviceRestoreForbidden';
  message: string;
}

export interface UndoOptionsPayload {
  dry_run: boolean;
  halt_on_error: boolean;
  journal_path?: string;
}

export interface UndoReportPayload {
  session_id: string;
  restored_count: number;
  failed_count: number;
  skipped_count: number;
  restored_bytes: number;
  duration_ms: number;
  dry_run: boolean;
  conflicts: UndoConflictDetail[];
}

export interface SessionHistoryItem {
  session_id: string;
  plan_id?: string;
  root_path: string;
  started_at: string;
  completed_at?: string;
  status: 'Active' | 'Completed' | 'Interrupted' | 'Reverted';
  total_actions: number;
  completed_actions: number;
  reverted_actions: number;
  failed_actions: number;
  quarantined_bytes: number;
  restored_bytes: number;
}

export interface RecoveryReportPayload {
  interrupted_sessions_found: number;
  reconciled_sessions_count: number;
  completed_at_quarantine_count: number;
  unmoved_at_source_count: number;
  corrupt_or_missing_count: number;
}

export interface CowDeduplicationResult {
  source: string;
  target: string;
  bytes_reclaimed: number;
  strategy: string;
  verified_hash: string;
  already_shared: boolean;
}

export type InvalidationReason = 'Modified' | 'Removed' | 'Renamed';

export interface CandidateInvalidatedEvent {
  path: string;
  reason: InvalidationReason;
  group_hash: string;
  remaining_files_in_group: number;
  new_reclaimable_bytes: number;
}

export interface InvalidationToastItem {
  id: string;
  notice: CandidateInvalidatedEvent;
  timestamp: number;
}

export type ActiveTab = 'scan' | 'plan' | 'monitor' | 'history' | 'invariants';
