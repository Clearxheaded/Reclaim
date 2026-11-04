import React from 'react';
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
  return (
    <aside className="w-64 bg-surface border-r border-border-subtle flex flex-col h-full select-none">
      {/* Brand Header */}
      <div className="p-4 border-b border-border-subtle flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/30 flex items-center justify-center text-sky-400 font-mono font-bold text-sm">
            Rc
          </div>
          <div>
            <h1 className="text-sm font-semibold tracking-wide text-gray-100 flex items-center gap-1.5">
              Reclaim
              <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-subtle text-gray-400 font-normal">
                v0.1.0
              </span>
            </h1>
            <p className="text-[11px] text-gray-400">Safe Filesystem Cleanup</p>
          </div>
        </div>
      </div>

      {/* Target Directory Scope Card */}
      <div className="p-3 border-b border-border-subtle bg-canvas/40">
        <div className="flex items-center justify-between mb-1.5">
          <span className="text-[11px] font-medium text-gray-400 uppercase tracking-wider flex items-center gap-1">
            <Folder className="w-3 h-3 text-sky-400" /> Target Scope
          </span>
          <button
            onClick={onSelectDir}
            disabled={isScanning}
            className="text-[11px] text-sky-400 hover:text-sky-300 font-medium transition-colors disabled:opacity-50"
          >
            Change
          </button>
        </div>
        {targetDir ? (
          <div
            className="p-2 rounded bg-elevated/70 border border-border-subtle cursor-pointer hover:border-border-muted transition-colors group"
            onClick={onSelectDir}
            title={targetDir}
          >
            <div className="text-[12px] font-mono font-medium text-gray-200 truncate group-hover:text-sky-300">
              {targetDir.split('/').pop() || targetDir}
            </div>
            <div className="text-[10px] font-mono text-gray-500 truncate mt-0.5">
              {targetDir}
            </div>
          </div>
        ) : (
          <button
            onClick={onSelectDir}
            className="w-full py-2 px-3 rounded border border-dashed border-border-muted hover:border-sky-500/50 bg-subtle/50 text-[12px] text-gray-300 hover:text-white flex items-center justify-center gap-2 transition-colors"
          >
            <FolderSearch className="w-3.5 h-3.5 text-sky-400" />
            <span>Select Folder...</span>
          </button>
        )}
      </div>

      {/* Navigation Links */}
      <nav className="flex-1 p-2 space-y-1 overflow-y-auto">
        <button
          onClick={() => onTabChange('scan')}
          className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${
            activeTab === 'scan'
              ? 'bg-subtle text-white border border-border-muted shadow-sm'
              : 'text-gray-400 hover:text-gray-200 hover:bg-subtle/50'
          }`}
        >
          <FileSearch className="w-4 h-4 text-sky-400" />
          <span>Scan & Clean</span>
          {isScanning && (
            <span className="ml-auto w-2 h-2 rounded-full bg-amber-400 animate-pulse" />
          )}
        </button>

        <button
          onClick={() => onTabChange('plan')}
          disabled={!hasPlan}
          className={`w-full flex items-center justify-between px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${
            activeTab === 'plan'
              ? 'bg-subtle text-white border border-border-muted shadow-sm'
              : hasPlan
              ? 'text-gray-400 hover:text-gray-200 hover:bg-subtle/50'
              : 'text-gray-600 cursor-not-allowed'
          }`}
        >
          <div className="flex items-center gap-2.5">
            <FileCheck2 className={`w-4 h-4 ${hasPlan ? 'text-amber-400' : 'text-gray-600'}`} />
            <span>Plan & Review</span>
          </div>
          {hasPlan && (
            <span className="text-[10px] font-mono font-medium px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">
              Ready
            </span>
          )}
        </button>

        <button
          onClick={() => onTabChange('monitor')}
          className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${
            activeTab === 'monitor'
              ? 'bg-subtle text-white border border-border-muted shadow-sm'
              : 'text-gray-400 hover:text-gray-200 hover:bg-subtle/50'
          }`}
        >
          <Activity className="w-4 h-4 text-emerald-400" />
          <span>Execution Monitor</span>
        </button>

        <button
          onClick={() => onTabChange('history')}
          className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${
            activeTab === 'history'
              ? 'bg-subtle text-white border border-border-muted shadow-sm'
              : 'text-gray-400 hover:text-gray-200 hover:bg-subtle/50'
          }`}
        >
          <History className="w-4 h-4 text-purple-400" />
          <span>History & Undo</span>
        </button>

        <div className="pt-2">
          <div className="h-px bg-border-subtle my-2" />
        </div>

        <button
          onClick={() => onTabChange('invariants')}
          className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${
            activeTab === 'invariants'
              ? 'bg-subtle text-white border border-border-muted shadow-sm'
              : 'text-gray-400 hover:text-gray-200 hover:bg-subtle/50'
          }`}
        >
          <ShieldCheck className="w-4 h-4 text-emerald-400" />
          <span>Safety Invariants</span>
        </button>
      </nav>

      {/* Live Scan & Storage Summary Widget */}
      {summary && (
        <div className="p-3 border-t border-border-subtle bg-canvas/30 space-y-2">
          <div className="text-[11px] font-medium text-gray-400 flex items-center justify-between">
            <span className="flex items-center gap-1.5">
              <HardDrive className="w-3.5 h-3.5 text-emerald-400" />
              Reclaimable Space
            </span>
          </div>
          <div className="p-2 rounded bg-emerald-500/10 border border-emerald-500/20">
            <div className="text-base font-semibold font-mono text-emerald-400">
              {formatBytes(summary.reclaimable_bytes)}
            </div>
            <div className="text-[10px] text-gray-400 mt-0.5">
              {summary.duplicate_groups_count} duplicate groups ({summary.duplicate_files_count} files)
            </div>
          </div>
          {summary.hard_link_savings_bytes > 0 && (
            <div className="text-[10px] text-gray-400 bg-subtle/40 px-2 py-1 rounded border border-border-subtle">
              💡 {formatBytes(summary.hard_link_savings_bytes)} in hard-link aliases preserved
            </div>
          )}
        </div>
      )}
    </aside>
  );
};

