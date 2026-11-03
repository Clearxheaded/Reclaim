//! # Safe Quarantine Execution Engine
//!
//! Implements Milestone 5: safe, reviewable, recoverable quarantine execution with
//! kernel-level non-replacing atomic moves and durable SQLite state machine journaling.
//!
//! ## Operational Flow
//! For every proposed action in a plan:
//! 1. Transition action to `ActionState::Validating` in the SQLite journal.
//! 2. **Precondition Revalidation (TOCTOU defense)**:
//!    - Verify keeper exists, is a regular file, and matches expected size, `FileId`, and content hash.
//!    - Verify source duplicate exists, is a regular file (symlinks rejected), and matches expected size, `FileId`, and content hash.
//!    - Ensure quarantine parent directory exists safely.
//!    - Verify source and quarantine destination reside on the same filesystem device.
//!    - Verify quarantine destination path does not collide with an existing file.
//! 3. If validation fails:
//!    - Transition action to `ActionState::Failed` in the SQLite journal.
//!    - Record failure reason in the audit trail event log.
//!    - Continue or halt according to execution policy.
//! 4. Transition action to `ActionState::Executing` in the SQLite journal (persisting intent before mutation).
//! 5. If `dry_run` is false, execute kernel-level atomic non-replacing move (`atomic_move_no_replace`).
//! 6. If move succeeds (or in `dry_run`): transition action to `ActionState::Completed` in the SQLite journal.
//! 7. If move fails (e.g. `EEXIST` collision): transition action to `ActionState::Failed` in the SQLite journal with OS error details.

use crate::error::StorageError;
use crate::journal::ActionJournal;
use crate::types::{ActionId, ActionState, PlanDetails, PlanId, SessionId};
use reclaim_core::mutation::{atomic_move_no_replace, validate_action_preconditions};
use reclaim_core::planner::{CleanupPlan, QuarantineAction};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Configuration options controlling execution behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecutionOptions {
    /// When true, performs complete pre-validation and journal logging without modifying files.
    pub dry_run: bool,
    /// When true, halts execution on the first action failure rather than continuing with remaining actions.
    pub halt_on_error: bool,
}

/// The final outcome status of an individual quarantine action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionExecutionStatus {
    /// The action completed successfully (file was safely moved, or simulated in dry-run).
    Completed,
    /// The action failed due to precondition mismatch, collision, or I/O error.
    Failed { reason: String },
    /// The action was skipped (e.g. already in terminal state, or halted early).
    Skipped { reason: String },
}

/// Detailed result of executing an individual quarantine action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    /// Unique action identifier in the durable journal.
    pub action_id: ActionId,
    /// Path of the source duplicate file.
    pub source_path: PathBuf,
    /// Staged quarantine destination path.
    pub quarantine_path: PathBuf,
    /// Protected keeper path preserving this content.
    pub keeper_path: PathBuf,
    /// Outcome status of the action.
    pub status: ActionExecutionStatus,
    /// Bytes reclaimed by this action (0 if failed or skipped).
    pub bytes_freed: u64,
}

/// Comprehensive report summarizing an execution session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    /// The session identifier under which execution was performed.
    pub session_id: SessionId,
    /// The plan identifier executed.
    pub plan_id: PlanId,
    /// Whether this run was a dry run.
    pub dry_run: bool,
    /// Total actions evaluated in the plan.
    pub total_actions: usize,
    /// Total actions successfully completed.
    pub completed_actions: usize,
    /// Total actions that failed validation or mutation.
    pub failed_actions: usize,
    /// Total actions that were skipped.
    pub skipped_actions: usize,
    /// Theoretical storage reclaimable by this plan.
    pub total_reclaimable_bytes: u64,
    /// Actual bytes freed during this execution run.
    pub bytes_reclaimed: u64,
    /// Wall-clock duration elapsed during execution.
    pub elapsed: Duration,
    /// Individual action results.
    pub results: Vec<ActionResult>,
}

/// Event emitted during execution for real-time progress monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionProgressEvent<'a> {
    /// Precondition revalidation started.
    Validating {
        action_id: ActionId,
        source: &'a PathBuf,
        destination: &'a PathBuf,
        index: usize,
        total: usize,
    },
    /// Mutation actively in-flight.
    Executing {
        action_id: ActionId,
        source: &'a PathBuf,
        destination: &'a PathBuf,
        dry_run: bool,
    },
    /// Action completed successfully.
    Completed {
        action_id: ActionId,
        source: &'a PathBuf,
        bytes_freed: u64,
        dry_run: bool,
    },
    /// Action failed.
    Failed {
        action_id: ActionId,
        source: &'a PathBuf,
        reason: &'a str,
    },
    /// Action skipped.
    Skipped {
        action_id: ActionId,
        source: &'a PathBuf,
        reason: &'a str,
    },
}

/// Safe execution runner enforcing fail-closed filesystem mutation and durable journaling.
#[derive(Debug, Clone, Default)]
pub struct SafeExecutor {
    options: ExecutionOptions,
}

impl SafeExecutor {
    /// Creates a new `SafeExecutor` with the specified execution options.
    pub fn new(options: ExecutionOptions) -> Self {
        Self { options }
    }

    /// Returns the active execution options.
    pub fn options(&self) -> &ExecutionOptions {
        &self.options
    }

    /// Executes a prospective `CleanupPlan`.
    ///
    /// Automatically registers the plan in the journal (under the active session or creates one),
    /// then executes all actions according to the strict state machine.
    pub fn execute_plan(
        &mut self,
        journal: &mut ActionJournal,
        plan: &CleanupPlan,
    ) -> Result<ExecutionReport, StorageError> {
        self.execute_plan_with_callback(journal, plan, |_| {})
    }

    /// Executes a prospective `CleanupPlan` with a progress callback.
    pub fn execute_plan_with_callback<F>(
        &mut self,
        journal: &mut ActionJournal,
        plan: &CleanupPlan,
        callback: F,
    ) -> Result<ExecutionReport, StorageError>
    where
        F: FnMut(ExecutionProgressEvent<'_>),
    {
        let session_id = match journal.active_session_id() {
            Some(id) => id,
            None => journal.create_session(&plan.root_dir, &plan.policy)?,
        };

        let plan_id = journal.save_plan_for_session(session_id, plan)?;
        let plan_details = journal.fetch_plan(plan_id)?;
        self.execute_plan_details(journal, &plan_details, callback)
    }

    /// Executes a previously persisted `PlanDetails` from the journal.
    pub fn execute_persisted_plan(
        &mut self,
        journal: &mut ActionJournal,
        plan_details: &PlanDetails,
    ) -> Result<ExecutionReport, StorageError> {
        self.execute_persisted_plan_with_callback(journal, plan_details, |_| {})
    }

    /// Executes a previously persisted `PlanDetails` with a progress callback.
    pub fn execute_persisted_plan_with_callback<F>(
        &mut self,
        journal: &mut ActionJournal,
        plan_details: &PlanDetails,
        callback: F,
    ) -> Result<ExecutionReport, StorageError>
    where
        F: FnMut(ExecutionProgressEvent<'_>),
    {
        self.execute_plan_details(journal, plan_details, callback)
    }

    fn execute_plan_details<F>(
        &mut self,
        journal: &mut ActionJournal,
        plan_details: &PlanDetails,
        mut callback: F,
    ) -> Result<ExecutionReport, StorageError>
    where
        F: FnMut(ExecutionProgressEvent<'_>),
    {
        let start_time = Instant::now();
        let total_actions = plan_details.actions.len();
        let mut completed_actions = 0;
        let mut failed_actions = 0;
        let mut skipped_actions = 0;
        let mut bytes_reclaimed = 0u64;
        let mut results = Vec::with_capacity(total_actions);

        for (idx, action) in plan_details.actions.iter().enumerate() {
            // If action is already in a terminal state (e.g. earlier completed or reverted), skip it
            if action.state.is_terminal() {
                let reason = format!("Action already in terminal state '{}'", action.state);
                callback(ExecutionProgressEvent::Skipped {
                    action_id: action.action_id,
                    source: &action.source_path,
                    reason: &reason,
                });
                results.push(ActionResult {
                    action_id: action.action_id,
                    source_path: action.source_path.clone(),
                    quarantine_path: action.quarantine_path.clone(),
                    keeper_path: action.keeper_path.clone(),
                    status: ActionExecutionStatus::Skipped { reason },
                    bytes_freed: 0,
                });
                skipped_actions += 1;
                continue;
            }

            // Step 1: Transition action to Validating
            callback(ExecutionProgressEvent::Validating {
                action_id: action.action_id,
                source: &action.source_path,
                destination: &action.quarantine_path,
                index: idx + 1,
                total: total_actions,
            });

            journal.record_action_transition(
                action.action_id,
                ActionState::Validating,
                Some("Revalidating preconditions (TOCTOU defense)"),
            )?;

            // Step 2: Precondition Revalidation
            let core_action = QuarantineAction {
                source_path: action.source_path.clone(),
                quarantine_path: action.quarantine_path.clone(),
                keeper_path: action.keeper_path.clone(),
                preconditions: action.preconditions.clone(),
                rationale: action.rationale.clone(),
            };

            let validation_result = validate_action_preconditions(&core_action);

            // Step 3: Handle Validation Failure
            if let Err(precond_err) = validation_result {
                let reason = precond_err.to_string();
                callback(ExecutionProgressEvent::Failed {
                    action_id: action.action_id,
                    source: &action.source_path,
                    reason: &reason,
                });

                journal.record_action_transition(
                    action.action_id,
                    ActionState::Failed,
                    Some(&format!("Precondition failure: {}", reason)),
                )?;

                results.push(ActionResult {
                    action_id: action.action_id,
                    source_path: action.source_path.clone(),
                    quarantine_path: action.quarantine_path.clone(),
                    keeper_path: action.keeper_path.clone(),
                    status: ActionExecutionStatus::Failed { reason },
                    bytes_freed: 0,
                });
                failed_actions += 1;

                if self.options.halt_on_error {
                    break;
                }
                continue;
            }

            // Step 4: Transition action to Executing (persisting intent before mutation)
            callback(ExecutionProgressEvent::Executing {
                action_id: action.action_id,
                source: &action.source_path,
                destination: &action.quarantine_path,
                dry_run: self.options.dry_run,
            });

            journal.record_action_transition(
                action.action_id,
                ActionState::Executing,
                Some("Preconditions verified; persisting intent before mutation"),
            )?;

            // Step 5: Execution / Dry-Run Simulation
            if self.options.dry_run {
                // Step 6 (dry run): Transition to Completed with simulation note
                journal.record_action_transition(
                    action.action_id,
                    ActionState::Completed,
                    Some("Dry run: simulation completed successfully; no files were moved"),
                )?;

                callback(ExecutionProgressEvent::Completed {
                    action_id: action.action_id,
                    source: &action.source_path,
                    bytes_freed: action.preconditions.expected_size,
                    dry_run: true,
                });

                results.push(ActionResult {
                    action_id: action.action_id,
                    source_path: action.source_path.clone(),
                    quarantine_path: action.quarantine_path.clone(),
                    keeper_path: action.keeper_path.clone(),
                    status: ActionExecutionStatus::Completed,
                    bytes_freed: action.preconditions.expected_size,
                });

                completed_actions += 1;
                bytes_reclaimed += action.preconditions.expected_size;
            } else {
                // Safely create intermediate quarantine destination directory
                if let Some(parent) = action.quarantine_path.parent() {
                    if !parent.exists() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            let reason = format!("Failed to create quarantine directory: {}", e);
                            callback(ExecutionProgressEvent::Failed {
                                action_id: action.action_id,
                                source: &action.source_path,
                                reason: &reason,
                            });
                            journal.record_action_transition(
                                action.action_id,
                                ActionState::Failed,
                                Some(&reason),
                            )?;
                            results.push(ActionResult {
                                action_id: action.action_id,
                                source_path: action.source_path.clone(),
                                quarantine_path: action.quarantine_path.clone(),
                                keeper_path: action.keeper_path.clone(),
                                status: ActionExecutionStatus::Failed { reason },
                                bytes_freed: 0,
                            });
                            failed_actions += 1;
                            if self.options.halt_on_error {
                                break;
                            }
                            continue;
                        }
                    }
                }

                // Perform physical atomic non-replacing move
                let move_result =
                    atomic_move_no_replace(&action.source_path, &action.quarantine_path);

                match move_result {
                    Ok(()) => {
                        // Step 6: Transition to Completed
                        journal.record_action_transition(
                            action.action_id,
                            ActionState::Completed,
                            Some("Quarantined successfully via atomic non-replacing move"),
                        )?;

                        callback(ExecutionProgressEvent::Completed {
                            action_id: action.action_id,
                            source: &action.source_path,
                            bytes_freed: action.preconditions.expected_size,
                            dry_run: false,
                        });

                        results.push(ActionResult {
                            action_id: action.action_id,
                            source_path: action.source_path.clone(),
                            quarantine_path: action.quarantine_path.clone(),
                            keeper_path: action.keeper_path.clone(),
                            status: ActionExecutionStatus::Completed,
                            bytes_freed: action.preconditions.expected_size,
                        });

                        completed_actions += 1;
                        bytes_reclaimed += action.preconditions.expected_size;
                    }
                    Err(move_err) => {
                        // Step 7: Transition to Failed
                        let reason = format!("Atomic move failed: {}", move_err);
                        callback(ExecutionProgressEvent::Failed {
                            action_id: action.action_id,
                            source: &action.source_path,
                            reason: &reason,
                        });

                        journal.record_action_transition(
                            action.action_id,
                            ActionState::Failed,
                            Some(&reason),
                        )?;

                        results.push(ActionResult {
                            action_id: action.action_id,
                            source_path: action.source_path.clone(),
                            quarantine_path: action.quarantine_path.clone(),
                            keeper_path: action.keeper_path.clone(),
                            status: ActionExecutionStatus::Failed { reason },
                            bytes_freed: 0,
                        });

                        failed_actions += 1;

                        if self.options.halt_on_error {
                            break;
                        }
                    }
                }
            }
        }

        // If all evaluated actions completed without failures, mark session Completed
        if failed_actions == 0 {
            let _ = journal.complete_session(plan_details.session_id);
        }

        let elapsed = start_time.elapsed();

        Ok(ExecutionReport {
            session_id: plan_details.session_id,
            plan_id: plan_details.plan_id,
            dry_run: self.options.dry_run,
            total_actions,
            completed_actions,
            failed_actions,
            skipped_actions,
            total_reclaimable_bytes: plan_details.summary.total_reclaimable_bytes,
            bytes_reclaimed,
            elapsed,
            results,
        })
    }
}
