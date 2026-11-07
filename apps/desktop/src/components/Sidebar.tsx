import React, { useEffect } from 'react';
import {
  FileSearch,
  FileCheck2,
  Activity,
  History,
  ShieldCheck,
  Folder,
  HardDrive,
  FolderSearch,
} from 'lucide-react';
import { ActiveTab, DuplicateSummaryPayload } from '../types/reclaim';
import { formatBytes } from '../utils/format';

interface SidebarProps {
  activeTab: ActiveTab;
  onTabChange: (tab: ActiveTab) => void;
  targetDir: string | null;
  onSelectDir: () => void;
  summary: DuplicateSummaryPayload | null;
  hasPlan: boolean;
  isScanning: boolean;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeTab,
  onTabChange,
  targetDir,
  onSelectDir,
  summary,
  hasPlan,
  isScanning,
}) => {
  // Global keyboard shortcuts for tab navigation (⌘1, ⌘2, ⌘3, ⌘4)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey) {
        if (e.key === '1') {
          e.preventDefault();
          onTabChange('scan');
        } else if (e.key === '2' && hasPlan) {
          e.preventDefault();
          onTabChange('plan');
        } else if (e.key === '3') {
          e.preventDefault();
          onTabChange('monitor');
        } else if (e.key === '4') {
          e.preventDefault();
          onTabChange('history');
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [hasPlan, onTabChange]);

  return (
    <aside className="w-56 bg-surface border-r border-border-subtle flex flex-col h-full select-none shrink-0">
      {/* Brand Header */}
      <div className="h-12 px-3 border-b border-border-subtle flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <img src="/logo.svg" alt="Reclaim" className="w-6 h-6 rounded" />
          <div className="min-w-0">
            <h1 className="text-xs font-semibold tracking-tight text-zinc-100 truncate">
              Reclaim
            </h1>
            <p className="text-[10px] text-zinc-500 truncate">Filesystem Deduplicator</p>
          </div>
        </div>
      </div>

      {/* Target Directory Scope Card */}
      <div className="p-2.5 border-b border-border-subtle bg-canvas/40">
        <div className="flex items-center justify-between mb-1">
          <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <Folder className="w-3 h-3 text-zinc-400" /> Target
          </span>
          <button
            onClick={onSelectDir}
            disabled={isScanning}
            className="text-[10px] text-zinc-400 hover:text-zinc-200 transition-colors disabled:opacity-50"
          >
            Change
          </button>
        </div>
        {targetDir ? (
          <div
            className="p-2 rounded bg-elevated/80 border border-white/5 cursor-pointer hover:border-white/10 transition-colors group"
            onClick={onSelectDir}
            title={targetDir}
          >
            <div className="text-xs font-mono font-medium text-zinc-200 truncate group-hover:text-white">
              {targetDir.split('/').pop() || targetDir}
            </div>
            <div className="text-[10px] font-mono text-zinc-500 truncate mt-0.5">
              {targetDir}
            </div>
          </div>
        ) : (
          <button
            onClick={onSelectDir}
            className="w-full py-1.5 px-2 rounded border border-dashed border-white/10 hover:border-white/20 bg-elevated/40 text-[11px] text-zinc-400 hover:text-zinc-200 flex items-center justify-center gap-1.5 transition-colors"
          >
            <FolderSearch className="w-3 h-3 text-zinc-400" />
            <span>Select Folder...</span>
          </button>
        )}
      </div>

      {/* Navigation Links */}
      <nav className="flex-1 p-2 space-y-0.5 overflow-y-auto">
        <div className="px-2 py-1 text-[9px] font-mono uppercase tracking-wider text-zinc-600">
          Views
        </div>

        <button
          onClick={() => onTabChange('scan')}
          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded text-xs transition-colors ${
            activeTab === 'scan'
              ? 'bg-elevated text-zinc-100 font-medium border border-white/10'
              : 'text-zinc-400 hover:text-zinc-200 hover:bg-elevated/40'
          }`}
        >
          <div className="flex items-center gap-2">
            <FileSearch className="w-3.5 h-3.5 text-zinc-400" />
            <span>Scan & Review</span>
          </div>
          <div className="flex items-center gap-1.5">
            {isScanning && (
              <span className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
            )}
            <kbd className="text-[9px] font-mono text-zinc-600">⌘1</kbd>
          </div>
        </button>

        <button
          onClick={() => onTabChange('plan')}
          disabled={!hasPlan}
          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded text-xs transition-colors ${
            activeTab === 'plan'
              ? 'bg-elevated text-zinc-100 font-medium border border-white/10'
              : hasPlan
              ? 'text-zinc-400 hover:text-zinc-200 hover:bg-elevated/40'
              : 'text-zinc-600 cursor-not-allowed opacity-50'
          }`}
        >
          <div className="flex items-center gap-2">
            <FileCheck2 className="w-3.5 h-3.5 text-zinc-400" />
            <span>Cleanup Plan</span>
          </div>
          <div className="flex items-center gap-1.5">
            {hasPlan && (
              <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-amber-500/10 text-amber-300 border border-amber-500/20">
                Staged
              </span>
            )}
            <kbd className="text-[9px] font-mono text-zinc-600">⌘2</kbd>
          </div>
        </button>

        <button
          onClick={() => onTabChange('monitor')}
          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded text-xs transition-colors ${
            activeTab === 'monitor'
              ? 'bg-elevated text-zinc-100 font-medium border border-white/10'
              : 'text-zinc-400 hover:text-zinc-200 hover:bg-elevated/40'
          }`}
        >
          <div className="flex items-center gap-2">
            <Activity className="w-3.5 h-3.5 text-zinc-400" />
            <span>Execution Monitor</span>
          </div>
          <kbd className="text-[9px] font-mono text-zinc-600">⌘3</kbd>
        </button>

        <button
          onClick={() => onTabChange('history')}
          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded text-xs transition-colors ${
            activeTab === 'history'
              ? 'bg-elevated text-zinc-100 font-medium border border-white/10'
              : 'text-zinc-400 hover:text-zinc-200 hover:bg-elevated/40'
          }`}
        >
          <div className="flex items-center gap-2">
            <History className="w-3.5 h-3.5 text-zinc-400" />
            <span>Journal & Undo</span>
          </div>
          <kbd className="text-[9px] font-mono text-zinc-600">⌘4</kbd>
        </button>

        <div className="pt-2">
          <div className="h-px bg-white/5 my-1" />
        </div>

        <button
          onClick={() => onTabChange('invariants')}
          className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded text-xs transition-colors ${
            activeTab === 'invariants'
              ? 'bg-elevated text-zinc-100 font-medium border border-white/10'
              : 'text-zinc-400 hover:text-zinc-200 hover:bg-elevated/40'
          }`}
        >
          <ShieldCheck className="w-3.5 h-3.5 text-zinc-400" />
          <span>Safety Invariants</span>
        </button>
      </nav>

      {/* Live Scan & Storage Summary Widget */}
      {summary && (
        <div className="p-3 border-t border-border-subtle bg-canvas/50 space-y-1.5">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 flex items-center gap-1">
            <HardDrive className="w-3 h-3 text-zinc-400" />
            Reclaimable Space
          </div>
          <div className="p-2 rounded bg-elevated/80 border border-white/5 space-y-0.5">
            <div className="text-sm font-semibold tabular-nums font-mono text-emerald-400">
              {formatBytes(summary.reclaimable_bytes)}
            </div>
            <div className="text-[10px] tabular-nums font-mono text-zinc-500">
              {summary.duplicate_groups_count} groups ({summary.duplicate_files_count} files)
            </div>
          </div>
          {summary.hard_link_savings_bytes > 0 && (
            <div className="text-[10px] tabular-nums font-mono text-zinc-400 px-1">
              Hard-link aliases: 0 B physical ({formatBytes(summary.hard_link_savings_bytes)} shared)
            </div>
          )}
        </div>
      )}
    </aside>
  );
};
