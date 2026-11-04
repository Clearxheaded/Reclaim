import React from 'react';
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

  return (
    <footer className="h-14 bg-surface border-t border-border-subtle px-4 flex items-center justify-between select-none">
      {/* Left: Summary Metrics */}
      <div className="flex items-center gap-6">
        <div>
          <div className="text-[10px] uppercase font-semibold tracking-wider text-gray-500">
            Selected for Quarantine
          </div>
          <div className="text-sm font-mono font-semibold text-gray-200">
            {totalActions} files
          </div>
        </div>

        <div className="h-6 w-px bg-border-subtle" />

        <div>
          <div className="text-[10px] uppercase font-semibold tracking-wider text-gray-500 flex items-center gap-1">
            <span>Reclaimable Storage</span>
            {hardLinkSavings > 0 && (
              <span
                title={`${formatBytes(hardLinkSavings)} in hard-link aliases preserved (not counted)`}
                className="cursor-help"
              >
                <HelpCircle className="w-3 h-3 text-sky-400" />
              </span>
            )}
          </div>
          <div className="text-sm font-mono font-bold text-emerald-400">
            {formatBytes(reclaimableBytes)}
          </div>
        </div>
      </div>

      {/* Right: Policy selector & Primary Action */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 text-xs text-gray-400">
          <span>Keeper Policy:</span>
          <select
            value={currentPolicyType}
            onChange={(e) => {
              const val = e.target.value as any;
              onPolicyChange({ type: val });
            }}
            className="bg-elevated border border-border-subtle rounded px-2 py-1 text-xs text-gray-200 focus:outline-none focus:border-sky-500"
          >
            <option value="ShortestPath">Shortest Path (Canonical)</option>
            <option value="OldestModified">Oldest Modified (Original)</option>
            <option value="NewestModified">Newest Modified (Latest)</option>
          </select>
        </div>

        <button
          onClick={onGeneratePlan}
          disabled={totalActions === 0 || isGenerating}
          className="px-4 py-2 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-gray-600 disabled:cursor-not-allowed text-white text-xs font-semibold flex items-center gap-2 shadow-sm transition-all"
        >
          <ShieldCheck className="w-4 h-4" />
          <span>{isGenerating ? 'Building Plan...' : 'Preview Cleanup Plan'}</span>
          <ArrowRight className="w-3.5 h-3.5" />
        </button>
      </div>
    </footer>
  );
};

