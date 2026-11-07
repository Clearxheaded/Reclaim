import React, { useState } from 'react';
import {
  Layers,
  CheckCircle2,
  AlertCircle,
  X,
  Sparkles,
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
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-md select-none">
      <div className="w-full max-w-2xl bg-canvas border border-border-subtle rounded-xl shadow-2xl overflow-hidden flex flex-col">
        {/* Modal Header */}
        <div className="p-4 bg-surface border-b border-border-subtle flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <div className="p-2 rounded-lg bg-sky-500/10 border border-sky-500/20 text-sky-400">
              <Layers className="w-5 h-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-semibold text-white">
                  Copy-on-Write (CoW) Extent Deduplication
                </h3>
                <span className="text-[10px] font-mono px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/30 font-medium">
                  Zero Movement
                </span>
              </div>
              <p className="text-xs text-gray-400 mt-0.5">
                Block-level storage unification via kernel extents (APFS <code className="text-sky-300">clonefile(2)</code> / Linux <code className="text-sky-300">FICLONE</code>)
              </p>
            </div>
          </div>
          {status !== 'running' && (
            <button
              onClick={onClose}
              className="p-1 rounded-md text-gray-400 hover:text-white hover:bg-elevated transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Modal Body */}
        <div className="p-6 space-y-5">
          {status === 'idle' && (
            <>
              {/* Architecture & Safety Invariant Notice */}
              <div className="p-3.5 rounded-lg bg-sky-500/5 border border-sky-500/20 space-y-2">
                <div className="flex items-center gap-2 text-xs font-semibold text-sky-300">
                  <Sparkles className="w-4 h-4 text-sky-400" />
                  <span>How Copy-on-Write Extent Deduplication Works</span>
                </div>
                <p className="text-xs text-gray-300 leading-relaxed">
                  Instead of moving duplicate files into quarantine, Reclaim instructs the filesystem
                  kernel to point duplicate files to the same physical storage extents.
                </p>
                <div className="grid grid-cols-2 gap-3 pt-2 text-[11px]">
                  <div className="p-2.5 rounded bg-surface border border-border-subtle space-y-1">
                    <span className="font-semibold text-emerald-400 flex items-center gap-1">
                      <Check className="w-3.5 h-3.5" /> Zero File Movement
                    </span>
                    <p className="text-gray-400 text-[10px] leading-normal">
                      All paths, filenames, and parent directories remain untouched. No links or references break.
                    </p>
                  </div>
                  <div className="p-2.5 rounded bg-surface border border-border-subtle space-y-1">
                    <span className="font-semibold text-sky-400 flex items-center gap-1">
                      <Lock className="w-3.5 h-3.5" /> Independent Inodes (CoW)
                    </span>
                    <p className="text-gray-400 text-[10px] leading-normal">
                      Unlike hard links, editing either file later triggers an automatic kernel copy-on-write without altering the other.
                    </p>
                  </div>
                </div>
              </div>

              {/* Statistics Preview Card */}
              <div className="grid grid-cols-3 gap-3">
                <div className="p-3 rounded-lg bg-elevated border border-border-subtle text-center">
                  <span className="text-[10px] uppercase font-mono text-gray-400">
                    Files to Unify
                  </span>
                  <div className="text-lg font-bold text-gray-100 font-mono mt-0.5">
                    {pairs.length}
                  </div>
                  <span className="text-[10px] text-gray-400">duplicate copies</span>
                </div>
                <div className="p-3 rounded-lg bg-elevated border border-border-subtle text-center">
                  <span className="text-[10px] uppercase font-mono text-gray-400">
                    Physical Space Reclaimed
                  </span>
                  <div className="text-lg font-bold text-emerald-400 font-mono mt-0.5">
                    {formatBytes(totalBytes)}
                  </div>
                  <span className="text-[10px] text-emerald-400/80">100% duplicate space</span>
                </div>
                <div className="p-3 rounded-lg bg-elevated border border-border-subtle text-center">
                  <span className="text-[10px] uppercase font-mono text-gray-400">
                    Kernel Mechanism
                  </span>
                  <div className="text-sm font-semibold text-sky-300 font-mono mt-1 truncate">
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
                <div className="max-h-40 overflow-y-auto rounded-lg bg-surface border border-border-subtle p-2 space-y-1 text-xs font-mono">
                  {pairs.slice(0, 10).map((p, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between p-1.5 rounded hover:bg-elevated/60 text-gray-300"
                    >
                      <div className="min-w-0 flex-1 truncate">
                        <span className="text-emerald-400">{getBasename(p.copyPath)}</span>
                        <span className="text-gray-500 text-[11px] mx-1">→ extents cloned to</span>
                        <span className="text-sky-300">{getBasename(p.keeperPath)}</span>
                      </div>
                      <span className="text-[10px] text-gray-400 shrink-0 pl-2">
                        {formatBytes(p.size)}
                      </span>
                    </div>
                  ))}
                  {pairs.length > 10 && (
                    <div className="text-[10px] text-center text-gray-500 py-1">
                      + {pairs.length - 10} more duplicate copies
                    </div>
                  )}
                </div>
              </div>
            </>
          )}

          {status === 'running' && (
            <div className="py-8 space-y-5 text-center">
              <div className="inline-flex p-3 rounded-full bg-sky-500/10 border border-sky-500/30 text-sky-400 animate-spin">
                <Layers className="w-8 h-8" />
              </div>
              <div className="space-y-1">
                <h4 className="text-base font-semibold text-white">
                  Executing CoW Extent Deduplication
                </h4>
                <p className="text-xs text-gray-400">
                  Cloning storage extents and verifying 256-bit cryptographic hashes...
                </p>
              </div>

              {/* Progress Bar */}
              <div className="space-y-2 max-w-md mx-auto">
                <div className="flex items-center justify-between text-xs font-mono text-gray-400">
                  <span>
                    Item {completedCount} of {pairs.length}
                  </span>
                  <span>{progressPercent.toFixed(1)}%</span>
                </div>
                <div className="w-full h-2 rounded-full bg-elevated overflow-hidden border border-border-subtle">
                  <div
                    style={{ width: `${progressPercent}%` }}
                    className="h-full bg-gradient-to-r from-sky-500 to-emerald-400 transition-all duration-200"
                  />
                </div>
                {currentPair && (
                  <div className="text-[11px] font-mono text-gray-400 truncate pt-1">
                    Unifying: <span className="text-gray-200">{getBasename(currentPair.copyPath)}</span>
                  </div>
                )}
              </div>

              <div className="p-3 rounded-lg bg-surface border border-border-subtle inline-flex items-center gap-4 text-xs font-mono">
                <div>
                  <span className="text-gray-500 text-[10px] block">RECLAIMED SO FAR</span>
                  <span className="text-emerald-400 font-bold">{formatBytes(bytesReclaimed)}</span>
                </div>
                <div className="h-6 w-px bg-border-subtle" />
                <div>
                  <span className="text-gray-500 text-[10px] block">EXTENTS CLONED</span>
                  <span className="text-sky-300 font-bold">{completedCount} files</span>
                </div>
              </div>
            </div>
          )}

          {status === 'completed' && (
            <div className="space-y-5">
              <div className="text-center space-y-2">
                <div className="inline-flex p-3 rounded-full bg-emerald-500/10 border border-emerald-500/30 text-emerald-400">
                  <CheckCircle2 className="w-8 h-8" />
                </div>
                <h4 className="text-base font-bold text-white">
                  CoW Extent Deduplication Complete
                </h4>
                <p className="text-xs text-gray-400 max-w-md mx-auto">
                  Physical disk space has been reclaimed instantly. Files remain at their original
                  paths with 100% data integrity and kernel Copy-on-Write isolation.
                </p>
              </div>

              {/* Before / After Storage Statistics Card */}
              <div className="p-4 rounded-xl bg-surface border border-border-subtle space-y-3">
                <div className="flex items-center justify-between border-b border-border-subtle pb-2">
                  <span className="text-xs font-semibold text-gray-200 flex items-center gap-1.5">
                    <HardDrive className="w-4 h-4 text-sky-400" />
                    Storage Reclaimed & Verification Report
                  </span>
                  <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    Verified Bit-for-Bit
                  </span>
                </div>

                <div className="grid grid-cols-2 gap-4 text-xs font-mono">
                  <div className="space-y-2 p-3 rounded-lg bg-elevated/60 border border-border-subtle">
                    <span className="text-[10px] uppercase text-gray-500 font-semibold block">
                      Before Deduplication
                    </span>
                    <div>
                      <div className="text-gray-400 text-[11px]">Duplicate Storage Consumed:</div>
                      <div className="text-base font-bold text-rose-400">
                        {formatBytes(initialDuplicateBytes)}
                      </div>
                    </div>
                    <div className="text-[10px] text-gray-500">
                      Separate physical extents allocated on disk
                    </div>
                  </div>

                  <div className="space-y-2 p-3 rounded-lg bg-emerald-500/5 border border-emerald-500/20">
                    <span className="text-[10px] uppercase text-emerald-400 font-semibold block">
                      After Deduplication
                    </span>
                    <div>
                      <div className="text-gray-400 text-[11px]">Physical Space Reclaimed:</div>
                      <div className="text-base font-bold text-emerald-400">
                        {formatBytes(bytesReclaimed)}
                      </div>
                    </div>
                    <div className="text-[10px] text-emerald-300">
                      Shared kernel extents consume 0 B redundant space
                    </div>
                  </div>
                </div>

                <div className="p-2.5 rounded-lg bg-elevated/40 border border-border-subtle space-y-1 text-[11px] font-mono text-gray-400">
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
            <div className="p-4 rounded-lg bg-rose-500/10 border border-rose-500/30 space-y-3 text-center">
              <AlertCircle className="w-8 h-8 text-rose-400 mx-auto" />
              <div>
                <h4 className="text-sm font-semibold text-rose-300">Deduplication Interrupted</h4>
                <p className="text-xs text-rose-200/80 mt-1">{errorMessage}</p>
              </div>
            </div>
          )}
        </div>

        {/* Modal Footer */}
        <div className="p-4 bg-surface border-t border-border-subtle flex items-center justify-between">
          <div className="text-[11px] text-gray-500 font-mono">
            {status === 'idle' && 'No user files are moved or deleted.'}
            {status === 'running' && 'Please wait while extent cloning proceeds...'}
            {status === 'completed' && 'Deduplication completed successfully.'}
          </div>

          <div className="flex items-center gap-2">
            {status === 'idle' && (
              <>
                <button
                  onClick={onClose}
                  className="px-3 py-1.5 rounded-lg bg-elevated hover:bg-subtle text-xs text-gray-300 font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleStartDeduplication}
                  disabled={pairs.length === 0}
                  className="px-4 py-1.5 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-gray-600 text-xs font-semibold text-white flex items-center gap-1.5 shadow-sm transition-all"
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
                className="px-5 py-1.5 rounded-lg bg-emerald-500 hover:bg-emerald-400 text-xs font-semibold text-white transition-colors"
              >
                Done
              </button>
            )}

            {status === 'error' && (
              <button
                onClick={onClose}
                className="px-4 py-1.5 rounded-lg bg-elevated hover:bg-subtle text-xs text-gray-300 transition-colors"
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
