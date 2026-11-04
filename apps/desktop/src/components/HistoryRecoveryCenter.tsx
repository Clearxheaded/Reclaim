import React, { useState } from 'react';
import {
  RotateCcw,
  History,
  RefreshCw,
  ShieldCheck,
} from 'lucide-react';
import {
  RecoveryReportPayload,
  SessionHistoryItem,
  UndoProgressEventPayload,
  UndoReportPayload,
} from '../types/reclaim';
import { formatBytes, formatDate } from '../utils/format';
import { recoverJournal, undoSession } from '../api/ipc';
import { ConflictModal } from './ConflictModal';

interface HistoryRecoveryCenterProps {
  history: SessionHistoryItem[];
  onRefreshHistory: () => void;
}

export const HistoryRecoveryCenter: React.FC<HistoryRecoveryCenterProps> = ({
  history,
  onRefreshHistory,
}) => {
  const [isUndoing, setIsUndoing] = useState<boolean>(false);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [undoProgress, setUndoProgress] = useState<UndoProgressEventPayload | null>(null);
  const [latestUndoReport, setLatestUndoReport] = useState<UndoReportPayload | null>(null);
  const [recoveryReport, setRecoveryReport] = useState<RecoveryReportPayload | null>(null);
  const [isRecovering, setIsRecovering] = useState<boolean>(false);

  const handleRunUndo = async (sessionId: string, dryRun: boolean) => {
    setIsUndoing(true);
    setActiveSessionId(sessionId);
    setUndoProgress(null);
    setLatestUndoReport(null);

    try {
      const report = await undoSession(
        sessionId,
        { dry_run: dryRun, halt_on_error: false },
        (prog) => setUndoProgress(prog)
      );
      setLatestUndoReport(report);
      onRefreshHistory();
    } catch (err) {
      alert(`Undo operation failed: ${err}`);
    } finally {
      setIsUndoing(false);
      setActiveSessionId(null);
    }
  };

  const handleRecoverJournal = async () => {
    setIsRecovering(true);
    try {
      const rep = await recoverJournal();
      setRecoveryReport(rep);
      onRefreshHistory();
    } catch (err) {
      alert(`Recovery check failed: ${err}`);
    } finally {
      setIsRecovering(false);
    }
  };

  const totalQuarantined = history.reduce((acc, h) => acc + h.quarantined_bytes, 0);
  const totalRestored = history.reduce((acc, h) => acc + h.restored_bytes, 0);

  return (
    <div className="flex-1 flex flex-col p-6 overflow-y-auto bg-canvas">
      {/* Top Header & Metrics */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-base font-semibold text-white flex items-center gap-2">
            <History className="w-5 h-5 text-purple-400" />
            <span>Cleanup History & Recovery Center</span>
          </h2>
          <p className="text-xs text-gray-400 mt-0.5">
            Durably tracked in SQLite write-ahead journal. Every session is conditionally reversible.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={handleRecoverJournal}
            disabled={isRecovering}
            className="px-3 py-1.5 rounded-lg bg-subtle hover:bg-elevated text-gray-300 hover:text-white text-xs font-medium border border-border-subtle flex items-center gap-1.5 transition-colors"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isRecovering ? 'animate-spin' : ''}`} />
            <span>Reconcile Interrupted Actions</span>
          </button>
        </div>
      </div>

      {/* Recovery Notice if Reconciled */}
      {recoveryReport && (
        <div className="mb-4 p-3 rounded-lg bg-sky-500/10 border border-sky-500/20 text-xs text-sky-300 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <ShieldCheck className="w-4 h-4 text-sky-400" />
            <span>
              Journal Reconciliation Complete: Checked startup state against physical disk.
              ({recoveryReport.reconciled_sessions_count} sessions audited, {recoveryReport.completed_at_quarantine_count} confirmed moved).
            </span>
          </div>
          <button
            onClick={() => setRecoveryReport(null)}
            className="text-[10px] text-gray-400 hover:text-white"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Storage Summary Cards */}
      <div className="grid grid-cols-3 gap-4 mb-6">
        <div className="p-4 rounded-xl bg-surface border border-border-subtle">
          <span className="text-[10px] uppercase font-semibold text-gray-500">
            Total Sessions Tracked
          </span>
          <div className="text-xl font-mono font-bold text-gray-100 mt-1">
            {history.length}
          </div>
        </div>

        <div className="p-4 rounded-xl bg-surface border border-border-subtle">
          <span className="text-[10px] uppercase font-semibold text-gray-500">
            Total Quarantined Storage
          </span>
          <div className="text-xl font-mono font-bold text-amber-400 mt-1">
            {formatBytes(totalQuarantined)}
          </div>
        </div>

        <div className="p-4 rounded-xl bg-surface border border-border-subtle">
          <span className="text-[10px] uppercase font-semibold text-gray-500">
            Total Restored Storage
          </span>
          <div className="text-xl font-mono font-bold text-emerald-400 mt-1">
            {formatBytes(totalRestored)}
          </div>
        </div>
      </div>

      {/* Session List */}
      <div className="space-y-3">
        <span className="text-xs font-semibold text-gray-300">
          Chronological Sessions ({history.length}):
        </span>

        {history.length === 0 ? (
          <div className="p-12 text-center text-xs text-gray-500 bg-surface rounded-xl border border-border-subtle">
            No cleanup sessions recorded in the journal yet.
          </div>
        ) : (
          history.map((sess) => {
            const isReverted = sess.status === 'Reverted';
            const isCurrentUndoing = isUndoing && activeSessionId === sess.session_id;

            return (
              <div
                key={sess.session_id}
                className="p-4 rounded-xl bg-surface border border-border-subtle hover:border-border-muted transition-colors space-y-3"
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2.5">
                    <span
                      className={`px-2 py-0.5 rounded text-[10px] font-semibold ${
                        isReverted
                          ? 'bg-purple-500/10 text-purple-400 border border-purple-500/30'
                          : sess.status === 'Completed'
                          ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/30'
                          : 'bg-amber-500/10 text-amber-400 border border-amber-500/30'
                      }`}
                    >
                      {sess.status.toUpperCase()}
                    </span>
                    <span className="text-xs font-mono text-gray-300">
                      Session: {sess.session_id}
                    </span>
                  </div>

                  <span className="text-[11px] text-gray-500">
                    {formatDate(sess.started_at)}
                  </span>
                </div>

                <div className="grid grid-cols-3 gap-3 text-xs bg-canvas/40 p-2.5 rounded-lg border border-border-subtle font-mono">
                  <div>
                    <span className="text-[10px] text-gray-500 block">Scope Directory:</span>
                    <span className="text-gray-300 truncate block" title={sess.root_path}>
                      {sess.root_path}
                    </span>
                  </div>
                  <div>
                    <span className="text-[10px] text-gray-500 block">Actions:</span>
                    <span className="text-gray-200">
                      {sess.completed_actions} completed
                      {sess.reverted_actions > 0 && ` (${sess.reverted_actions} restored)`}
                    </span>
                  </div>
                  <div>
                    <span className="text-[10px] text-gray-500 block">Quarantined Storage:</span>
                    <span className="text-amber-400">{formatBytes(sess.quarantined_bytes)}</span>
                  </div>
                </div>

                {/* Undo In-Flight Progress */}
                {isCurrentUndoing && undoProgress && (
                  <div className="p-2 rounded bg-canvas border border-purple-500/30 text-xs font-mono text-purple-300 truncate">
                    [{undoProgress.status}] {undoProgress.source_path} ({undoProgress.completed_count}/{undoProgress.total_count})
                  </div>
                )}

                {/* Undo Controls */}
                <div className="flex items-center justify-between pt-1">
                  <div className="text-[11px] text-gray-500 flex items-center gap-1.5">
                    <ShieldCheck className="w-3.5 h-3.5 text-emerald-400" />
                    <span>Preconditions strictly validated before restoration.</span>
                  </div>

                  <div className="flex gap-2">
                    <button
                      onClick={() => handleRunUndo(sess.session_id, true)}
                      disabled={isReverted || isUndoing}
                      className="px-3 py-1.5 rounded bg-subtle hover:bg-elevated disabled:opacity-40 disabled:cursor-not-allowed text-xs text-gray-300 transition-colors"
                    >
                      Simulate Dry Run
                    </button>
                    <button
                      onClick={() => handleRunUndo(sess.session_id, false)}
                      disabled={isReverted || isUndoing}
                      className={`px-3 py-1.5 rounded text-xs font-semibold flex items-center gap-1.5 transition-colors ${
                        isReverted
                          ? 'bg-subtle text-gray-500 cursor-not-allowed'
                          : 'bg-purple-600 hover:bg-purple-500 text-white'
                      }`}
                    >
                      <RotateCcw className="w-3.5 h-3.5" />
                      <span>
                        {isCurrentUndoing
                          ? 'Restoring...'
                          : isReverted
                          ? 'Already Restored'
                          : 'Undo Session'}
                      </span>
                    </button>
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* Conflict Modal if Undo Refused Overwrite */}
      {latestUndoReport && latestUndoReport.conflicts.length > 0 && (
        <ConflictModal
          conflicts={latestUndoReport.conflicts}
          onClose={() => setLatestUndoReport(null)}
        />
      )}
    </div>
  );
};
