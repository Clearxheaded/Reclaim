import React, { useState } from 'react';
import {
  FolderSync,
  Play,
  CheckCircle2,
  FileCheck,
} from 'lucide-react';
import { CleanupPlanPayload, ExecutionOptionsPayload } from '../types/reclaim';
import { formatBytes } from '../utils/format';

interface PlanReviewModalProps {
  plan: CleanupPlanPayload;
  onClose: () => void;
  onExecute: (options: ExecutionOptionsPayload) => void;
  isExecuting: boolean;
}

export const PlanReviewModal: React.FC<PlanReviewModalProps> = ({
  plan,
  onClose,
  onExecute,
  isExecuting,
}) => {
  const [dryRun, setDryRun] = useState<boolean>(false);
  const [haltOnError, setHaltOnError] = useState<boolean>(true);

  return (
    <div className="fixed inset-0 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 z-50 animate-in fade-in duration-150">
      <div className="bg-surface border border-border-muted rounded-xl max-w-2xl w-full shadow-2xl flex flex-col max-h-[85vh] overflow-hidden">
        {/* Header */}
        <div className="p-4 border-b border-border-subtle flex items-center justify-between bg-canvas/40">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-400">
              <FileCheck className="w-4 h-4" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-white">Review Cleanup Plan</h3>
              <p className="text-[11px] font-mono text-gray-400">
                Plan ID: {plan.plan_id}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            disabled={isExecuting}
            className="text-xs text-gray-400 hover:text-white px-2 py-1 rounded bg-subtle"
          >
            Esc
          </button>
        </div>

        {/* Content Body */}
        <div className="flex-1 p-5 space-y-4 overflow-y-auto">
          {/* Key Metrics Banner */}
          <div className="grid grid-cols-3 gap-3">
            <div className="p-3 rounded-lg bg-elevated border border-border-subtle">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Proposed Actions
              </span>
              <div className="text-base font-mono font-bold text-gray-200 mt-0.5">
                {plan.summary.total_actions} files
              </div>
            </div>

            <div className="p-3 rounded-lg bg-elevated border border-border-subtle">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Storage to Move
              </span>
              <div className="text-base font-mono font-bold text-amber-400 mt-0.5">
                {formatBytes(plan.summary.total_bytes_to_quarantine)}
              </div>
            </div>

            <div className="p-3 rounded-lg bg-elevated border border-border-subtle">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Protected Keepers
              </span>
              <div className="text-base font-mono font-bold text-sky-400 mt-0.5">
                {plan.summary.protected_keepers_count} preserved
              </div>
            </div>
          </div>

          {/* Staging Quarantine Directory */}
          <div className="p-3 rounded-lg bg-canvas border border-border-subtle space-y-1">
            <div className="text-xs font-medium text-gray-400 flex items-center gap-1.5">
              <FolderSync className="w-3.5 h-3.5 text-sky-400" />
              <span>Target Quarantine Staging Directory:</span>
            </div>
            <div className="font-mono text-xs text-sky-300 break-all bg-elevated/70 p-2 rounded border border-border-subtle">
              {plan.quarantine_dir}
            </div>
            <div className="text-[10px] text-gray-500">
              Preserved on the exact same filesystem volume to allow atomic no-replace moves.
            </div>
          </div>

          {/* Proposed Actions Preview Table */}
          <div className="space-y-1.5">
            <div className="text-xs font-semibold text-gray-300">
              Preview of Actions ({plan.actions.length} items):
            </div>
            <div className="border border-border-subtle rounded-lg max-h-48 overflow-y-auto divide-y divide-border-subtle bg-canvas/30">
              {plan.actions.map((act) => (
                <div key={act.action_id} className="p-2.5 text-xs space-y-1">
                  <div className="flex items-center justify-between font-mono text-[11px]">
                    <span className="text-amber-300 truncate max-w-sm" title={act.source_path}>
                      {act.source_path}
                    </span>
                    <span className="text-gray-400">
                      {formatBytes(act.preconditions.expected_size)}
                    </span>
                  </div>
                  <div className="text-[10px] text-gray-500 flex items-center gap-1.5">
                    <span className="text-sky-400">Keeper:</span>
                    <span className="truncate max-w-md font-mono" title={act.keeper_path}>
                      {act.keeper_path}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Safety Invariants Checklist */}
          <div className="p-3 rounded-lg bg-emerald-500/5 border border-emerald-500/20 space-y-1.5 text-xs text-gray-300">
            <div className="font-semibold text-emerald-400 flex items-center gap-1.5">
              <CheckCircle2 className="w-4 h-4" />
              <span>Verified Filesystem Safety Invariants:</span>
            </div>
            <ul className="text-[11px] text-gray-400 space-y-1 list-disc pl-4">
              <li>Atomic non-replacing moves guarantee zero silent overwrites.</li>
              <li>TOCTOU defense: verifies surviving keeper file and BLAKE3 hash before moving.</li>
              <li>Every action is journaled in SQLite and can be undone from the History tab.</li>
            </ul>
          </div>

          {/* Execution Options */}
          <div className="p-3 rounded-lg bg-surface border border-border-subtle space-y-2">
            <div className="flex items-center justify-between">
              <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={dryRun}
                  onChange={(e) => setDryRun(e.target.checked)}
                  className="rounded border-border-muted bg-elevated text-sky-500 focus:ring-0"
                />
                <span className="font-medium">Dry Run Simulation (Zero mutations)</span>
              </label>
              <span className="text-[10px] text-gray-500">Test preconditions without moving</span>
            </div>

            <div className="flex items-center justify-between">
              <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={haltOnError}
                  onChange={(e) => setHaltOnError(e.target.checked)}
                  className="rounded border-border-muted bg-elevated text-sky-500 focus:ring-0"
                />
                <span className="font-medium">Halt immediately on first error</span>
              </label>
              <span className="text-[10px] text-gray-500">Fail closed on unexpected conflict</span>
            </div>
          </div>
        </div>

        {/* Footer Actions */}
        <div className="p-4 border-t border-border-subtle bg-canvas/60 flex items-center justify-between">
          <button
            onClick={onClose}
            disabled={isExecuting}
            className="px-4 py-2 rounded-lg bg-subtle hover:bg-elevated text-gray-300 hover:text-white text-xs font-medium transition-colors"
          >
            Cancel
          </button>

          <button
            onClick={() => onExecute({ dry_run: dryRun, halt_on_error: haltOnError })}
            disabled={isExecuting}
            className={`px-5 py-2 rounded-lg text-xs font-semibold flex items-center gap-2 text-white shadow-sm transition-all ${
              dryRun
                ? 'bg-sky-600 hover:bg-sky-500'
                : 'bg-amber-600 hover:bg-amber-500'
            }`}
          >
            <Play className="w-3.5 h-3.5 fill-current" />
            <span>
              {isExecuting
                ? 'Executing...'
                : dryRun
                ? 'Simulate Dry Run'
                : 'Execute Reversible Quarantine'}
            </span>
          </button>
        </div>
      </div>
    </div>
  );
};
