import React, { useState, useEffect } from 'react';
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
  CleanupPlanPayload,
  DuplicateOutputPayload,
  DuplicateProgressEvent,
  ExecutionOptionsPayload,
  ExecutionProgressEventPayload,
  ExecutionReportPayload,
  KeeperPolicy,
  ScanProgressEvent,
  SessionHistoryItem,
} from './types/reclaim';
import {
  cancelScan,
  executeQuarantinePlan,
  generateCleanupPlan,
  getSessionHistory,
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

  // Duplicate Inspector & Keeper Overrides
  const [keeperOverrides, setKeeperOverrides] = useState<Map<string, string>>(new Map());
  const [keeperPolicy, setKeeperPolicy] = useState<KeeperPolicy>({ type: 'ShortestPath' });

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

  const refreshHistory = async () => {
    try {
      const history = await getSessionHistory();
      setSessionHistory(history);
    } catch (err) {
      console.warn('Could not fetch session history:', err);
    }
  };

  const handleSelectDirectory = async () => {
    const selected = await pickDirectory();
    if (selected) {
      setTargetDir(selected);
      // Reset previous results if switching directory
      setDuplicateOutput(null);
      setCurrentPlan(null);
      setExecReport(null);
      setKeeperOverrides(new Map());
    }
  };

  const handleStartScan = async (includeEmpty: boolean, policy: KeeperPolicy) => {
    if (!targetDir) return;

    setIsScanning(true);
    setScanProgress(null);
    setDuplicateProgress(null);
    setDuplicateOutput(null);
    setCurrentPlan(null);
    setKeeperPolicy(policy);
    setKeeperOverrides(new Map());

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

  const handleGeneratePlan = async () => {
    if (!targetDir || !duplicateOutput) return;

    setIsGeneratingPlan(true);
    try {
      // Clone groups with custom keeper overrides if any
      const groups = duplicateOutput.groups.map((group) => {
        const override = keeperOverrides.get(group.blake3_hash);
        if (override) {
          const foundIdx = group.physical_copies.findIndex((c) => c.primary_path === override);
          if (foundIdx > 0) {
            const copies = [...group.physical_copies];
            const [selected] = copies.splice(foundIdx, 1);
            copies.unshift(selected);
            return { ...group, physical_copies: copies };
          }
        }
        return group;
      });

      const plan = await generateCleanupPlan(targetDir, undefined, keeperPolicy, groups);
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

  // Compute total actions count
  const totalProposedActions = duplicateOutput
    ? duplicateOutput.groups.reduce((acc, g) => acc + Math.max(0, g.physical_copies.length - 1), 0)
    : 0;

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
                  keeperOverrides={keeperOverrides}
                  onSetKeeper={handleSetKeeper}
                />
                <SafetyActionBar
                  totalActions={totalProposedActions}
                  reclaimableBytes={duplicateOutput.summary.reclaimable_bytes}
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
              setDuplicateOutput(null);
              setCurrentPlan(null);
              setExecReport(null);
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

