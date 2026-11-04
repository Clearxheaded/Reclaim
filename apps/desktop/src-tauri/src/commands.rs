use crate::types::*;
use reclaim_core::{
    find_duplicates, generate_plan, scan_directory, ActionPreconditions, CancellationToken,
    CleanupPlan, DuplicateGroup, DuplicateOptions, DuplicateStage, FileEntry, KeeperPolicy,
    PhysicalCopy, PlanOptions, QuarantineAction, ScanOptions,
};
use reclaim_storage::datetime::format_rfc3339;
use reclaim_storage::{
    reconcile_startup_state, ActionExecutionStatus, ActionJournal, ActionRecoveryResolution,
    ExecutionOptions, ExecutionProgressEvent, SafeExecutor, SafeReverter, SessionId, SessionStatus,
    UndoConflictReason, UndoOptions, UndoProgressEvent,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{Emitter, State, Window};

pub struct AppState {
    pub cancel_token: Mutex<Arc<CancellationToken>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            cancel_token: Mutex::new(Arc::new(CancellationToken::new())),
        }
    }
}

// ----------------------------------------------------------------------------
// Command 1: Scan and find duplicates with real-time streaming progress
// ----------------------------------------------------------------------------
#[tauri::command]
pub async fn scan_and_find_duplicates(
    window: Window,
    root_path: String,
    options: ScanOptionsDto,
    state: State<'_, AppState>,
) -> Result<DuplicateOutputDto, String> {
    let cancel = Arc::new(CancellationToken::new());
    if let Ok(mut guard) = state.cancel_token.lock() {
        *guard = cancel.clone();
    }

    let root_buf = PathBuf::from(&root_path);
    if !root_buf.is_dir() {
        return Err(format!("Target path is not a directory: {}", root_path));
    }

    let cancel_clone = cancel.clone();
    let win_scan = window.clone();
    let win_dup = window.clone();

    tokio::task::spawn_blocking(move || {
        // Step 1: Scan directory
        let scan_cb = Box::new(move |p: &reclaim_core::ScanProgress| {
            let _ = win_scan.emit(
                "reclaim://scan-progress",
                ScanProgressDto {
                    directories_visited: p.directories_visited,
                    files_discovered: p.files_discovered,
                    total_file_bytes: p.total_file_bytes,
                    current_directory: p.current_directory.to_string_lossy().to_string(),
                },
            );
        });

        let scan_opts = ScanOptions {
            cancellation_token: Some((*cancel_clone).clone()),
            progress_callback: Some(scan_cb),
        };

        let scan_output =
            scan_directory(&root_buf, scan_opts).map_err(|e| format!("Scan error: {}", e))?;

        if cancel_clone.is_cancelled() {
            return Err("Scan cancelled by user".to_string());
        }

        // Step 2: Duplicate detection
        let dup_cb = Box::new(move |p: &reclaim_core::DuplicateProgress| {
            let (stage_dto, cur_file, tot_cand, cur_path) = match &p.stage {
                DuplicateStage::GroupingBySize => {
                    (DuplicateStageDto::GroupingBySize, 0u64, 0u64, String::new())
                }
                DuplicateStage::PrefixHashing {
                    current_file,
                    total_candidates,
                    current_path,
                } => (
                    DuplicateStageDto::PrefixHashing,
                    *current_file,
                    *total_candidates,
                    current_path.to_string_lossy().to_string(),
                ),
                DuplicateStage::FullHashing {
                    current_file,
                    total_candidates,
                    current_path,
                } => (
                    DuplicateStageDto::FullHashing,
                    *current_file,
                    *total_candidates,
                    current_path.to_string_lossy().to_string(),
                ),
            };

            let _ = win_dup.emit(
                "reclaim://duplicate-progress",
                DuplicateProgressDto {
                    stage: stage_dto,
                    current_file: cur_file,
                    total_candidates: tot_cand,
                    current_path: cur_path,
                },
            );
        });

        let dup_opts = DuplicateOptions {
            prefix_size: reclaim_core::DEFAULT_PREFIX_SIZE,
            include_empty: options.include_empty,
            cancellation_token: Some((*cancel_clone).clone()),
            progress_callback: Some(dup_cb),
        };

        let dup_output = find_duplicates(&scan_output.entries, &dup_opts);

        // Convert domain groups to DTOs
        let groups = dup_output
            .groups
            .into_iter()
            .map(|g| {
                let file_count = g.file_count();
                let physical_copies = g
                    .physical_copies
                    .into_iter()
                    .map(|pc| {
                        let primary = pc
                            .paths
                            .first()
                            .cloned()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        PhysicalCopyDto {
                            file_id: pc.file_id.map(|id| id.to_string()),
                            primary_path: primary,
                            all_paths: pc
                                .paths
                                .into_iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect(),
                        }
                    })
                    .collect();

                DuplicateGroupDto {
                    size: g.file_size,
                    blake3_hash: g.hash.to_hex().to_string(),
                    physical_copies,
                    total_physical_copies: g.distinct_physical_copies,
                    total_path_count: file_count,
                    reclaimable_bytes: g.reclaimable_bytes,
                }
            })
            .collect();

        Ok(DuplicateOutputDto {
            groups,
            summary: DuplicateSummaryDto {
                total_files_scanned: dup_output.summary.total_files_evaluated,
                duplicate_groups_count: dup_output.summary.duplicate_groups_count,
                duplicate_files_count: dup_output.summary.duplicate_files_count,
                reclaimable_bytes: dup_output.summary.total_reclaimable_bytes,
                hard_link_savings_bytes: dup_output.summary.hard_link_aliases_count,
            },
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// ----------------------------------------------------------------------------
// Command 2: Cancel scan
// ----------------------------------------------------------------------------
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(guard) = state.cancel_token.lock() {
        guard.cancel();
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Command 3: Generate prospective cleanup plan
// ----------------------------------------------------------------------------
#[tauri::command]
pub fn generate_cleanup_plan(
    root_dir: String,
    quarantine_dir: Option<String>,
    policy: KeeperPolicyDto,
    duplicate_groups: Vec<DuplicateGroupDto>,
) -> Result<CleanupPlanDto, String> {
    let root_path = PathBuf::from(&root_dir);
    let q_dir = quarantine_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| root_path.join(".reclaim").join("quarantine"));

    let keeper_policy = match &policy {
        KeeperPolicyDto::ShortestPath => KeeperPolicy::ShortestPath,
        KeeperPolicyDto::OldestModified => KeeperPolicy::OldestModified,
        KeeperPolicyDto::NewestModified => KeeperPolicy::NewestModified,
        KeeperPolicyDto::PreferredDirectory { directory } => {
            KeeperPolicy::PreferredDirectory(PathBuf::from(directory))
        }
    };

    let plan_opts = PlanOptions {
        root_dir: root_path.clone(),
        quarantine_dir: Some(q_dir.clone()),
        policy: keeper_policy,
    };

    // Reconstruct core DuplicateGroups from DTOs
    let mut core_groups = Vec::new();
    for g in duplicate_groups {
        let hash = blake3::Hash::from_hex(&g.blake3_hash)
            .map_err(|e| format!("Invalid hex hash: {}", e))?;

        let mut entries = Vec::new();
        let mut physical_copies = Vec::new();

        for pc in g.physical_copies {
            let paths: Vec<PathBuf> = pc.all_paths.into_iter().map(PathBuf::from).collect();
            for p in &paths {
                entries.push(FileEntry {
                    path: p.clone(),
                    size: g.size,
                    modified: None,
                    file_id: None,
                    readonly: false,
                });
            }
            physical_copies.push(PhysicalCopy {
                file_id: None,
                paths,
            });
        }

        core_groups.push(DuplicateGroup {
            hash,
            file_size: g.size,
            entries,
            distinct_physical_copies: g.total_physical_copies,
            physical_copies,
            reclaimable_bytes: g.reclaimable_bytes,
        });
    }

    let plan = generate_plan(&core_groups, &plan_opts).map_err(|e| format!("Plan error: {}", e))?;

    let actions = plan
        .actions
        .into_iter()
        .map(|a| QuarantineActionDto {
            source_path: a.source_path.to_string_lossy().to_string(),
            quarantine_path: a.quarantine_path.to_string_lossy().to_string(),
            keeper_path: a.keeper_path.to_string_lossy().to_string(),
            rationale: a.rationale,
            preconditions: ActionPreconditionsDto {
                expected_size: a.preconditions.expected_size,
                expected_blake3_hash: a.preconditions.expected_hash,
                source_file_id: a.preconditions.source_file_id.map(|id| id.to_string()),
                keeper_file_id: a.preconditions.keeper_file_id.map(|id| id.to_string()),
            },
        })
        .collect();

    let created_at_str = format_rfc3339(plan.created_at);

    Ok(CleanupPlanDto {
        plan_id: uuid::Uuid::new_v4().to_string(),
        root_dir: plan.root_dir.to_string_lossy().to_string(),
        quarantine_dir: plan.quarantine_dir.to_string_lossy().to_string(),
        created_at: created_at_str,
        policy,
        actions,
        summary: CleanupPlanSummaryDto {
            total_actions: plan.summary.planned_actions,
            total_bytes_to_quarantine: plan.summary.total_reclaimable_bytes,
            protected_keepers_count: plan.summary.protected_keepers,
        },
    })
}

// ----------------------------------------------------------------------------
// Command 4: Execute quarantine plan with live streaming progress
// ----------------------------------------------------------------------------
#[tauri::command]
pub async fn execute_quarantine_plan(
    window: Window,
    plan: CleanupPlanDto,
    options: ExecutionOptionsDto,
) -> Result<ExecutionReportDto, String> {
    tokio::task::spawn_blocking(move || {
        let root_path = PathBuf::from(&plan.root_dir);
        let journal_path = options
            .journal_path
            .map(PathBuf::from)
            .unwrap_or_else(|| root_path.join(".reclaim").join("journal.db"));

        // Ensure journal directory exists
        if let Some(parent) = journal_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut journal =
            ActionJournal::open(&journal_path).map_err(|e| format!("Journal open error: {}", e))?;

        let exec_opts = ExecutionOptions {
            dry_run: options.dry_run,
            halt_on_error: options.halt_on_error,
        };

        let keeper_policy = match plan.policy {
            KeeperPolicyDto::ShortestPath => KeeperPolicy::ShortestPath,
            KeeperPolicyDto::OldestModified => KeeperPolicy::OldestModified,
            KeeperPolicyDto::NewestModified => KeeperPolicy::NewestModified,
            KeeperPolicyDto::PreferredDirectory { directory } => {
                KeeperPolicy::PreferredDirectory(PathBuf::from(directory))
            }
        };

        // Convert DTO back to core CleanupPlan
        let core_actions: Vec<QuarantineAction> = plan
            .actions
            .into_iter()
            .map(|a| QuarantineAction {
                source_path: PathBuf::from(a.source_path),
                quarantine_path: PathBuf::from(a.quarantine_path),
                keeper_path: PathBuf::from(a.keeper_path),
                rationale: a.rationale,
                preconditions: ActionPreconditions {
                    expected_size: a.preconditions.expected_size,
                    expected_hash: a.preconditions.expected_blake3_hash,
                    source_modified: None,
                    source_file_id: None,
                    keeper_modified: None,
                    keeper_file_id: None,
                },
            })
            .collect();

        let core_plan = CleanupPlan {
            created_at: SystemTime::now(),
            root_dir: PathBuf::from(plan.root_dir),
            quarantine_dir: PathBuf::from(plan.quarantine_dir),
            policy: keeper_policy,
            actions: core_actions,
            keepers: Vec::new(),
            summary: reclaim_core::PlanSummary {
                total_duplicate_files: plan.summary.total_actions
                    + plan.summary.protected_keepers_count,
                duplicate_groups_count: plan.summary.protected_keepers_count,
                planned_actions: plan.summary.total_actions,
                protected_keepers: plan.summary.protected_keepers_count,
                total_reclaimable_bytes: plan.summary.total_bytes_to_quarantine,
                files_left_untouched: plan.summary.protected_keepers_count,
            },
        };

        let win_cb = window.clone();
        let progress_cb = move |event: ExecutionProgressEvent<'_>| {
            let (action_id_str, source_str, dest_str, state_str, err_msg, comp_cnt, tot_cnt) =
                match event {
                    ExecutionProgressEvent::Validating {
                        action_id,
                        source,
                        destination,
                        index,
                        total,
                    } => (
                        action_id.to_string(),
                        source.to_string_lossy().to_string(),
                        destination.to_string_lossy().to_string(),
                        "Validating".to_string(),
                        None,
                        index,
                        total,
                    ),
                    ExecutionProgressEvent::Executing {
                        action_id,
                        source,
                        destination,
                        ..
                    } => (
                        action_id.to_string(),
                        source.to_string_lossy().to_string(),
                        destination.to_string_lossy().to_string(),
                        "Executing".to_string(),
                        None,
                        0,
                        0,
                    ),
                    ExecutionProgressEvent::Completed {
                        action_id, source, ..
                    } => (
                        action_id.to_string(),
                        source.to_string_lossy().to_string(),
                        String::new(),
                        "Completed".to_string(),
                        None,
                        0,
                        0,
                    ),
                    ExecutionProgressEvent::Failed {
                        action_id,
                        source,
                        reason,
                    } => (
                        action_id.to_string(),
                        source.to_string_lossy().to_string(),
                        String::new(),
                        "Failed".to_string(),
                        Some(reason.to_string()),
                        0,
                        0,
                    ),
                    ExecutionProgressEvent::Skipped {
                        action_id,
                        source,
                        reason,
                    } => (
                        action_id.to_string(),
                        source.to_string_lossy().to_string(),
                        String::new(),
                        "Skipped".to_string(),
                        Some(reason.to_string()),
                        0,
                        0,
                    ),
                };

            let _ = win_cb.emit(
                "reclaim://execution-progress",
                ExecutionProgressDto {
                    action_id: action_id_str,
                    source_path: source_str,
                    quarantine_path: dest_str,
                    state: state_str,
                    error_message: err_msg,
                    completed_count: comp_cnt,
                    total_count: tot_cnt,
                },
            );
        };

        let mut executor = SafeExecutor::new(exec_opts);
        let report = executor
            .execute_plan_with_callback(&mut journal, &core_plan, progress_cb)
            .map_err(|e| format!("Execution error: {}", e))?;

        let results = report
            .results
            .into_iter()
            .map(|r| {
                let (status_str, err_msg) = match r.status {
                    ActionExecutionStatus::Completed => ("Completed".to_string(), None),
                    ActionExecutionStatus::Failed { reason } => {
                        ("Failed".to_string(), Some(reason))
                    }
                    ActionExecutionStatus::Skipped { reason } => {
                        ("Skipped".to_string(), Some(reason))
                    }
                };

                ActionResultDto {
                    action_id: r.action_id.to_string(),
                    source_path: r.source_path.to_string_lossy().to_string(),
                    quarantine_path: r.quarantine_path.to_string_lossy().to_string(),
                    status: status_str,
                    bytes_freed: r.bytes_freed,
                    error: err_msg,
                }
            })
            .collect();

        Ok(ExecutionReportDto {
            session_id: report.session_id.to_string(),
            plan_id: report.plan_id.to_string(),
            total_actions: report.total_actions,
            completed_actions: report.completed_actions,
            failed_actions: report.failed_actions,
            skipped_actions: report.skipped_actions,
            bytes_quarantined: report.bytes_reclaimed,
            duration_ms: report.elapsed.as_millis() as u64,
            dry_run: report.dry_run,
            results,
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// ----------------------------------------------------------------------------
// Command 5: Conditional undo session with vacancy and hash verification
// ----------------------------------------------------------------------------
#[tauri::command]
pub async fn undo_session(
    window: Window,
    session_id: Option<String>,
    options: UndoOptionsDto,
) -> Result<UndoReportDto, String> {
    tokio::task::spawn_blocking(move || {
        let journal_path = options
            .journal_path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".reclaim/journal.db"));

        let mut journal =
            ActionJournal::open(&journal_path).map_err(|e| format!("Journal open error: {}", e))?;

        let undo_opts = UndoOptions {
            dry_run: options.dry_run,
            halt_on_error: options.halt_on_error,
        };

        let target_session = match session_id {
            Some(sid) => {
                let uuid = uuid::Uuid::parse_str(&sid)
                    .map_err(|_| format!("Invalid session UUID: {}", sid))?;
                SessionId::from_uuid(uuid)
            }
            None => {
                let last = journal
                    .get_last_executed_session()
                    .map_err(|e| format!("Failed to fetch last session: {}", e))?;
                match last {
                    Some(s) => s.session_id,
                    None => return Err("No executed sessions found in journal".to_string()),
                }
            }
        };

        let win_cb = window.clone();
        let progress_cb = move |event: UndoProgressEvent<'_>| {
            let (act_id, src, quar, stat, reason_opt, comp, tot) = match event {
                UndoProgressEvent::Validating {
                    action_id,
                    source,
                    quarantine,
                    index,
                    total,
                } => (
                    action_id.to_string(),
                    source.to_string_lossy().to_string(),
                    quarantine.to_string_lossy().to_string(),
                    "Validating".to_string(),
                    None,
                    index,
                    total,
                ),
                UndoProgressEvent::Restoring {
                    action_id,
                    source,
                    quarantine,
                    ..
                } => (
                    action_id.to_string(),
                    source.to_string_lossy().to_string(),
                    quarantine.to_string_lossy().to_string(),
                    "Restoring".to_string(),
                    None,
                    0,
                    0,
                ),
                UndoProgressEvent::Restored {
                    action_id, source, ..
                } => (
                    action_id.to_string(),
                    source.to_string_lossy().to_string(),
                    String::new(),
                    "Restored".to_string(),
                    None,
                    0,
                    0,
                ),
                UndoProgressEvent::Conflict {
                    action_id,
                    source,
                    reason,
                } => (
                    action_id.to_string(),
                    source.to_string_lossy().to_string(),
                    String::new(),
                    "Conflict".to_string(),
                    Some(reason.to_string()),
                    0,
                    0,
                ),
                UndoProgressEvent::Skipped {
                    action_id,
                    source,
                    reason,
                } => (
                    action_id.to_string(),
                    source.to_string_lossy().to_string(),
                    String::new(),
                    "Skipped".to_string(),
                    Some(reason.to_string()),
                    0,
                    0,
                ),
            };

            let _ = win_cb.emit(
                "reclaim://undo-progress",
                UndoProgressDto {
                    action_id: act_id,
                    source_path: src,
                    quarantine_path: quar,
                    status: stat,
                    reason: reason_opt,
                    completed_count: comp,
                    total_count: tot,
                },
            );
        };

        let reverter = SafeReverter::new(undo_opts);
        let report = reverter
            .revert_session_with_callback(&mut journal, target_session, progress_cb)
            .map_err(|e| format!("Undo error: {}", e))?;

        let conflicts = report
            .conflicts
            .into_iter()
            .map(|c| {
                let reason_str = match &c.reason {
                    UndoConflictReason::DestinationOccupied { .. } => "DestinationOccupied",
                    UndoConflictReason::QuarantinedFileMissing { .. } => "QuarantineFileMissing",
                    UndoConflictReason::QuarantinedFileHashMismatch { .. } => {
                        "QuarantineFileTampered"
                    }
                    UndoConflictReason::KeeperMissing { .. } => "KeeperMissing",
                    UndoConflictReason::KeeperHashMismatch { .. } => "KeeperTampered",
                    UndoConflictReason::CrossDeviceMoveUnsupported { .. } => {
                        "CrossDeviceRestoreForbidden"
                    }
                    _ => "OtherSafetyConflict",
                };

                UndoConflictDetailDto {
                    action_id: c.action_id.to_string(),
                    source_path: c.source_path.to_string_lossy().to_string(),
                    quarantine_path: c.quarantine_path.to_string_lossy().to_string(),
                    reason: reason_str.to_string(),
                    message: c.reason.to_string(),
                }
            })
            .collect();

        Ok(UndoReportDto {
            session_id: report.session_id.map(|s| s.to_string()),
            restored_count: report.actions_restored,
            failed_count: report.actions_conflicted,
            skipped_count: report.actions_skipped,
            restored_bytes: report.bytes_restored,
            duration_ms: report.elapsed.as_millis() as u64,
            dry_run: report.dry_run,
            conflicts,
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// ----------------------------------------------------------------------------
// Command 6: Fetch session history from journal
// ----------------------------------------------------------------------------
#[tauri::command]
pub fn get_session_history(journal_path: Option<String>) -> Result<Vec<SessionHistoryDto>, String> {
    let j_path = journal_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".reclaim/journal.db"));

    if !j_path.exists() {
        return Ok(Vec::new());
    }

    let journal = ActionJournal::open(&j_path).map_err(|e| format!("Journal open error: {}", e))?;

    let sessions = journal
        .list_all_sessions()
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut result = Vec::new();
    for sess in sessions {
        let summary = journal.fetch_session_summary(sess.session_id).ok();
        let (
            completed_actions,
            reverted_actions,
            failed_actions,
            quarantined_bytes,
            restored_bytes,
        ) = match summary {
            Some(s) => (
                s.completed_actions,
                s.reverted_actions,
                s.failed_actions,
                s.active_quarantined_bytes,
                s.restored_bytes,
            ),
            None => (0, 0, 0, 0, 0),
        };

        let status_str = match sess.status {
            SessionStatus::Active => "Active",
            SessionStatus::Completed => "Completed",
            SessionStatus::Interrupted => "Interrupted",
        };

        result.push(SessionHistoryDto {
            session_id: sess.session_id.to_string(),
            plan_ids: sess.plan_ids.into_iter().map(|p| p.to_string()).collect(),
            root_path: sess.target_root.to_string_lossy().to_string(),
            started_at: format_rfc3339(sess.started_at),
            completed_at: sess.ended_at.map(format_rfc3339),
            status: status_str.to_string(),
            total_actions: completed_actions + reverted_actions + failed_actions,
            completed_actions,
            reverted_actions,
            failed_actions,
            quarantined_bytes,
            restored_bytes,
        });
    }

    Ok(result)
}

// ----------------------------------------------------------------------------
// Command 7: Reconcile startup state / crash recovery
// ----------------------------------------------------------------------------
#[tauri::command]
pub fn recover_journal(journal_path: Option<String>) -> Result<RecoveryReportDto, String> {
    let j_path = journal_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".reclaim/journal.db"));

    if !j_path.exists() {
        return Ok(RecoveryReportDto {
            is_clean: true,
            interrupted_sessions_found: 0,
            reconciled_sessions_count: 0,
            completed_at_quarantine_count: 0,
            unmoved_at_source_count: 0,
            corrupt_or_missing_count: 0,
        });
    }

    let mut journal =
        ActionJournal::open(&j_path).map_err(|e| format!("Journal open error: {}", e))?;

    let report =
        reconcile_startup_state(&mut journal).map_err(|e| format!("Recovery error: {}", e))?;

    let mut completed_at_quarantine = 0;
    let mut unmoved_at_source = 0;
    let mut corrupt_or_missing = 0;

    for detail in &report.details {
        match detail.resolution {
            ActionRecoveryResolution::CompletedAtQuarantine => completed_at_quarantine += 1,
            ActionRecoveryResolution::UnmovedAtSource => unmoved_at_source += 1,
            ActionRecoveryResolution::CorruptOrMissing { .. } => corrupt_or_missing += 1,
        }
    }

    Ok(RecoveryReportDto {
        is_clean: report.is_clean,
        interrupted_sessions_found: report.sessions_interrupted,
        reconciled_sessions_count: report.sessions_interrupted,
        completed_at_quarantine_count: completed_at_quarantine,
        unmoved_at_source_count: unmoved_at_source,
        corrupt_or_missing_count: corrupt_or_missing,
    })
}

// ----------------------------------------------------------------------------
// Command 8: Reveal file in native Finder / Explorer
// ----------------------------------------------------------------------------
#[tauri::command]
pub fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to reveal in Finder: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,\"{}\"", path))
            .spawn()
            .map_err(|e| format!("Failed to reveal in Explorer: {}", e))?;
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(parent) = p.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("Failed to open directory: {}", e))?;
        }
    }

    Ok(())
}
