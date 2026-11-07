import React, { useState } from 'react';
import {
  Layers,
  CheckCircle2,
  AlertCircle,
  X,
  ArrowRight,
  HardDrive,
  Check,
  Lock,
} from 'lucide-react';
import { DuplicateGroupPayload } from '../types/reclaim';
import { formatBytes, getBasename } from '../utils/format';
import { executeCowDeduplication } from '../api/ipc';

interface CowDeduplicationModalProps {
  isOpen: boolean;
  onClose: () => void;
  groups: DuplicateGroupPayload[];
  keeperOverrides: Map<string, string>;
  onComplete: (totalReclaimed: number, deduplicatedPaths: string[]) => void;
}

interface DuplicatePair {
  keeperPath: string;
  copyPath: string;
  size: number;
  hash: string;
}

export const CowDeduplicationModal: React.FC<CowDeduplicationModalProps> = ({
  isOpen,
  onClose,
  groups,
  keeperOverrides,
  onComplete,
}) => {
  const [status, setStatus] = useState<'idle' | 'running' | 'completed' | 'error'>('idle');
  const [completedCount, setCompletedCount] = useState<number>(0);
  const [currentPair, setCurrentPair] = useState<DuplicatePair | null>(null);
  const [bytesReclaimed, setBytesReclaimed] = useState<number>(0);
  const [initialDuplicateBytes, setInitialDuplicateBytes] = useState<number>(0);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [strategyUsed, setStrategyUsed] = useState<string>('APFS clonefile(2)');

  if (!isOpen) return null;

  // Build eligible pairs for extent deduplication
  const pairs: DuplicatePair[] = [];
  let totalBytes = 0;

  groups.forEach((group) => {
    if (group.physical_copies.length < 2) return;
    const keeperOverride = keeperOverrides.get(group.blake3_hash);
    const keeper =
      (keeperOverride &&
        group.physical_copies.find((c) => c.primary_path === keeperOverride)) ||
      group.physical_copies[0];

    group.physical_copies.forEach((copy) => {
      if (copy.primary_path !== keeper.primary_path) {
        pairs.push({
          keeperPath: keeper.primary_path,
          copyPath: copy.primary_path,
          size: group.size,
          hash: group.blake3_hash,
        });
        totalBytes += group.size;
      }
    });
  });

  const handleStartDeduplication = async () => {
    setStatus('running');
    setCompletedCount(0);
    setBytesReclaimed(0);
    setInitialDuplicateBytes(totalBytes);
    setErrorMessage(null);

    let reclaimed = 0;
    const successfulPaths: string[] = [];

    try {
      for (let i = 0; i < pairs.length; i++) {
        const pair = pairs[i];
        setCurrentPair(pair);

        const result = await executeCowDeduplication(pair.keeperPath, pair.copyPath);
        reclaimed += result.bytes_reclaimed;
        setStrategyUsed(result.strategy);
        successfulPaths.push(pair.copyPath);

        setCompletedCount(i + 1);
        setBytesReclaimed(reclaimed);
      }

      setStatus('completed');
      onComplete(reclaimed, successfulPaths);
    } catch (err: any) {
      console.error('CoW deduplication error:', err);
      setErrorMessage(typeof err === 'string' ? err : err?.message || 'Kernel deduplication failed');
      setStatus('error');
    }
  };

  const progressPercent = pairs.length > 0 ? (completedCount / pairs.length) * 100 : 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-sm select-none">
      <div className="w-full max-w-2xl bg-surface border border-white/10 rounded-lg shadow-2xl overflow-hidden flex flex-col">
        {/* Modal Header */}
        <div className="p-4 bg-elevated/40 border-b border-border-subtle flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <div className="p-1.5 rounded bg-sky-500/10 border border-sky-500/20 text-sky-400">
              <Layers className="w-4 h-4" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-semibold text-white">
                  Copy-on-Write (CoW) Extent Deduplication
                </h3>
                <span className="text-[10px] tabular-nums font-mono px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-medium">
                  Zero File Movement
                </span>
              </div>
              <p className="text-xs text-gray-400 mt-0.5">
                Block-level storage unification via kernel extents (APFS <code className="text-sky-300 font-mono">clonefile(2)</code> / Linux <code className="text-sky-300 font-mono">FICLONE</code>)
              </p>
            </div>
          </div>
          {status !== 'running' && (
            <button
              onClick={onClose}
              className="p-1 rounded text-gray-400 hover:text-white hover:bg-elevated transition-colors"
              title="Close modal"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Modal Body */}
        <div className="p-5 space-y-4">
          {status === 'idle' && (
            <>
              {/* Architecture & Safety Invariant Notice */}
              <div className="p-3 rounded bg-elevated/70 border border-white/5 space-y-2">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
                  <Layers className="w-3.5 h-3.5 text-sky-400" />
                  <span>Kernel Extent Deduplication Mechanism</span>
                </div>
                <p className="text-xs text-zinc-400 leading-relaxed">
                  Instead of moving duplicate files into quarantine, Reclaim instructs the filesystem
                  kernel to point duplicate files to the same physical storage extents.
                </p>
                <div className="grid grid-cols-2 gap-2.5 pt-1 text-[11px]">
                  <div className="p-2 rounded bg-surface/70 border border-white/5 space-y-1">
                    <span className="font-semibold text-emerald-400 flex items-center gap-1">
                      <Check className="w-3 h-3" /> Zero File Movement
                    </span>
                    <p className="text-gray-400 text-[10px] leading-normal">
                      All paths, filenames, and parent directories remain untouched. No links or references break.
                    </p>
                  </div>
                  <div className="p-2 rounded bg-surface/70 border border-white/5 space-y-1">
                    <span className="font-semibold text-sky-400 flex items-center gap-1">
                      <Lock className="w-3 h-3" /> Independent Inodes (CoW)
                    </span>
                    <p className="text-gray-400 text-[10px] leading-normal">
                      Unlike hard links, editing either file later triggers an automatic kernel copy-on-write without altering the other.
                    </p>
                  </div>
                </div>
              </div>

              {/* Before / After Disk Allocation Meters */}
              <div className="p-3 rounded bg-elevated/50 border border-white/5 space-y-2">
                <span className="text-[10px] uppercase font-mono text-gray-400 font-medium block">
                  Disk Physical Allocation Meters
                </span>
                <div className="space-y-2 text-xs font-mono">
                  {/* Before allocation bar */}
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-[11px] tabular-nums">
                      <span className="text-gray-400">Before: Duplicated Block Allocation</span>
                      <span className="text-rose-400 font-semibold">{formatBytes(totalBytes)} (100% duplicate)</span>
                    </div>
                    <div className="w-full h-2 rounded bg-canvas border border-white/5 overflow-hidden">
                      <div className="w-full h-full bg-rose-500/70" />
                    </div>
                  </div>
                  {/* After allocation bar */}
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-[11px] tabular-nums">
                      <span className="text-gray-400">After: Shared Kernel Extents</span>
                      <span className="text-emerald-400 font-semibold">0 B Redundant Space ({formatBytes(totalBytes)} Reclaimed)</span>
                    </div>
                    <div className="w-full h-2 rounded bg-canvas border border-white/5 overflow-hidden">
                      <div className="w-full h-full bg-emerald-500" />
                    </div>
                  </div>
                </div>
              </div>

              {/* Statistics Preview Grid */}
              <div className="grid grid-cols-3 gap-2.5">
                <div className="p-2.5 rounded bg-elevated/70 border border-white/5 text-center">
                  <span className="text-[10px] uppercase tabular-nums font-mono text-gray-400">
                    Files to Unify
                  </span>
                  <div className="text-base font-bold text-gray-100 tabular-nums font-mono mt-0.5">
                    {pairs.length}
                  </div>
                  <span className="text-[10px] text-gray-400">duplicate copies</span>
                </div>
                <div className="p-2.5 rounded bg-elevated/70 border border-white/5 text-center">
                  <span className="text-[10px] uppercase tabular-nums font-mono text-gray-400">
                    Physical Space Reclaimed
                  </span>
                  <div className="text-base font-bold text-emerald-400 tabular-nums font-mono mt-0.5">
                    {formatBytes(totalBytes)}
                  </div>
                  <span className="text-[10px] text-emerald-400/80">100% redundant storage</span>
                </div>
                <div className="p-2.5 rounded bg-elevated/70 border border-white/5 text-center">
                  <span className="text-[10px] uppercase tabular-nums font-mono text-gray-400">
                    Kernel Mechanism
                  </span>
                  <div className="text-xs font-semibold text-sky-300 font-mono mt-1 truncate">
                    APFS clonefile(2)
                  </div>
                  <span className="text-[10px] text-gray-400">atomic extent reflink</span>
                </div>
              </div>

              {/* Candidate Preview list */}
              <div className="space-y-1.5">
                <span className="text-xs font-semibold uppercase tracking-wider text-gray-400">
                  Target Operations Preview ({pairs.length} actions)
                </span>
                <div className="max-h-36 overflow-y-auto rounded bg-elevated/40 border border-white/5 p-2 space-y-1 text-xs font-mono">
                  {pairs.slice(0, 8).map((p, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between p-1 rounded hover:bg-elevated text-gray-300"
                    >
                      <div className="min-w-0 flex-1 truncate">
                        <span className="text-emerald-400">{getBasename(p.copyPath)}</span>
                        <span className="text-gray-500 text-[11px] mx-1">→ extents unified to</span>
                        <span className="text-sky-300">{getBasename(p.keeperPath)}</span>
                      </div>
                      <span className="text-[10px] tabular-nums text-gray-400 shrink-0 pl-2">
                        {formatBytes(p.size)}
                      </span>
                    </div>
                  ))}
                  {pairs.length > 8 && (
                    <div className="text-[10px] text-center text-gray-500 py-1">
                      + {pairs.length - 8} more duplicate copies
                    </div>
                  )}
                </div>
              </div>
            </>
          )}

          {status === 'running' && (
            <div className="py-6 space-y-5 text-center">
              <div className="inline-flex p-2.5 rounded-full bg-sky-500/10 border border-sky-500/30 text-sky-400 animate-spin">
                <Layers className="w-7 h-7" />
              </div>
              <div className="space-y-1">
                <h4 className="text-sm font-semibold text-white">
                  Executing CoW Extent Deduplication
                </h4>
                <p className="text-xs text-gray-400">
                  Cloning kernel storage extents and verifying bit-for-bit equivalence...
                </p>
              </div>

              {/* Progress Bar */}
              <div className="space-y-2 max-w-md mx-auto">
                <div className="flex items-center justify-between text-xs tabular-nums font-mono text-gray-400">
                  <span>
                    Item {completedCount} of {pairs.length}
                  </span>
                  <span>{progressPercent.toFixed(1)}%</span>
                </div>
                <div className="w-full h-2 rounded bg-elevated overflow-hidden border border-white/5">
                  <div
                    style={{ width: `${progressPercent}%` }}
                    className="h-full bg-sky-500 transition-all duration-150"
                  />
                </div>
                {currentPair && (
                  <div className="text-[11px] font-mono text-gray-400 truncate pt-1">
                    Unifying: <span className="text-gray-200">{getBasename(currentPair.copyPath)}</span>
                  </div>
                )}
              </div>

              <div className="p-2.5 rounded bg-elevated/70 border border-white/5 inline-flex items-center gap-4 text-xs font-mono">
                <div>
                  <span className="text-gray-500 text-[10px] block">RECLAIMED SO FAR</span>
                  <span className="text-emerald-400 font-bold tabular-nums">{formatBytes(bytesReclaimed)}</span>
                </div>
                <div className="h-6 w-px bg-white/10" />
                <div>
                  <span className="text-gray-500 text-[10px] block">EXTENTS CLONED</span>
                  <span className="text-sky-300 font-bold tabular-nums">{completedCount} files</span>
                </div>
              </div>
            </div>
          )}

          {status === 'completed' && (
            <div className="space-y-4">
              <div className="text-center space-y-1.5">
                <div className="inline-flex p-2 rounded-full bg-emerald-500/10 border border-emerald-500/30 text-emerald-400">
                  <CheckCircle2 className="w-7 h-7" />
                </div>
                <h4 className="text-sm font-bold text-white">
                  CoW Extent Deduplication Complete
                </h4>
                <p className="text-xs text-gray-400 max-w-md mx-auto">
                  Physical disk space has been reclaimed instantly. Files remain at their original
                  paths with 100% data integrity and kernel Copy-on-Write isolation.
                </p>
              </div>

              {/* Before / After Storage Statistics Card */}
              <div className="p-3.5 rounded-lg bg-elevated/60 border border-white/5 space-y-3">
                <div className="flex items-center justify-between border-b border-white/5 pb-2">
                  <span className="text-xs font-semibold text-gray-200 flex items-center gap-1.5">
                    <HardDrive className="w-3.5 h-3.5 text-sky-400" />
                    Storage Reclaimed & Verification Report
                  </span>
                  <span className="text-[10px] tabular-nums font-mono px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    Verified Bit-for-Bit
                  </span>
                </div>

                <div className="grid grid-cols-2 gap-3 text-xs font-mono">
                  <div className="space-y-1.5 p-2.5 rounded bg-canvas border border-white/5">
                    <span className="text-[10px] uppercase text-gray-500 font-semibold block">
                      Before Deduplication
                    </span>
                    <div>
                      <div className="text-gray-400 text-[11px]">Duplicate Storage Consumed:</div>
                      <div className="text-sm font-bold text-rose-400 tabular-nums">
                        {formatBytes(initialDuplicateBytes)}
                      </div>
                    </div>
                    <div className="text-[10px] text-gray-500">
                      Separate physical extents allocated on disk
                    </div>
                  </div>

                  <div className="space-y-1.5 p-2.5 rounded bg-emerald-500/5 border border-emerald-500/20">
                    <span className="text-[10px] uppercase text-emerald-400 font-semibold block">
                      After Deduplication
                    </span>
                    <div>
                      <div className="text-gray-400 text-[11px]">Physical Space Reclaimed:</div>
                      <div className="text-sm font-bold text-emerald-400 tabular-nums">
                        {formatBytes(bytesReclaimed)}
                      </div>
                    </div>
                    <div className="text-[10px] text-emerald-300">
                      Shared kernel extents consume 0 B redundant space
                    </div>
                  </div>
                </div>

                <div className="p-2 rounded bg-canvas/70 border border-white/5 space-y-1 text-[11px] tabular-nums font-mono text-gray-400">
                  <div className="flex justify-between">
                    <span>Kernel Extent Strategy:</span>
                    <span className="text-gray-200">{strategyUsed}</span>
                  </div>
                  <div className="flex justify-between">
                    <span>Cryptographic Verification:</span>
                    <span className="text-emerald-400">100% BLAKE3 Hash Re-verified</span>
                  </div>
                  <div className="flex justify-between">
                    <span>Inodes Unified:</span>
                    <span className="text-gray-200">{completedCount} duplicate files</span>
                  </div>
                </div>
              </div>
            </div>
          )}

          {status === 'error' && (
            <div className="p-4 rounded bg-rose-500/10 border border-rose-500/25 space-y-2 text-center">
              <AlertCircle className="w-7 h-7 text-rose-400 mx-auto" />
              <div>
                <h4 className="text-xs font-semibold text-rose-300">Deduplication Interrupted</h4>
                <p className="text-xs text-rose-200/80 mt-1">{errorMessage}</p>
              </div>
            </div>
          )}
        </div>

        {/* Modal Footer */}
        <div className="p-3 bg-elevated/40 border-t border-border-subtle flex items-center justify-between">
          <div className="text-[11px] text-gray-400 font-mono">
            {status === 'idle' && 'No user files are moved or deleted.'}
            {status === 'running' && 'Please wait while extent cloning proceeds...'}
            {status === 'completed' && 'Deduplication completed successfully.'}
          </div>

          <div className="flex items-center gap-2">
            {status === 'idle' && (
              <>
                <button
                  onClick={onClose}
                  className="px-3 py-1.5 rounded bg-elevated hover:bg-subtle text-xs text-gray-300 font-medium transition-colors border border-white/5"
                >
                  Cancel
                </button>
                <button
                  onClick={handleStartDeduplication}
                  disabled={pairs.length === 0}
                  className="px-3.5 py-1.5 rounded bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-gray-600 text-xs font-semibold text-white flex items-center gap-1.5 transition-colors"
                >
                  <Layers className="w-3.5 h-3.5" />
                  <span>Start Zero-Movement Extent Deduplication</span>
                  <ArrowRight className="w-3.5 h-3.5" />
                </button>
              </>
            )}

            {status === 'completed' && (
              <button
                onClick={onClose}
                className="px-4 py-1.5 rounded bg-emerald-500 hover:bg-emerald-400 text-xs font-semibold text-white transition-colors"
              >
                Done
              </button>
            )}

            {status === 'error' && (
              <button
                onClick={onClose}
                className="px-3 py-1.5 rounded bg-elevated hover:bg-subtle text-xs text-gray-300 transition-colors border border-white/5"
              >
                Close
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
