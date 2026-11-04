import React, { useState, useEffect } from 'react';
import { ArrowRight, Folder } from 'lucide-react';
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

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === 'k' || e.key === 'o' || e.key === 'K' || e.key === 'O')) {
        e.preventDefault();
        onSelectDir();
      } else if (e.key === 'Enter' && targetDir && !isScanning) {
        if (document.activeElement?.tagName !== 'BUTTON' && document.activeElement?.tagName !== 'SELECT') {
          e.preventDefault();
          onStartScan(includeEmpty, getPolicy());
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [targetDir, isScanning, includeEmpty, policyType, onSelectDir, onStartScan]);

  return (
    <div className="flex-1 flex flex-col items-center justify-center p-8 overflow-y-auto bg-canvas select-none">
      <div className="max-w-md w-full space-y-4">
        {/* Header */}
        <div className="space-y-1">
          <h2 className="text-sm font-semibold tracking-tight text-zinc-100">
            Select Directory to Scan
          </h2>
          <p className="text-xs text-zinc-500">
            Scans files recursively and identifies exact duplicates using two-stage BLAKE3 content hashing.
          </p>
        </div>

        {/* Directory Selector Bar */}
        <div
          onClick={onSelectDir}
          className="border border-white/10 hover:border-white/20 bg-surface rounded-lg p-3.5 flex items-center justify-between cursor-pointer transition-colors group"
        >
          <div className="flex items-center gap-3 min-w-0 flex-1 pr-2">
            <div className="w-8 h-8 rounded bg-elevated border border-white/5 flex items-center justify-center text-zinc-400 group-hover:text-white shrink-0">
              <Folder className="w-4 h-4" />
            </div>
            <div className="min-w-0 flex-1">
              {targetDir ? (
                <>
                  <div className="text-xs font-mono font-medium text-zinc-200 truncate">
                    {targetDir.split(/[/\\]/).pop() || targetDir}
                  </div>
                  <div className="text-[10px] font-mono text-zinc-500 truncate mt-0.5">
                    {targetDir}
                  </div>
                </>
              ) : (
                <div className="text-xs text-zinc-400 group-hover:text-zinc-200">
                  Choose directory to scan...
                </div>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <kbd className="px-1.5 py-0.5 rounded bg-elevated border border-white/5 text-[9px] font-mono text-zinc-400">
              ⌘O
            </kbd>
            <button
              type="button"
              className="px-2.5 py-1 rounded bg-elevated hover:bg-subtle text-xs font-medium text-zinc-200 border border-white/5 transition-colors"
            >
              Browse
            </button>
          </div>
        </div>

        {/* Configuration Row */}
        <div className="bg-surface rounded-lg border border-white/5 p-3.5 space-y-3">
          <div className="flex items-center justify-between text-xs">
            <label className="text-zinc-400 font-medium">Default Keeper Policy</label>
            <select
              value={policyType}
              onChange={(e) => setPolicyType(e.target.value as any)}
              className="bg-elevated border border-white/10 rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-white/20 cursor-pointer"
            >
              <option value="ShortestPath">Shortest Path (Canonical)</option>
              <option value="OldestModified">Oldest Modified (Original)</option>
              <option value="NewestModified">Newest Modified (Latest)</option>
            </select>
          </div>

          <div className="h-px bg-white/5" />

          <div className="flex items-center justify-between text-xs">
            <div>
              <span className="text-zinc-300 block font-medium">Include Empty Files</span>
              <span className="text-[10px] text-zinc-500 block">Include 0-byte files in duplicate groups</span>
            </div>
            <input
              type="checkbox"
              id="includeEmpty"
              checked={includeEmpty}
              onChange={(e) => setIncludeEmpty(e.target.checked)}
              className="rounded border-white/10 bg-canvas text-sky-500 focus:ring-0 cursor-pointer"
            />
          </div>
        </div>

        {/* Action Button */}
        <button
          onClick={() => onStartScan(includeEmpty, getPolicy())}
          disabled={!targetDir || isScanning}
          className="w-full py-2 px-4 rounded bg-sky-500 hover:bg-sky-400 disabled:bg-subtle disabled:text-zinc-600 disabled:cursor-not-allowed text-white text-xs font-semibold flex items-center justify-center gap-2 transition-colors"
        >
          <span>Start Scan</span>
          <kbd className="px-1 py-0.2 rounded bg-sky-600/60 text-[9px] font-mono text-sky-100">Enter</kbd>
          <ArrowRight className="w-3.5 h-3.5" />
        </button>

        <p className="text-center text-[10px] font-mono text-zinc-600">
          Scanning is read-only. No files are modified or deleted during scan.
        </p>
      </div>
    </div>
  );
};
