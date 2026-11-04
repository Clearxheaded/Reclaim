import React, { useState } from 'react';
import { FolderSearch, ShieldCheck, Settings2, ArrowRight } from 'lucide-react';
import { KeeperPolicy } from '../types/reclaim';

interface DirectoryPickerProps {
  targetDir: string | null;
  onSelectDir: () => void;
  onStartScan: (includeEmpty: boolean, policy: KeeperPolicy) => void;
  isScanning: boolean;
}

export const DirectoryPicker: React.FC<DirectoryPickerProps> = ({
  targetDir,
  onSelectDir,
  onStartScan,
  isScanning,
}) => {
  const [includeEmpty, setIncludeEmpty] = useState<boolean>(false);
  const [policyType, setPolicyType] = useState<'ShortestPath' | 'OldestModified' | 'NewestModified'>('ShortestPath');

  const getPolicy = (): KeeperPolicy => {
    switch (policyType) {
      case 'OldestModified':
        return { type: 'OldestModified' };
      case 'NewestModified':
        return { type: 'NewestModified' };
      default:
        return { type: 'ShortestPath' };
    }
  };

  return (
    <div className="flex-1 flex flex-col items-center justify-center p-8 overflow-y-auto">
      <div className="max-w-xl w-full space-y-6">
        {/* Welcome / Header */}
        <div className="text-center space-y-2">
          <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-sky-500/10 border border-sky-500/20 text-sky-400 text-xs font-medium">
            <ShieldCheck className="w-3.5 h-3.5" />
            <span>Read-Only & Exact Content Verification</span>
          </div>
          <h2 className="text-2xl font-semibold tracking-tight text-white">
            Find and review duplicate files safely
          </h2>
          <p className="text-sm text-gray-400">
            Select any folder to scan. Reclaim identifies exact duplicates using two-stage BLAKE3
            cryptographic hashing while strictly respecting hard links.
          </p>
        </div>

        {/* Directory Drop/Select Zone */}
        <div
          onClick={onSelectDir}
          className="border-2 border-dashed border-border-muted hover:border-sky-500/50 bg-surface/50 hover:bg-surface rounded-xl p-8 flex flex-col items-center justify-center gap-3 cursor-pointer transition-all group"
        >
          <div className="w-12 h-12 rounded-xl bg-subtle border border-border-subtle flex items-center justify-center text-gray-400 group-hover:text-sky-400 group-hover:scale-105 transition-all">
            <FolderSearch className="w-6 h-6" />
          </div>
          <div className="text-center">
            {targetDir ? (
              <div>
                <p className="text-sm font-medium text-gray-200">Selected target directory:</p>
                <p className="text-xs font-mono text-sky-400 bg-subtle/80 px-2.5 py-1 rounded-md mt-1 border border-border-subtle max-w-md truncate">
                  {targetDir}
                </p>
                <p className="text-xs text-gray-500 mt-2">Click to choose a different folder</p>
              </div>
            ) : (
              <div>
                <p className="text-sm font-medium text-gray-200 group-hover:text-white">
                  Click to select a directory to scan
                </p>
                <p className="text-xs text-gray-500 mt-1">
                  Works on local directories, external drives, and project workspaces
                </p>
              </div>
            )}
          </div>
        </div>

        {/* Scan Options & Keeper Policy */}
        <div className="bg-surface rounded-lg border border-border-subtle p-4 space-y-3.5">
          <div className="flex items-center gap-2 text-xs font-medium text-gray-300">
            <Settings2 className="w-3.5 h-3.5 text-gray-400" />
            <span>Scan Configuration</span>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {/* Policy Selector */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">
                Default Keeper Selection Policy
              </label>
              <select
                value={policyType}
                onChange={(e) => setPolicyType(e.target.value as any)}
                className="w-full bg-elevated border border-border-subtle rounded-md px-2.5 py-1.5 text-xs text-gray-200 focus:outline-none focus:border-sky-500 transition-colors"
              >
                <option value="ShortestPath">Shortest Path (Keep canonical)</option>
                <option value="OldestModified">Oldest Modified (Keep original)</option>
                <option value="NewestModified">Newest Modified (Keep latest)</option>
              </select>
              <p className="text-[10px] text-gray-500 mt-1">
                You can manually override keeper files for any duplicate group after scanning.
              </p>
            </div>

            {/* Empty Files Toggle */}
            <div className="flex items-start gap-2.5 pt-1">
              <input
                type="checkbox"
                id="includeEmpty"
                checked={includeEmpty}
                onChange={(e) => setIncludeEmpty(e.target.checked)}
                className="mt-0.5 rounded border-border-muted bg-elevated text-sky-500 focus:ring-0 focus:ring-offset-0"
              />
              <label htmlFor="includeEmpty" className="text-xs text-gray-300 cursor-pointer">
                <span className="font-medium text-gray-200">Include 0-byte (empty) files</span>
                <p className="text-[10px] text-gray-500 mt-0.5">
                  Treat empty files as duplicate candidates (0 bytes reclaimable).
                </p>
              </label>
            </div>
          </div>
        </div>

        {/* Primary Action Button */}
        <button
          onClick={() => onStartScan(includeEmpty, getPolicy())}
          disabled={!targetDir || isScanning}
          className="w-full py-2.5 px-4 rounded-lg bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-gray-600 disabled:cursor-not-allowed text-white text-sm font-semibold flex items-center justify-center gap-2 transition-all shadow-sm"
        >
          <span>Scan For Duplicates</span>
          <ArrowRight className="w-4 h-4" />
        </button>

        {/* Safety Invariant Assurance */}
        <div className="flex items-center justify-center gap-1.5 text-[11px] text-gray-500">
          <ShieldCheck className="w-3.5 h-3.5 text-emerald-400" />
          <span>Scanning is strictly read-only and never modifies or deletes any files.</span>
        </div>
      </div>
    </div>
  );
};

