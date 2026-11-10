import React, { useState, useEffect, useMemo } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Sidebar } from './components/Sidebar';
import { DirectoryPicker } from './components/DirectoryPicker';
import { ScanProgressView } from './components/ScanProgressView';
import { DuplicateInspector } from './components/DuplicateInspector';
import { SafetyActionBar } from './components/SafetyActionBar';
import { PlanReviewModal } from './components/PlanReviewModal';
import { ExecutionMonitor } from './components/ExecutionMonitor';
import { HistoryRecoveryCenter } from './components/HistoryRecoveryCenter';
import { SafetyInvariantsModal } from './components/SafetyInvariantsModal';
import {
  ActiveTab,
  CandidateInvalidatedEvent,
  CleanupPlanPayload,
  DuplicateGroupPayload,
  DuplicateOutputPayload,
  DuplicateProgressEvent,
  ExecutionOptionsPayload,
  ExecutionProgressEventPayload,
  ExecutionReportPayload,
  InvalidationToastItem,
  KeeperPolicy,
  ScanProgressEvent,
  SessionHistoryItem,
} from './types/reclaim';
import {
  cancelScan,
  executeQuarantinePlan,
  generateCleanupPlan,
  getSessionHistory,
  isTauri,
  pickDirectory,
  scanAndFindDuplicates,
} from './api/ipc';

export const App: React.FC = () => {
  const [activeTab, setActiveTab] = useState<ActiveTab>('scan');
  const [targetDir, setTargetDir] = useState<string | null>(null);

  // Scan state
  const [isScanning, setIsScanning] = useState<boolean>(false);
  const [scanProgress, setScanProgress] = useState<ScanProgressEvent | null>(null);
  const [duplicateProgress, setDuplicateProgress] = useState<DuplicateProgressEvent | null>(null);
  const [duplicateOutput, setDuplicateOutput] = useState<DuplicateOutputPayload | null>(null);

  // Duplicate Inspector, Keeper Overrides, & Real-Time Invalidation / CoW State
  const [keeperOverrides, setKeeperOverrides] = useState<Map<string, string>>(new Map());
  const [keeperPolicy, setKeeperPolicy] = useState<KeeperPolicy>({ type: 'ShortestPath' });
  const [invalidatedFiles, setInvalidatedFiles] = useState<Map<string, CandidateInvalidatedEvent>>(
    new Map()
  );
  const [invalidationToasts, setInvalidationToasts] = useState<InvalidationToastItem[]>([]);
  const [cowDeduplicatedPaths, setCowDeduplicatedPaths] = useState<Set<string>>(new Set());

  // Plan & Execution State
  const [currentPlan, setCurrentPlan] = useState<CleanupPlanPayload | null>(null);
  const [isGeneratingPlan, setIsGeneratingPlan] = useState<boolean>(false);
  const [showPlanModal, setShowPlanModal] = useState<boolean>(false);
  const [isExecuting, setIsExecuting] = useState<boolean>(false);
  const [execProgress, setExecProgress] = useState<ExecutionProgressEventPayload | null>(null);
  const [execReport, setExecReport] = useState<ExecutionReportPayload | null>(null);

  // History State
  const [sessionHistory, setSessionHistory] = useState<SessionHistoryItem[]>([]);

  // Initial load of session history
  useEffect(() => {
    refreshHistory();
  }, []);

  // Active TOCTOU Defense: listen for kernel filesystem invalidations across all tabs
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;
    let active = true;

    listen<CandidateInvalidatedEvent>('reclaim://candidate-invalidated', (event) => {
      if (!active) return;
      const notice = event.payload;

      setInvalidatedFiles((prev) => {
        const next = new Map(prev);
        next.set(notice.path, notice);
        return next;
      });

      const toastId = `${Date.now()}-${Math.random()}`;
      setInvalidationToasts((prev) => [
        ...prev.slice(-2),
        { id: toastId, notice, timestamp: Date.now() },
      ]);
      setTimeout(() => {
        setInvalidationToasts((prev) => prev.filter((t) => t.id !== toastId));
      }, 6000);

      setDuplicateOutput((prev) => {
        if (!prev) return prev;
        const isGroupDissolved = notice.remaining_files_in_group <= 1;
        return {
          ...prev,
          summary: {
            ...prev.summary,
            reclaimable_bytes: notice.new_reclaimable_bytes,
            duplicate_files_count: Math.max(0, prev.summary.duplicate_files_count - 1),
            duplicate_groups_count: isGroupDissolved
              ? Math.max(0, prev.summary.duplicate_groups_count - 1)
              : prev.summary.duplicate_groups_count,
          },
        };
      });

      setCurrentPlan(null);
    }).then((unsub) => {
      if (active) {
        unlisten = unsub;
      } else {
        unsub();
      }
    });

    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, []);

  const refreshHistory = async () => {
    try {
      const history = await getSessionHistory();
      setSessionHistory(history);
    } catch (err) {
      console.warn('Could not fetch session history:', err);
    }
  };

  const resetScanDerivedState = () => {
    setDuplicateOutput(null);
    setCurrentPlan(null);
    setExecReport(null);
    setKeeperOverrides(new Map());
    setInvalidatedFiles(new Map());
    setInvalidationToasts([]);
    setCowDeduplicatedPaths(new Set());
  };

  const handleSelectDirectory = async () => {
    const selected = await pickDirectory();
    if (selected) {
      setTargetDir(selected);
      resetScanDerivedState();
    }
  };

  const handleStartScan = async (includeEmpty: boolean, policy: KeeperPolicy) => {
    if (!targetDir) return;

    setIsScanning(true);
    setScanProgress(null);
    setDuplicateProgress(null);
    resetScanDerivedState();
    setKeeperPolicy(policy);

    try {
      const result = await scanAndFindDuplicates(
        targetDir,
        includeEmpty,
        (sp) => setScanProgress(sp),
        (dp) => setDuplicateProgress(dp)
      );
      setDuplicateOutput(result);
    } catch (err) {
      alert(`Scan encountered an error: ${err}`);
    } finally {
      setIsScanning(false);
    }
  };

  const handleCancelScan = async () => {
    await cancelScan();
    setIsScanning(false);
  };

  const handleSetKeeper = (groupHash: string, keeperPath: string) => {
    const next = new Map(keeperOverrides);
    next.set(groupHash, keeperPath);
    setKeeperOverrides(next);
    // Invalidate previously computed plan to require fresh preview
    setCurrentPlan(null);
  };

  const handleCowDeduplicated = (dedupedPaths: string[], totalReclaimed: number) => {
    setCowDeduplicatedPaths((prev) => {
      const next = new Set(prev);
      dedupedPaths.forEach((p) => next.add(p));
      return next;
    });
    setDuplicateOutput((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        summary: {
          ...prev.summary,
          reclaimable_bytes: Math.max(0, prev.summary.reclaimable_bytes - totalReclaimed),
          hard_link_savings_bytes: prev.summary.hard_link_savings_bytes + totalReclaimed,
        },
      };
    });
    setCurrentPlan(null);
  };

  // Filter out watcher-invalidated and CoW-deduplicated candidates before planning & action counts
  const eligibleGroupsForPlanning = useMemo<DuplicateGroupPayload[]>(() => {
    if (!duplicateOutput) return [];

    const nextGroups: DuplicateGroupPayload[] = [];
    for (const group of duplicateOutput.groups) {
      const eligibleCopies = group.physical_copies.filter((copy) => {
        const isInvalidated =
          invalidatedFiles.has(copy.primary_path) ||
          copy.all_paths.some((p) => invalidatedFiles.has(p));
        const isCowDeduped = cowDeduplicatedPaths.has(copy.primary_path);
        return !isInvalidated && !isCowDeduped;
      });

      if (eligibleCopies.length < 2) {
        continue;
      }

      const copies = [...eligibleCopies];
      const override = keeperOverrides.get(group.blake3_hash);
      if (override) {
        const foundIdx = copies.findIndex((c) => c.primary_path === override);
        if (foundIdx > 0) {
          const [selected] = copies.splice(foundIdx, 1);
          copies.unshift(selected);
        }
      }

      const totalPathCount = copies.reduce((sum, c) => sum + c.all_paths.length, 0);
      nextGroups.push({
        ...group,
        physical_copies: copies,
        total_physical_copies: copies.length,
        total_path_count: totalPathCount,
        reclaimable_bytes: Math.max(0, copies.length - 1) * group.size,
      });
    }

    return nextGroups;
  }, [duplicateOutput, invalidatedFiles, cowDeduplicatedPaths, keeperOverrides]);

  const handleGeneratePlan = async () => {
    if (!targetDir || !duplicateOutput || eligibleGroupsForPlanning.length === 0) return;

    setIsGeneratingPlan(true);
    try {
      const plan = await generateCleanupPlan(
        targetDir,
        undefined,
        keeperPolicy,
        eligibleGroupsForPlanning
      );
      setCurrentPlan(plan);
      setShowPlanModal(true);
    } catch (err) {
      alert(`Failed to generate prospective plan: ${err}`);
    } finally {
      setIsGeneratingPlan(false);
    }
  };

  const handleExecutePlan = async (options: ExecutionOptionsPayload) => {
    if (!currentPlan) return;

    setIsExecuting(true);
    setShowPlanModal(false);
    setActiveTab('monitor');
    setExecProgress(null);
    setExecReport(null);

    try {
      const report = await executeQuarantinePlan(currentPlan, options, (prog) => {
        setExecProgress(prog);
      });
      setExecReport(report);
      refreshHistory();
    } catch (err) {
      alert(`Execution failed: ${err}`);
    } finally {
      setIsExecuting(false);
    }
  };

  // Compute total actions and reclaimable bytes strictly from eligible candidates
  const totalProposedActions = eligibleGroupsForPlanning.reduce(
    (acc, g) => acc + Math.max(0, g.physical_copies.length - 1),
    0
  );
  const eligibleReclaimableBytes = eligibleGroupsForPlanning.reduce(
    (acc, g) => acc + g.reclaimable_bytes,
    0
  );

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-canvas text-gray-100 font-sans">
      {/* Primary Sidebar */}
      <Sidebar
        activeTab={activeTab}
        onTabChange={(tab) => setActiveTab(tab)}
        targetDir={targetDir}
        onSelectDir={handleSelectDirectory}
        summary={duplicateOutput?.summary || null}
        hasPlan={!!currentPlan}
        isScanning={isScanning}
      />

      {/* Main Content Viewport */}
      <main className="flex-1 flex flex-col overflow-hidden relative">
        {activeTab === 'scan' && (
          <>
            {isScanning ? (
              <ScanProgressView
                scanProgress={scanProgress}
                duplicateProgress={duplicateProgress}
                onCancel={handleCancelScan}
              />
            ) : !duplicateOutput ? (
              <DirectoryPicker
                targetDir={targetDir}
                onSelectDir={handleSelectDirectory}
                onStartScan={handleStartScan}
                isScanning={isScanning}
              />
            ) : (
              <div className="flex-1 flex flex-col overflow-hidden">
                <DuplicateInspector
                  groups={duplicateOutput.groups}
                  summary={duplicateOutput.summary}
                  totalScannedBytes={scanProgress?.bytes_scanned}
                  totalScannedFiles={scanProgress?.files_scanned}
                  targetDir={targetDir}
                  keeperOverrides={keeperOverrides}
                  invalidatedFiles={invalidatedFiles}
                  cowDeduplicatedPaths={cowDeduplicatedPaths}
                  invalidationToasts={invalidationToasts}
                  onDismissToast={(toastId) =>
                    setInvalidationToasts((prev) => prev.filter((t) => t.id !== toastId))
                  }
                  onSetKeeper={handleSetKeeper}
                  onCowDeduplicated={handleCowDeduplicated}
                />
                <SafetyActionBar
                  totalActions={totalProposedActions}
                  reclaimableBytes={eligibleReclaimableBytes}
                  hardLinkSavings={duplicateOutput.summary.hard_link_savings_bytes}
                  policy={keeperPolicy}
                  onPolicyChange={setKeeperPolicy}
                  onGeneratePlan={handleGeneratePlan}
                  isGenerating={isGeneratingPlan}
                />
              </div>
            )}
          </>
        )}

        {activeTab === 'plan' && currentPlan && (
          <div className="flex-1 flex items-center justify-center p-8">
            <div className="text-center space-y-3">
              <h3 className="text-sm font-semibold text-white">Cleanup Plan Ready for Review</h3>
              <p className="text-xs text-gray-400">
                Plan ID: {currentPlan.plan_id} — {currentPlan.summary.total_actions} actions
                staged.
              </p>
              <button
                onClick={() => setShowPlanModal(true)}
                className="px-4 py-2 rounded-lg bg-sky-500 hover:bg-sky-400 text-white text-xs font-semibold"
              >
                Open Review Modal
              </button>
            </div>
          </div>
        )}

        {activeTab === 'monitor' && (
          <ExecutionMonitor
            isExecuting={isExecuting}
            progress={execProgress}
            report={execReport}
            onOpenHistory={() => setActiveTab('history')}
            onNewScan={() => {
              resetScanDerivedState();
              setActiveTab('scan');
            }}
          />
        )}

        {activeTab === 'history' && (
          <HistoryRecoveryCenter
            history={sessionHistory}
            onRefreshHistory={refreshHistory}
          />
        )}

        {activeTab === 'invariants' && <SafetyInvariantsModal />}

        {/* Explicit Plan Approval Modal */}
        {showPlanModal && currentPlan && (
          <PlanReviewModal
            plan={currentPlan}
            onClose={() => setShowPlanModal(false)}
            onExecute={handleExecutePlan}
            isExecuting={isExecuting}
          />
        )}
      </main>
    </div>
  );
};
