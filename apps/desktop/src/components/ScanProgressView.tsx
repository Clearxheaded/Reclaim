import React from 'react';
import { Loader2, XCircle, Search, Hash, CheckCircle2 } from 'lucide-react';
import { DuplicateProgressEvent, ScanProgressEvent } from '../types/reclaim';
import { formatBytes } from '../utils/format';

interface ScanProgressViewProps {
  scanProgress: ScanProgressEvent | null;
  duplicateProgress: DuplicateProgressEvent | null;
  onCancel: () => void;
}

export const ScanProgressView: React.FC<ScanProgressViewProps> = ({
  scanProgress,
  duplicateProgress,
  onCancel,
}) => {
  const isHashingStage = !!duplicateProgress;

  return (
    <div className="flex-1 flex flex-col items-center justify-center p-8 overflow-y-auto bg-canvas select-none">
      <div className="max-w-lg w-full bg-surface border border-white/10 rounded-lg p-5 space-y-5 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Loader2 className="w-4 h-4 animate-spin" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-white">
                {isHashingStage ? 'Analyzing Duplicate Candidates' : 'Scanning Directory Tree'}
              </h3>
              <p className="text-[11px] text-gray-400">
                {isHashingStage
                  ? duplicateProgress?.stage === 'FullHashing'
                    ? 'Computing cryptographic BLAKE3 content hashes'
                    : 'Filtering candidates by size & 16KB prefix'
                  : 'Reading directory metadata non-recursively'}
              </p>
            </div>
          </div>

          <button
            onClick={onCancel}
            className="px-2.5 py-1 rounded bg-elevated hover:bg-rose-500/15 text-gray-400 hover:text-rose-300 text-xs font-medium border border-white/5 hover:border-rose-500/30 flex items-center gap-1.5 transition-colors"
          >
            <XCircle className="w-3.5 h-3.5" />
            <span>Cancel</span>
          </button>
        </div>

        {/* Multi-Stage Visual Stepper */}
        <div className="grid grid-cols-3 gap-2">
          {/* Stage 1: Filesystem Traversal */}
          <div
            className={`p-2.5 rounded border text-xs ${
              !isHashingStage
                ? 'bg-sky-500/10 border-sky-500/30 text-sky-300'
                : 'bg-elevated/40 border-white/5 text-gray-400'
            }`}
          >
            <div className="flex items-center gap-1.5 mb-1 font-medium">
              {isHashingStage ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
              ) : (
                <Search className="w-3.5 h-3.5 text-sky-400" />
              )}
              <span>1. Traversal</span>
            </div>
            <div className="text-[11px] tabular-nums font-mono text-gray-300 truncate">
              {scanProgress ? `${scanProgress.files_scanned} files` : 'Indexing...'}
            </div>
          </div>

          {/* Stage 2: Prefix Filter */}
          <div
            className={`p-2.5 rounded border text-xs ${
              isHashingStage && duplicateProgress.stage === 'PrefixHashing'
                ? 'bg-amber-500/10 border-amber-500/30 text-amber-300'
                : duplicateProgress?.stage === 'FullHashing'
                ? 'bg-elevated/40 border-white/5 text-gray-400'
                : 'bg-elevated/20 border-white/5 text-gray-600'
            }`}
          >
            <div className="flex items-center gap-1.5 mb-1 font-medium">
              {duplicateProgress?.stage === 'FullHashing' ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
              ) : (
                <Hash className="w-3.5 h-3.5 text-amber-400" />
              )}
              <span>2. Prefix Filter</span>
            </div>
            <div className="text-[11px] tabular-nums font-mono text-gray-300 truncate">
              {isHashingStage ? '16 KB Check' : 'Pending'}
            </div>
          </div>

          {/* Stage 3: Full BLAKE3 */}
          <div
            className={`p-2.5 rounded border text-xs ${
              duplicateProgress?.stage === 'FullHashing'
                ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-300'
                : 'bg-elevated/20 border-white/5 text-gray-600'
            }`}
          >
            <div className="flex items-center gap-1.5 mb-1 font-medium">
              <Hash className="w-3.5 h-3.5" />
              <span>3. Exact BLAKE3</span>
            </div>
            <div className="text-[11px] tabular-nums font-mono text-gray-300 truncate">
              {duplicateProgress?.stage === 'FullHashing' ? 'Verification' : 'Pending'}
            </div>
          </div>
        </div>

        {/* Live Metrics Grid */}
        <div className="bg-canvas border border-white/5 rounded-lg p-3 grid grid-cols-2 gap-3">
          <div>
            <span className="text-[10px] uppercase font-semibold tracking-wider text-gray-500">
              Files Traversed
            </span>
            <div className="text-base tabular-nums font-mono font-bold text-gray-200 mt-0.5">
              {scanProgress ? scanProgress.files_scanned.toLocaleString() : '0'}
            </div>
            <div className="text-[10px] tabular-nums font-mono text-gray-500 mt-0.5">
              {scanProgress?.directories_scanned || 0} folders, {scanProgress?.symlinks_skipped || 0} symlinks skipped
            </div>
          </div>

          <div>
            <span className="text-[10px] uppercase font-semibold tracking-wider text-gray-500">
              {isHashingStage ? 'Bytes Hashed' : 'Bytes Scanned'}
            </span>
            <div className="text-base tabular-nums font-mono font-bold text-gray-200 mt-0.5">
              {formatBytes(
                isHashingStage
                  ? duplicateProgress.bytes_processed
                  : scanProgress?.bytes_scanned || 0
              )}
            </div>
            <div className="text-[10px] tabular-nums font-mono text-gray-500 mt-0.5">
              {isHashingStage && duplicateProgress.total_bytes > 0
                ? `of ${formatBytes(duplicateProgress.total_bytes)} candidate data`
                : 'Streaming stream metadata'}
            </div>
          </div>
        </div>

        {/* Active Path Display */}
        <div className="space-y-1">
          <div className="flex items-center justify-between text-[11px] text-gray-400">
            <span>Currently Examining:</span>
          </div>
          <div className="p-2 rounded bg-elevated/80 border border-white/5 font-mono text-[11px] text-gray-300 truncate">
            {isHashingStage
              ? duplicateProgress.current_file
              : scanProgress?.current_path || 'Starting traversal...'}
          </div>
        </div>
      </div>
    </div>
  );
};
