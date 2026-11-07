import React from 'react';
import {
  CheckCircle2,
  XCircle,
  Loader2,
  History,
} from 'lucide-react';
import { ExecutionProgressEventPayload, ExecutionReportPayload } from '../types/reclaim';
import { formatBytes } from '../utils/format';

interface ExecutionMonitorProps {
  isExecuting: boolean;
  progress: ExecutionProgressEventPayload | null;
  report: ExecutionReportPayload | null;
  onOpenHistory: () => void;
  onNewScan: () => void;
}

export const ExecutionMonitor: React.FC<ExecutionMonitorProps> = ({
  isExecuting,
  progress,
  report,
  onOpenHistory,
  onNewScan,
}) => {
  const percent = report
    ? 100
    : progress && progress.total_count > 0
    ? Math.round((progress.completed_count / progress.total_count) * 100)
    : 0;

  return (
    <div className="flex-1 flex flex-col p-5 overflow-hidden bg-canvas select-none">
      {/* Top Banner */}
      <div className="bg-surface border border-white/10 rounded-lg p-4 mb-3.5 shadow-sm">
        <div className="flex items-center justify-between mb-3.5">
          <div className="flex items-center gap-3">
            <div
              className={`w-8 h-8 rounded flex items-center justify-center ${
                report
                  ? report.failed_actions === 0
                    ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20'
                    : 'bg-rose-500/10 text-rose-400 border border-rose-500/20'
                  : 'bg-amber-500/10 text-amber-400 border border-amber-500/20'
              }`}
            >
              {report ? (
                report.failed_actions === 0 ? (
                  <CheckCircle2 className="w-4 h-4" />
                ) : (
                  <XCircle className="w-4 h-4" />
                )
              ) : (
                <Loader2 className="w-4 h-4 animate-spin" />
              )}
            </div>
            <div>
              <h2 className="text-xs font-semibold text-white">
                {report
                  ? report.dry_run
                    ? 'Dry Run Simulation Complete'
                    : 'Quarantine Execution Complete'
                  : 'Executing Safe Quarantine Moves'}
              </h2>
              <p className="text-[11px] text-gray-400">
                {report
                  ? report.dry_run
                    ? 'All preconditions validated with zero filesystem mutations'
                    : `Atomic moves recorded in durable SQLite journal: ${report.session_id}`
                  : 'Validating TOCTOU preconditions & moving via atomic non-replace'}
              </p>
            </div>
          </div>

          {report && (
            <div className="flex gap-2">
              <button
                onClick={onOpenHistory}
                className="px-3 py-1 rounded bg-elevated hover:bg-subtle text-gray-200 text-xs font-medium border border-white/10 flex items-center gap-1.5 transition-colors"
              >
                <History className="w-3.5 h-3.5 text-slate-400" />
                <span>History & Undo</span>
              </button>
              <button
                onClick={onNewScan}
                className="px-3 py-1 rounded bg-sky-500 hover:bg-sky-400 text-white text-xs font-medium transition-colors"
              >
                New Scan
              </button>
            </div>
          )}
        </div>

        {/* Progress Bar */}
        <div className="space-y-1">
          <div className="flex justify-between text-xs tabular-nums font-mono text-gray-400">
            <span>
              {report
                ? `${report.completed_actions} of ${report.total_actions} actions processed`
                : progress
                ? `${progress.completed_count} of ${progress.total_count} files`
                : 'Preparing executor...'}
            </span>
            <span className="text-gray-200 font-semibold">{percent}%</span>
          </div>
          <div className="w-full h-2 rounded bg-elevated border border-white/5 overflow-hidden">
            <div
              className={`h-full transition-all duration-200 ${
                report ? 'bg-emerald-500' : 'bg-sky-500'
              }`}
              style={{ width: `${percent}%` }}
            />
          </div>
        </div>

        {/* Active File In Flight */}
        {isExecuting && progress && (
          <div className="mt-3 p-2 rounded bg-canvas border border-white/5 text-xs font-mono text-gray-300 truncate flex items-center gap-2">
            <span className="px-1.5 py-0.2 rounded text-[10px] tabular-nums font-mono font-semibold bg-amber-500/10 text-amber-300 border border-amber-500/20">
              {progress.state.toUpperCase()}
            </span>
            <span className="truncate">{progress.source_path}</span>
          </div>
        )}
      </div>

      {/* Finished Report Summary Card */}
      {report && (
        <div className="grid grid-cols-4 gap-2.5 mb-3.5">
          <div className="p-2.5 rounded bg-surface border border-white/5">
            <span className="text-[10px] uppercase font-semibold text-gray-400">
              Completed Moves
            </span>
            <div className="text-base tabular-nums font-mono font-bold text-emerald-400 mt-0.5">
              {report.completed_actions}
            </div>
          </div>

          <div className="p-2.5 rounded bg-surface border border-white/5">
            <span className="text-[10px] uppercase font-semibold text-gray-400">
              Failed Actions
            </span>
            <div
              className={`text-base tabular-nums font-mono font-bold mt-0.5 ${
                report.failed_actions > 0 ? 'text-rose-400' : 'text-gray-400'
              }`}
            >
              {report.failed_actions}
            </div>
          </div>

          <div className="p-2.5 rounded bg-surface border border-white/5">
            <span className="text-[10px] uppercase font-semibold text-gray-400">
              Storage Quarantined
            </span>
            <div className="text-base tabular-nums font-mono font-bold text-gray-200 mt-0.5">
              {formatBytes(report.bytes_quarantined)}
            </div>
          </div>

          <div className="p-2.5 rounded bg-surface border border-white/5">
            <span className="text-[10px] uppercase font-semibold text-gray-400">
              Duration
            </span>
            <div className="text-base tabular-nums font-mono font-bold text-gray-200 mt-0.5">
              {report.duration_ms} ms
            </div>
          </div>
        </div>
      )}

      {/* Detailed Action Results Table */}
      <div className="flex-1 bg-surface border border-border-subtle rounded-lg flex flex-col overflow-hidden">
        <div className="p-2.5 border-b border-border-subtle bg-elevated/40 flex items-center justify-between text-xs font-semibold text-gray-300">
          <span>Execution Journal Stream</span>
          <span className="text-[11px] tabular-nums font-mono text-gray-400">
            {report ? `${report.results.length} total entries` : 'In progress...'}
          </span>
        </div>

        <div className="flex-1 overflow-y-auto divide-y divide-white/5 font-mono text-xs">
          {report ? (
            report.results.map((res) => (
              <div key={res.action_id} className="p-2.5 flex items-center justify-between hover:bg-elevated/40 transition-colors">
                <div className="min-w-0 pr-4">
                  <div className="text-gray-200 truncate">{res.source_path}</div>
                  <div className="text-[10px] text-gray-400 truncate mt-0.5">
                    → {res.quarantine_path}
                  </div>
                  {res.error && (
                    <div className="text-[10px] text-rose-400 mt-1">Error: {res.error}</div>
                  )}
                </div>
                <div className="shrink-0">
                  <span
                    className={`px-1.5 py-0.5 rounded text-[10px] font-semibold tabular-nums ${
                      res.status === 'Completed'
                        ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/30'
                        : 'bg-rose-500/10 text-rose-400 border border-rose-500/30'
                    }`}
                  >
                    {res.status.toUpperCase()}
                  </span>
                </div>
              </div>
            ))
          ) : (
            <div className="p-8 text-center text-gray-400 text-xs">
              Actions will appear here as they are validated and moved...
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
