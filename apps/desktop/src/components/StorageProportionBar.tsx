import React, { useState } from 'react';
import {
  HardDrive,
  ShieldCheck,
  Trash2,
  Link2,
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

  // Total pool calculation
  const totalScanned = Math.max(totalScannedBytes ?? totalDupBytes, totalDupBytes);
  const uniqueBytes = Math.max(0, totalScanned - totalDupBytes);

  // Proportions
  const keeperPercent = totalScanned > 0 ? (protectedKeeperBytes / totalScanned) * 100 : 0;
  const reclaimablePercent = totalScanned > 0 ? (reclaimableBytes / totalScanned) * 100 : 0;
  const uniquePercent = totalScanned > 0 ? (uniqueBytes / totalScanned) * 100 : 0;
  const hasHardLinks = hardLinkSavingsBytes > 0;

  return (
    <div className="bg-surface border-b border-border-subtle px-4 py-2.5 select-none space-y-2">
      {/* Top Header Row: Title & Proportional Legend */}
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500">
          Storage Allocation
        </span>

        {/* Proportional Segment Legend */}
        <div className="flex items-center gap-4 text-[11px]">
          <div
            onMouseEnter={() => setHoveredSegment('keepers')}
            onMouseLeave={() => setHoveredSegment(null)}
            className={`flex items-center gap-1.5 cursor-pointer transition-opacity ${
              hoveredSegment && hoveredSegment !== 'keepers' ? 'opacity-40' : 'opacity-100'
            }`}
          >
            <span className="w-1.5 h-1.5 rounded-full bg-sky-400" />
            <span className="text-zinc-400">Keepers</span>
            <span className="tabular-nums font-mono text-zinc-200 font-medium">
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
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
            <span className="text-zinc-400">Reclaimable</span>
            <span className="tabular-nums font-mono text-emerald-400 font-medium">
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
              <span className="w-2.5 h-1.5 rounded-sm bg-hardlink-dashed bg-slate-700 border border-slate-600/40" />
              <span className="text-zinc-400">Hard Links</span>
              <span className="tabular-nums font-mono text-zinc-400 font-medium">0 B</span>
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
              <span className="w-1.5 h-1.5 rounded-full bg-zinc-600" />
              <span className="text-zinc-500">Unique</span>
              <span className="tabular-nums font-mono text-zinc-500">{formatBytes(uniqueBytes)}</span>
            </div>
          )}
        </div>
      </div>

      {/* Sleek Breakdown Bar with Live Transitions */}
      <div className="space-y-1">
        <div className="relative w-full h-2 rounded-sm bg-elevated overflow-hidden flex border border-white/5 p-px">
          {/* Protected Keepers Segment */}
          {keeperPercent > 0 && (
            <div
              style={{ width: `${Math.max(keeperPercent, 1.2)}%` }}
              onMouseEnter={() => setHoveredSegment('keepers')}
              onMouseLeave={() => setHoveredSegment(null)}
              className={`h-full bg-sky-500 transition-[width,filter] duration-300 ease-out cursor-pointer relative border-r border-black/30 ${
                hoveredSegment === 'keepers' ? 'brightness-125 z-10' : 'brightness-100'
              }`}
              title={`Keepers: ${formatBytes(protectedKeeperBytes)} (${keeperPercent.toFixed(1)}%)`}
            />
          )}

          {/* Exact Reclaimable Segment */}
          {reclaimablePercent > 0 && (
            <div
              style={{ width: `${Math.max(reclaimablePercent, 1.2)}%` }}
              onMouseEnter={() => setHoveredSegment('reclaimable')}
              onMouseLeave={() => setHoveredSegment(null)}
              className={`h-full bg-emerald-500 transition-[width,filter] duration-300 ease-out cursor-pointer relative border-r border-black/30 ${
                hoveredSegment === 'reclaimable' ? 'brightness-125 z-10' : 'brightness-100'
              }`}
              title={`Reclaimable: ${formatBytes(reclaimableBytes)} (${reclaimablePercent.toFixed(1)}%)`}
            />
          )}

          {/* Hard-Link Inode Deduplication: 0 B Dashed Pattern */}
          {hasHardLinks && (
            <div
              style={{ width: '2%' }}
              onMouseEnter={() => setHoveredSegment('hardlinks')}
              onMouseLeave={() => setHoveredSegment(null)}
              title={`Physical Hard Links: ${formatBytes(hardLinkSavingsBytes)} shared content consumes 0 B physical storage`}
              className={`h-full bg-hardlink-dashed bg-slate-700/80 border-r border-black/30 transition-[width,filter] duration-300 ease-out cursor-pointer relative ${
                hoveredSegment === 'hardlinks' ? 'brightness-125 z-10' : 'brightness-100'
              }`}
            />
          )}

          {/* Unique Scanned Files Segment */}
          {uniquePercent > 0 && (
            <div
              style={{ width: `${uniquePercent}%` }}
              onMouseEnter={() => setHoveredSegment('unique')}
              onMouseLeave={() => setHoveredSegment(null)}
              className={`h-full bg-zinc-700/40 transition-[width,background-color] duration-300 ease-out cursor-pointer ${
                hoveredSegment === 'unique' ? 'bg-zinc-600/70 z-10' : ''
              }`}
              title={`Unique: ${formatBytes(uniqueBytes)} (${uniquePercent.toFixed(1)}%)`}
            />
          )}
        </div>

        {/* Micro-meters Scale Ticks */}
        <div className="flex items-center justify-between text-[8px] tabular-nums font-mono text-zinc-600 px-0.5">
          <span>0%</span>
          <span>25%</span>
          <span>50%</span>
          <span>75%</span>
          <span>100%</span>
        </div>
      </div>

      {/* Seamless Status Ticker Strip */}
      <div className="grid grid-cols-4 divide-x divide-white/5 bg-elevated/40 border border-white/5 rounded text-xs py-1.5 px-1">
        {/* Metric 1 */}
        <div className="px-3 min-w-0">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <HardDrive className="w-2.5 h-2.5 text-zinc-500" /> Scanned
          </div>
          <div className="text-xs tabular-nums font-mono font-medium text-zinc-200 mt-0.5 truncate">
            {formatBytes(totalScanned)}
            <span className="text-[10px] text-zinc-500 ml-1">({formatNumber(totalScannedFiles ?? summary.total_files_scanned)})</span>
          </div>
        </div>

        {/* Metric 2 */}
        <div className="px-3 min-w-0">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <ShieldCheck className="w-2.5 h-2.5 text-sky-400" /> Keepers
          </div>
          <div className="text-xs tabular-nums font-mono font-medium text-sky-300 mt-0.5 truncate">
            {formatBytes(protectedKeeperBytes)}
            <span className="text-[10px] text-zinc-500 ml-1">({formatNumber(summary.duplicate_groups_count)})</span>
          </div>
        </div>

        {/* Metric 3 */}
        <div className="px-3 min-w-0">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <Trash2 className="w-2.5 h-2.5 text-emerald-400" /> Reclaimable
          </div>
          <div className="text-xs tabular-nums font-mono font-medium text-emerald-400 mt-0.5 truncate">
            {formatBytes(reclaimableBytes)}
            <span className="text-[10px] text-zinc-500 ml-1">({formatNumber(Math.max(0, summary.duplicate_files_count - summary.duplicate_groups_count))})</span>
          </div>
        </div>

        {/* Metric 4 */}
        <div className="px-3 min-w-0">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <Link2 className="w-2.5 h-2.5 text-zinc-500" /> Hard Links
          </div>
          <div className="text-xs tabular-nums font-mono font-medium text-zinc-400 mt-0.5 truncate">
            0 B <span className="text-[10px] text-zinc-500">(physical)</span>
          </div>
        </div>
      </div>
    </div>
  );
};
