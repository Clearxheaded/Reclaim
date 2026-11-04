import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  CleanupPlanPayload,
  DuplicateGroupPayload,
  DuplicateOutputPayload,
  DuplicateProgressEvent,
  ExecutionOptionsPayload,
  ExecutionProgressEventPayload,
  ExecutionReportPayload,
  KeeperPolicy,
  RecoveryReportPayload,
  ScanProgressEvent,
  SessionHistoryItem,
  UndoOptionsPayload,
  UndoProgressEventPayload,
  UndoReportPayload,
} from '../types/reclaim';

// Check if running inside Tauri desktop environment
export const isTauri = (): boolean => {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
};

// Select directory using native OS dialog
export async function pickDirectory(): Promise<string | null> {
  if (isTauri()) {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Select Folder to Scan for Duplicates',
      });
      if (typeof selected === 'string') {
        return selected;
      }
      return null;
    } catch (err) {
      console.error('Failed to open directory picker:', err);
      return null;
    }
  }

  // Fallback for browser preview
  const demoPath = prompt('Enter a directory path to scan:', '/Volumes/Data/Projects');
  return demoPath || null;
}

// Reveal file in native Finder / Explorer
export async function revealInFileManager(filePath: string): Promise<void> {
  if (isTauri()) {
    await invoke('reveal_in_file_manager', { path: filePath });
    return;
  }
  console.log('[Mock IPC] Reveal in file manager:', filePath);
}

// Cancel in-flight scan
export async function cancelScan(): Promise<void> {
  if (isTauri()) {
    await invoke('cancel_scan');
    return;
  }
  console.log('[Mock IPC] Scan cancelled');
}

// Start recursive scan and two-stage BLAKE3 duplicate detection
export async function scanAndFindDuplicates(
  rootPath: string,
  includeEmpty: boolean,
  onScanProgress?: (event: ScanProgressEvent) => void,
  onDuplicateProgress?: (event: DuplicateProgressEvent) => void
): Promise<DuplicateOutputPayload> {
  let unlistenScan: UnlistenFn | undefined;
  let unlistenDup: UnlistenFn | undefined;

  if (isTauri()) {
    if (onScanProgress) {
      unlistenScan = await listen<ScanProgressEvent>('reclaim://scan-progress', (e) => {
        onScanProgress(e.payload);
      });
    }
    if (onDuplicateProgress) {
      unlistenDup = await listen<DuplicateProgressEvent>('reclaim://duplicate-progress', (e) => {
        onDuplicateProgress(e.payload);
      });
    }

    try {
      const result = await invoke<DuplicateOutputPayload>('scan_and_find_duplicates', {
        rootPath,
        options: { include_empty: includeEmpty },
      });
      return result;
    } finally {
      if (unlistenScan) unlistenScan();
      if (unlistenDup) unlistenDup();
    }
  }

  // --- Browser Mock Implementation for UI Preview ---
  console.log('[Mock IPC] Running simulated scan on:', rootPath);
  return simulateMockScan(rootPath, onScanProgress, onDuplicateProgress);
}

// Generate prospective cleanup plan without modifying files
export async function generateCleanupPlan(
  rootDir: string,
  quarantineDir: string | undefined,
  policy: KeeperPolicy,
  duplicateGroups: DuplicateGroupPayload[]
): Promise<CleanupPlanPayload> {
  if (isTauri()) {
    return await invoke<CleanupPlanPayload>('generate_cleanup_plan', {
      rootDir,
      quarantineDir,
      policy,
      duplicateGroups,
    });
  }

  // Browser Mock
  return simulateMockPlan(rootDir, quarantineDir, duplicateGroups);
}

// Execute safe quarantine plan with atomic non-replace moves
export async function executeQuarantinePlan(
  plan: CleanupPlanPayload,
  options: ExecutionOptionsPayload,
  onProgress?: (event: ExecutionProgressEventPayload) => void
): Promise<ExecutionReportPayload> {
  let unlistenExec: UnlistenFn | undefined;

  if (isTauri()) {
    if (onProgress) {
      unlistenExec = await listen<ExecutionProgressEventPayload>('reclaim://execution-progress', (e) => {
        onProgress(e.payload);
      });
    }

    try {
      return await invoke<ExecutionReportPayload>('execute_quarantine_plan', {
        plan,
        options,
      });
    } finally {
      if (unlistenExec) unlistenExec();
    }
  }

  // Browser Mock
  return simulateMockExecution(plan, options, onProgress);
}

// Revert previous session conditionally
export async function undoSession(
  sessionId?: string,
  options?: UndoOptionsPayload,
  onProgress?: (event: UndoProgressEventPayload) => void
): Promise<UndoReportPayload> {
  let unlistenUndo: UnlistenFn | undefined;

  if (isTauri()) {
    if (onProgress) {
      unlistenUndo = await listen<UndoProgressEventPayload>('reclaim://undo-progress', (e) => {
        onProgress(e.payload);
      });
    }

    try {
      return await invoke<UndoReportPayload>('undo_session', {
        sessionId: sessionId || null,
        options: options || { dry_run: false, halt_on_error: false },
      });
    } finally {
      if (unlistenUndo) unlistenUndo();
    }
  }

  // Browser Mock
  return simulateMockUndo(sessionId);
}

// Fetch session history from journal
export async function getSessionHistory(journalPath?: string): Promise<SessionHistoryItem[]> {
  if (isTauri()) {
    return await invoke<SessionHistoryItem[]>('get_session_history', {
      journalPath: journalPath || null,
    });
  }

  // Browser Mock
  return mockHistoryItems;
}

// Reconcile startup state / interrupted sessions
export async function recoverJournal(journalPath?: string): Promise<RecoveryReportPayload> {
  if (isTauri()) {
    return await invoke<RecoveryReportPayload>('recover_journal', {
      journalPath: journalPath || null,
    });
  }

  return {
    interrupted_sessions_found: 1,
    reconciled_sessions_count: 1,
    completed_at_quarantine_count: 1,
    unmoved_at_source_count: 0,
    corrupt_or_missing_count: 0,
  };
}

// ============================================================================
// Realistic Mock Simulators for Browser / Headless Prototyping
// ============================================================================

async function simulateMockScan(
  root: string,
  onScanProgress?: (event: ScanProgressEvent) => void,
  onDuplicateProgress?: (event: DuplicateProgressEvent) => void
): Promise<DuplicateOutputPayload> {
  // Step 1: Scan
  for (let i = 1; i <= 5; i++) {
    await new Promise((r) => setTimeout(r, 60));
    if (onScanProgress) {
      onScanProgress({
        entries_scanned: i * 800,
        files_scanned: i * 650,
        directories_scanned: i * 150,
        symlinks_skipped: 4,
        bytes_scanned: i * 1024 * 1024 * 750,
        current_path: `${root}/Project_Assets/Archive_2026/chunk_${i}.bin`,
      });
    }
  }

  // Step 2: Hashing
  for (let i = 1; i <= 4; i++) {
    await new Promise((r) => setTimeout(r, 70));
    if (onDuplicateProgress) {
      onDuplicateProgress({
        stage: i <= 2 ? 'PrefixHashing' : 'FullHashing',
        current_step: i * 25,
        total_steps: 100,
        bytes_processed: i * 600 * 1024 * 1024,
        total_bytes: 2400 * 1024 * 1024,
        current_file: `${root}/Video_B-Roll/Footage_0${i}.mp4`,
      });
    }
  }

  return {
    groups: [
      {
        size: 1420584960, // ~1.42 GB
        blake3_hash: '9f82c4e207d3910c80b67812fa92c3d18201bfa8e469e35b7194f1c9447e1122',
        total_physical_copies: 2,
        total_path_count: 3,
        reclaimable_bytes: 1420584960,
        physical_copies: [
          {
            device_id: 16777234,
            inode: 894321,
            primary_path: `${root}/Originals/Cinematic_Reel_4K.mov`,
            all_paths: [`${root}/Originals/Cinematic_Reel_4K.mov`],
          },
          {
            device_id: 16777234,
            inode: 894322,
            primary_path: `${root}/Backup_Copy/Cinematic_Reel_4K_Copy.mov`,
            all_paths: [
              `${root}/Backup_Copy/Cinematic_Reel_4K_Copy.mov`,
              `${root}/Staging/Hardlink_Alias_Reel.mov`,
            ],
          },
        ],
      },
      {
        size: 894201856, // ~894 MB
        blake3_hash: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
        total_physical_copies: 2,
        total_path_count: 2,
        reclaimable_bytes: 894201856,
        physical_copies: [
          {
            device_id: 16777234,
            inode: 902144,
            primary_path: `${root}/Renders/Final_Cut_v1.mp4`,
            all_paths: [`${root}/Renders/Final_Cut_v1.mp4`],
          },
          {
            device_id: 16777234,
            inode: 902145,
            primary_path: `${root}/Downloads/Final_Cut_v1(1).mp4`,
            all_paths: [`${root}/Downloads/Final_Cut_v1(1).mp4`],
          },
        ],
      },
      {
        size: 245760120, // ~245 MB
        blake3_hash: '2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae',
        total_physical_copies: 3,
        total_path_count: 3,
        reclaimable_bytes: 491520240, // 2 copies reclaimable
        physical_copies: [
          {
            device_id: 16777234,
            inode: 911002,
            primary_path: `${root}/Datasets/raw_training_corpus.tar.gz`,
            all_paths: [`${root}/Datasets/raw_training_corpus.tar.gz`],
          },
          {
            device_id: 16777234,
            inode: 911003,
            primary_path: `${root}/ML_Experiments/backup_corpus.tar.gz`,
            all_paths: [`${root}/ML_Experiments/backup_corpus.tar.gz`],
          },
          {
            device_id: 16777234,
            inode: 911004,
            primary_path: `${root}/Temp_Exports/corpus_archive.tar.gz`,
            all_paths: [`${root}/Temp_Exports/corpus_archive.tar.gz`],
          },
        ],
      },
    ],
    summary: {
      total_files_scanned: 3250,
      duplicate_groups_count: 3,
      duplicate_files_count: 8,
      reclaimable_bytes: 2806307056, // ~2.81 GB
      total_duplicate_bytes: 4226892016,
      hard_link_savings_bytes: 1420584960,
    },
  };
}

function simulateMockPlan(
  rootDir: string,
  quarantineDir: string | undefined,
  groups: DuplicateGroupPayload[]
): CleanupPlanPayload {
  const qDir = quarantineDir || `${rootDir}/.reclaim/quarantine`;
  const actions = [];
  let totalBytes = 0;

  for (const group of groups) {
    if (group.physical_copies.length < 2) continue;
    const keeper = group.physical_copies[0];
    for (let i = 1; i < group.physical_copies.length; i++) {
      const copy = group.physical_copies[i];
      actions.push({
        action_id: `act_${Math.random().toString(36).substring(2, 9)}`,
        source_path: copy.primary_path,
        quarantine_path: `${qDir}/${copy.primary_path.split('/').pop()}`,
        keeper_path: keeper.primary_path,
        preconditions: {
          expected_size: group.size,
          expected_blake3_hash: group.blake3_hash,
          expected_device_id: copy.device_id,
          expected_inode: copy.inode,
          expected_nlink: copy.all_paths.length,
          expected_modified: Math.floor(Date.now() / 1000) - 3600,
        },
      });
      totalBytes += group.size;
    }
  }

  return {
    plan_id: `plan_${Math.random().toString(36).substring(2, 10)}`,
    root_dir: rootDir,
    quarantine_dir: qDir,
    created_at: new Date().toISOString(),
    actions,
    summary: {
      total_actions: actions.length,
      total_bytes_to_quarantine: totalBytes,
      protected_keepers_count: groups.length,
    },
  };
}

async function simulateMockExecution(
  plan: CleanupPlanPayload,
  options: ExecutionOptionsPayload,
  onProgress?: (event: ExecutionProgressEventPayload) => void
): Promise<ExecutionReportPayload> {
  const results = [];
  let completed = 0;

  for (let i = 0; i < plan.actions.length; i++) {
    const act = plan.actions[i];
    if (onProgress) {
      onProgress({
        action_id: act.action_id,
        source_path: act.source_path,
        quarantine_path: act.quarantine_path,
        state: 'Validating',
        completed_count: completed,
        total_count: plan.actions.length,
      });
      await new Promise((r) => setTimeout(r, 60));
      onProgress({
        action_id: act.action_id,
        source_path: act.source_path,
        quarantine_path: act.quarantine_path,
        state: 'Executing',
        completed_count: completed,
        total_count: plan.actions.length,
      });
      await new Promise((r) => setTimeout(r, 80));
      onProgress({
        action_id: act.action_id,
        source_path: act.source_path,
        quarantine_path: act.quarantine_path,
        state: 'Completed',
        completed_count: completed + 1,
        total_count: plan.actions.length,
      });
    }
    completed++;
    results.push({
      action_id: act.action_id,
      source_path: act.source_path,
      quarantine_path: act.quarantine_path,
      status: 'Completed' as const,
    });
  }

  return {
    session_id: `sess_${Math.random().toString(36).substring(2, 10)}`,
    plan_id: plan.plan_id,
    total_actions: plan.actions.length,
    completed_actions: completed,
    failed_actions: 0,
    bytes_quarantined: plan.summary.total_bytes_to_quarantine,
    duration_ms: 450,
    dry_run: options.dry_run,
    results,
  };
}

async function simulateMockUndo(sessionId?: string): Promise<UndoReportPayload> {
  await new Promise((r) => setTimeout(r, 400));
  return {
    session_id: sessionId || 'sess_demo_1',
    restored_count: 3,
    failed_count: 0,
    skipped_count: 0,
    restored_bytes: 2806307056,
    duration_ms: 120,
    dry_run: false,
    conflicts: [],
  };
}

const mockHistoryItems: SessionHistoryItem[] = [
  {
    session_id: 'e6b4f738-921a-4d92-bb52-094192b0c144',
    plan_id: 'c12e9871-33da-4712-9c10-82a170fb21e4',
    root_path: '/Volumes/Data/Projects',
    started_at: '2026-10-31T14:15:22Z',
    completed_at: '2026-10-31T14:15:24Z',
    status: 'Completed',
    total_actions: 5,
    completed_actions: 5,
    reverted_actions: 0,
    failed_actions: 0,
    quarantined_bytes: 3820491820,
    restored_bytes: 0,
  },
  {
    session_id: 'b110a293-8f04-4b53-a189-53892019ff81',
    plan_id: '809214ab-3019-4822-b011-cc7291a03491',
    root_path: '/Volumes/Data/Downloads',
    started_at: '2026-10-28T09:30:10Z',
    completed_at: '2026-10-28T09:30:11Z',
    status: 'Reverted',
    total_actions: 2,
    completed_actions: 2,
    reverted_actions: 2,
    failed_actions: 0,
    quarantined_bytes: 145029100,
    restored_bytes: 145029100,
  },
];

