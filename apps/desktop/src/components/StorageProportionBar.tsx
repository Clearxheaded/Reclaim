import React, { useState } from 'react';
import {
  HardDrive,
  ShieldCheck,
  Trash2,
  Link2,
  Sparkles,
} from 'lucide-react';
import { DuplicateSummaryPayload } from '../types/reclaim';
import { formatBytes, formatNumber } from '../utils/format';

interface StorageProportionBarProps {
  summary: DuplicateSummaryPayload;
  totalScannedBytes?: number;
  totalScannedFiles?: number;
}

export const StorageProportionBar: React.FC<StorageProportionBarProps> = ({
  summary,
  totalScannedBytes,
  totalScannedFiles,
}) => {
  const [hoveredSegment, setHoveredSegment] = useState<
    'keepers' | 'reclaimable' | 'hardlinks' | 'unique' | null
  >(null);

  // Storage metric calculations
  const totalDupBytes = summary.total_duplicate_bytes;
  const reclaimableBytes = summary.reclaimable_bytes;
  const protectedKeeperBytes = Math.max(0, totalDupBytes - reclaimableBytes);
  const hardLinkSavingsBytes = summary.hard_link_savings_bytes;

  // Determine total pool to scale against
  const totalScanned = Math.max(totalScannedBytes ?? totalDupBytes, totalDupBytes);
  const uniqueBytes = Math.max(0, totalScanned - totalDupBytes);

  // Proportions (guaranteeing minimum width for visibility if > 0)
  const keeperPercent = totalScanned > 0 ? (protectedKeeperBytes / totalScanned) * 100 : 0;
  const reclaimablePercent = totalScanned > 0 ? (reclaimableBytes / totalScanned) * 100 : 0;
  const uniquePercent = totalScanned > 0 ? (uniqueBytes / totalScanned) * 100 : 0;

  // For hard links: since they physically occupy 0 B extra, DaisyDisk represents them
  // as an overlay / distinct hash badge or pinned indicator.
  const hasHardLinks = hardLinkSavingsBytes > 0;

  return (
    <div className="bg-surface/90 border-b border-border-subtle p-3.5 select-none space-y-3">
      {/* Top Header Row: Title & Proportional Legend */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Sparkles className="w-4 h-4 text-sky-400" />
          <h3 className="text-xs font-semibold uppercase tracking-wider text-gray-200">
            Storage Space Breakdown
          </h3>
          <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-elevated text-gray-400 border border-border-subtle">
            DaisyDisk Engine
          </span>
        </div>

        {/* Proportional Segment Legend */}
        <div className="flex items-center gap-4 text-[11px]">
          <div
            onMouseEnter={() => setHoveredSegment('keepers')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`flex items-center gap-1.5 cursor-pointer transition-opacity ${
              hoveredSegment && hoveredSegment !== 'keepers' ? 'opacity-40' : 'opacity-100'
            }`}
          >
            <span className="w-2.5 h-2.5 rounded-full bg-sky-400 shadow-[0_0_8px_rgba(56,189,248,0.4)]" />
            <span className="text-gray-300 font-medium">Keepers</span>
            <span className="font-mono text-sky-400 font-semibold">
              {formatBytes(protectedKeeperBytes)}
            </span>
          </div>

          <div
            onMouseEnter={() => setHoveredSegment('reclaimable')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`flex items-center gap-1.5 cursor-pointer transition-opacity ${
              hoveredSegment && hoveredSegment !== 'reclaimable' ? 'opacity-40' : 'opacity-100'
            }`}
          >
            <span className="w-2.5 h-2.5 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(16,185,129,0.4)]" />
            <span className="text-gray-300 font-medium">Reclaimable</span>
            <span className="font-mono text-emerald-400 font-semibold">
              {formatBytes(reclaimableBytes)}
            </span>
          </div>

          {hasHardLinks && (
            <div
              onMouseEnter={() => setHoveredSegment('hardlinks')}
              onMouseLeave={() => setHoveredSegment(null)}
              className={`flex items-center gap-1.5 cursor-pointer transition-opacity ${
                hoveredSegment && hoveredSegment !== 'hardlinks' ? 'opacity-40' : 'opacity-100'
              }`}
            >
              <span className="w-2.5 h-2.5 rounded-full bg-purple-400 shadow-[0_0_8px_rgba(168,85,247,0.4)]" />
              <span className="text-gray-300 font-medium">Hard-Link Aliases</span>
              <span className="font-mono text-purple-400 font-semibold">0 B</span>
            </div>
          )}

          {uniqueBytes > 0 && (
            <div
              onMouseEnter={() => setHoveredSegment('unique')}
              onMouseLeave={() => setHoveredSegment(null)}
              className={`flex items-center gap-1.5 cursor-pointer transition-opacity ${
                hoveredSegment && hoveredSegment !== 'unique' ? 'opacity-40' : 'opacity-100'
              }`}
            >
              <span className="w-2.5 h-2.5 rounded-full bg-slate-500" />
              <span className="text-gray-400">Unique Scanned</span>
              <span className="font-mono text-gray-400">{formatBytes(uniqueBytes)}</span>
            </div>
          )}
        </div>
      </div>

      {/* DaisyDisk-style Multi-Segment Proportional Storage Bar */}
      <div className="relative w-full h-3 rounded-full bg-elevated overflow-hidden flex border border-border-subtle p-0.5 shadow-inner">
        {/* Protected Keepers Segment */}
        {keeperPercent > 0 && (
          <div
            style={{ width: `${Math.max(keeperPercent, 1.5)}%` }}
            onMouseEnter={() => setHoveredSegment('keepers')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`h-full bg-gradient-to-r from-sky-500 to-sky-400 transition-all cursor-pointer relative group ${
              hoveredSegment === 'keepers'
                ? 'brightness-125 shadow-[0_0_12px_rgba(56,189,248,0.8)] z-10'
                : 'brightness-100'
            }`}
          />
        )}

        {/* Exact Reclaimable Segment */}
        {reclaimablePercent > 0 && (
          <div
            style={{ width: `${Math.max(reclaimablePercent, 1.5)}%` }}
            onMouseEnter={() => setHoveredSegment('reclaimable')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`h-full bg-gradient-to-r from-emerald-500 to-emerald-400 transition-all cursor-pointer relative group ${
              hoveredSegment === 'reclaimable'
                ? 'brightness-125 shadow-[0_0_12px_rgba(16,185,129,0.8)] z-10'
                : 'brightness-100'
            }`}
          />
        )}

        {/* Hard-Link Inode Deduplication Indicator */}
        {hasHardLinks && (
          <div
            style={{ width: '2.5%' }}
            onMouseEnter={() => setHoveredSegment('hardlinks')}
            onMouseLeave={() => setHoveredSegment(null)}
            title={`Physical Hard Links: ${formatBytes(hardLinkSavingsBytes)} shared content consumes 0 B physical storage`}
            className={`h-full bg-[repeating-linear-gradient(45deg,#a855f7,#a855f7_2px,#7e22ce_2px,#7e22ce_4px)] transition-all cursor-pointer relative ${
              hoveredSegment === 'hardlinks'
                ? 'brightness-125 shadow-[0_0_12px_rgba(168,85,247,0.8)] z-10'
                : 'brightness-100'
            }`}
          />
        )}

        {/* Unique Scanned Files Segment */}
        {uniquePercent > 0 && (
          <div
            style={{ width: `${uniquePercent}%` }}
            onMouseEnter={() => setHoveredSegment('unique')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`h-full bg-slate-700/60 transition-all cursor-pointer ${
              hoveredSegment === 'unique' ? 'bg-slate-600 z-10' : ''
            }`}
          />
        )}
      </div>

      {/* 4 Diagnostic Stat Cards */}
      <div className="grid grid-cols-4 gap-2 pt-0.5">
        {/* Card 1: Total Scanned */}
        <div className="p-2 rounded-lg bg-elevated/80 border border-border-subtle flex items-center justify-between">
          <div className="min-w-0">
            <span className="text-[10px] font-medium uppercase tracking-wider text-gray-400 flex items-center gap-1">
              <HardDrive className="w-3 h-3 text-gray-400" /> Total Scanned
            </span>
            <div className="text-xs font-mono font-bold text-gray-200 mt-0.5 truncate">
              {formatBytes(totalScanned)}
            </div>
            <div className="text-[10px] font-mono text-gray-400 mt-0.5">
              {formatNumber(totalScannedFiles ?? summary.total_files_scanned)} files
            </div>
          </div>
        </div>

        {/* Card 2: Protected Keepers */}
        <div
          className={`p-2 rounded-lg border transition-all ${
            hoveredSegment === 'keepers'
              ? 'bg-sky-500/15 border-sky-500/50'
              : 'bg-sky-500/5 border-sky-500/20'
          } flex items-center justify-between`}
        >
          <div className="min-w-0">
            <span className="text-[10px] font-medium uppercase tracking-wider text-sky-400 flex items-center gap-1">
              <ShieldCheck className="w-3 h-3 text-sky-400" /> Protected Keepers
            </span>
            <div className="text-xs font-mono font-bold text-sky-300 mt-0.5 truncate">
              {formatBytes(protectedKeeperBytes)}
            </div>
            <div className="text-[10px] font-mono text-sky-400/80 mt-0.5">
              {summary.duplicate_groups_count} original files preserved
            </div>
          </div>
        </div>

        {/* Card 3: Exact Reclaimable */}
        <div
          className={`p-2 rounded-lg border transition-all ${
            hoveredSegment === 'reclaimable'
              ? 'bg-emerald-500/15 border-emerald-500/50'
              : 'bg-emerald-500/5 border-emerald-500/20'
          } flex items-center justify-between`}
        >
          <div className="min-w-0">
            <span className="text-[10px] font-medium uppercase tracking-wider text-emerald-400 flex items-center gap-1">
              <Trash2 className="w-3 h-3 text-emerald-400" /> Exact Reclaimable
            </span>
            <div className="text-xs font-mono font-bold text-emerald-400 mt-0.5 truncate">
              {formatBytes(reclaimableBytes)}
            </div>
            <div className="text-[10px] font-mono text-emerald-400/80 mt-0.5">
              {Math.max(0, summary.duplicate_files_count - summary.duplicate_groups_count)} redundant
              copies
            </div>
          </div>
        </div>

        {/* Card 4: Hard-Link Aliases */}
        <div
          className={`p-2 rounded-lg border transition-all ${
            hoveredSegment === 'hardlinks'
              ? 'bg-purple-500/15 border-purple-500/50'
              : 'bg-purple-500/5 border-purple-500/20'
          } flex items-center justify-between`}
        >
          <div className="min-w-0">
            <span className="text-[10px] font-medium uppercase tracking-wider text-purple-400 flex items-center gap-1">
              <Link2 className="w-3 h-3 text-purple-400" /> Hard-Link Aliases
            </span>
            <div className="text-xs font-mono font-bold text-purple-300 mt-0.5 truncate">
              0 B <span className="text-[10px] font-normal text-purple-400/80">(Physical)</span>
            </div>
            <div className="text-[10px] font-mono text-purple-400/80 mt-0.5 truncate" title={`Shared content: ${formatBytes(hardLinkSavingsBytes)}`}>
              {hardLinkSavingsBytes > 0 ? `${formatBytes(hardLinkSavingsBytes)} shared content` : 'No hard links found'}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
