import React, { useEffect } from 'react';
import { ArrowRight, ShieldCheck, HelpCircle } from 'lucide-react';
import { KeeperPolicy } from '../types/reclaim';
import { formatBytes } from '../utils/format';

interface SafetyActionBarProps {
  totalActions: number;
  reclaimableBytes: number;
  hardLinkSavings: number;
  policy: KeeperPolicy;
  onPolicyChange: (policy: KeeperPolicy) => void;
  onGeneratePlan: () => void;
  isGenerating: boolean;
}

export const SafetyActionBar: React.FC<SafetyActionBarProps> = ({
  totalActions,
  reclaimableBytes,
  hardLinkSavings,
  policy,
  onPolicyChange,
  onGeneratePlan,
  isGenerating,
}) => {
  const currentPolicyType = 'type' in policy ? policy.type : 'ShortestPath';

  // Keyboard shortcut listener: ⌘P to Preview Cleanup Plan
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === 'p' || e.key === 'P')) {
        if (totalActions > 0 && !isGenerating) {
          e.preventDefault();
          onGeneratePlan();
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [totalActions, isGenerating, onGeneratePlan]);

  return (
    <footer className="h-13 bg-surface border-t border-border-subtle px-4 flex items-center justify-between select-none">
      {/* Left: Summary Metrics */}
      <div className="flex items-center gap-5">
        <div>
          <div className="text-[10px] uppercase font-semibold tracking-wider text-gray-500">
            Selected for Quarantine
          </div>
          <div className="text-xs tabular-nums font-mono font-semibold text-gray-200">
            {totalActions} files
          </div>
        </div>

        <div className="h-5 w-px bg-white/10" />

        <div>
          <div className="text-[10px] uppercase font-semibold tracking-wider text-gray-500 flex items-center gap-1">
            <span>Reclaimable Storage</span>
            {hardLinkSavings > 0 && (
              <span
                title={`${formatBytes(hardLinkSavings)} in hard-link aliases preserved at 0 B`}
                className="cursor-help"
              >
                <HelpCircle className="w-3 h-3 text-slate-400" />
              </span>
            )}
          </div>
          <div className="text-xs tabular-nums font-mono font-bold text-emerald-400">
            {formatBytes(reclaimableBytes)}
          </div>
        </div>
      </div>

      {/* Center: Clean Keyboard Shortcuts Hints */}
      <div className="hidden md:flex items-center gap-3 text-[10px] tabular-nums font-mono text-gray-500">
        <span className="flex items-center gap-1">
          <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">J</kbd>
          <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">K</kbd>
          <span className="text-gray-400">nav</span>
        </span>
        <span>•</span>
        <span className="flex items-center gap-1">
          <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">Space</kbd>
          <span className="text-gray-400">preview</span>
        </span>
        <span>•</span>
        <span className="flex items-center gap-1">
          <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">Enter</kbd>
          <span className="text-gray-400">promote keeper</span>
        </span>
      </div>

      {/* Right: Policy Selector & Minimal Elevated Action Button */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 text-xs text-gray-400">
          <span className="text-[11px]">Keeper Policy:</span>
          <select
            value={currentPolicyType}
            onChange={(e) => {
              const val = e.target.value as any;
              onPolicyChange({ type: val });
            }}
            className="bg-elevated border border-white/10 rounded px-2 py-1 text-xs text-gray-200 focus:outline-none focus:border-sky-500/50 cursor-pointer"
          >
            <option value="ShortestPath">Shortest Path (Canonical)</option>
            <option value="OldestModified">Oldest Modified (Original)</option>
            <option value="NewestModified">Newest Modified (Latest)</option>
          </select>
        </div>

        <button
          onClick={onGeneratePlan}
          disabled={totalActions === 0 || isGenerating}
          className="px-3.5 py-1.5 rounded bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-gray-600 disabled:cursor-not-allowed text-white text-xs font-semibold flex items-center gap-1.5 transition-colors shadow-sm"
        >
          <ShieldCheck className="w-3.5 h-3.5" />
          <span>{isGenerating ? 'Building Plan...' : 'Preview Cleanup Plan'}</span>
          <kbd className="px-1 py-0.2 rounded bg-sky-600/60 text-[10px] font-mono text-sky-100">⌘P</kbd>
          <ArrowRight className="w-3 h-3" />
        </button>
      </div>
    </footer>
  );
};
