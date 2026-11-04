import React, { useState } from 'react';
import {
  ShieldCheck,
  FolderOpen,
  ChevronRight,
  ChevronDown,
  Layers,
  Copy,
  Hash,
  Check,
  AlertCircle,
} from 'lucide-react';
import { DuplicateGroupPayload, PhysicalCopyPayload } from '../types/reclaim';
import { formatBytes, getBasename, truncateHash } from '../utils/format';
import { revealInFileManager } from '../api/ipc';

interface DuplicateInspectorProps {
  groups: DuplicateGroupPayload[];
  keeperOverrides: Map<string, string>; // group.blake3_hash -> keeper primary_path
  onSetKeeper: (groupHash: string, keeperPath: string) => void;
}

export const DuplicateInspector: React.FC<DuplicateInspectorProps> = ({
  groups,
  keeperOverrides,
  onSetKeeper,
}) => {
  const [selectedGroupIndex, setSelectedGroupIndex] = useState<number>(0);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<number>>(new Set([0, 1, 2]));
  const [copiedHash, setCopiedHash] = useState<boolean>(false);

  const selectedGroup = groups[selectedGroupIndex] || groups[0];

  const getKeeperForGroup = (group: DuplicateGroupPayload): PhysicalCopyPayload => {
    const override = keeperOverrides.get(group.blake3_hash);
    if (override) {
      const found = group.physical_copies.find((c) => c.primary_path === override);
      if (found) return found;
    }
    return group.physical_copies[0];
  };

  const currentKeeper = selectedGroup ? getKeeperForGroup(selectedGroup) : null;
  const activeSelectedPath =
    selectedFilePath || (currentKeeper ? currentKeeper.primary_path : null);

  const toggleGroupExpand = (index: number, e: React.MouseEvent) => {
    e.stopPropagation();
    const next = new Set(expandedGroups);
    if (next.has(index)) {
      next.delete(index);
    } else {
      next.add(index);
    }
    setExpandedGroups(next);
  };

  const handleCopyHash = (hash: string) => {
    navigator.clipboard.writeText(hash);
    setCopiedHash(true);
    setTimeout(() => setCopiedHash(false), 2000);
  };

  if (!groups || groups.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-gray-400">
        <ShieldCheck className="w-12 h-12 text-emerald-400 mb-3" />
        <h3 className="text-base font-semibold text-white">No Duplicate Files Detected</h3>
        <p className="text-xs text-gray-500 max-w-sm mt-1">
          All scanned files have unique cryptographic BLAKE3 content hashes. No cleanup is required.
        </p>
      </div>
    );
  }

  return (
    <div className="flex-1 flex overflow-hidden">
      {/* Center List: Duplicate Groups */}
      <div className="flex-1 flex flex-col border-r border-border-subtle bg-canvas overflow-hidden">
        {/* List Header / Filter bar */}
        <div className="p-3 border-b border-border-subtle bg-surface flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Layers className="w-4 h-4 text-sky-400" />
            <span className="text-xs font-semibold text-gray-200">
              Duplicate Groups ({groups.length})
            </span>
          </div>
          <span className="text-[11px] text-gray-400">
            Click any row to promote as Protected Keeper
          </span>
        </div>

        {/* Groups Scroll List */}
        <div className="flex-1 overflow-y-auto divide-y divide-border-subtle/50">
          {groups.map((group, groupIdx) => {
            const isGroupSelected = groupIdx === selectedGroupIndex;
            const isExpanded = expandedGroups.has(groupIdx);
            const keeper = getKeeperForGroup(group);
            const groupFilename = getBasename(keeper.primary_path);

            return (
              <div
                key={group.blake3_hash}
                onClick={() => {
                  setSelectedGroupIndex(groupIdx);
                  setSelectedFilePath(keeper.primary_path);
                }}
                className={`transition-colors cursor-pointer ${
                  isGroupSelected ? 'bg-subtle/70' : 'hover:bg-subtle/30'
                }`}
              >
                {/* Group Row Header */}
                <div className="p-3 flex items-center justify-between">
                  <div className="flex items-center gap-2.5 min-w-0">
                    <button
                      onClick={(e) => toggleGroupExpand(groupIdx, e)}
                      className="p-1 rounded hover:bg-elevated text-gray-400 hover:text-white"
                    >
                      {isExpanded ? (
                        <ChevronDown className="w-3.5 h-3.5" />
                      ) : (
                        <ChevronRight className="w-3.5 h-3.5" />
                      )}
                    </button>
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-gray-100 truncate flex items-center gap-2">
                        <span>{groupFilename}</span>
                        <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-elevated text-gray-400 border border-border-subtle">
                          {formatBytes(group.size)}
                        </span>
                      </div>
                      <div className="text-[11px] text-gray-400 mt-0.5 flex items-center gap-2">
                        <span>{group.total_physical_copies} copies</span>
                        <span>•</span>
                        <span className="text-emerald-400 font-medium">
                          {formatBytes(group.reclaimable_bytes)} reclaimable
                        </span>
                        {group.total_path_count > group.total_physical_copies && (
                          <>
                            <span>•</span>
                            <span className="text-gray-500 font-mono text-[10px]">
                              +{group.total_path_count - group.total_physical_copies} hard-link aliases
                            </span>
                          </>
                        )}
                      </div>
                    </div>
                  </div>

                  <div className="text-right">
                    <span className="text-[10px] font-mono text-gray-500">
                      #{truncateHash(group.blake3_hash, 4)}
                    </span>
                  </div>
                </div>

                {/* Expanded Physical Copies & Aliases */}
                {isExpanded && (
                  <div className="pl-9 pr-3 pb-2.5 space-y-1.5">
                    {group.physical_copies.map((copy) => {
                      const isKeeper = copy.primary_path === keeper.primary_path;
                      const isFileSelected = copy.primary_path === activeSelectedPath;

                      return (
                        <div key={copy.primary_path} className="space-y-1">
                          <div
                            onClick={(e) => {
                              e.stopPropagation();
                              setSelectedGroupIndex(groupIdx);
                              setSelectedFilePath(copy.primary_path);
                            }}
                            className={`p-2 rounded-md text-xs flex items-center justify-between border transition-all ${
                              isFileSelected
                                ? 'bg-elevated border-sky-500/40'
                                : 'bg-surface/60 border-border-subtle hover:border-border-muted'
                            }`}
                          >
                            <div className="flex items-center gap-2 min-w-0">
                              {isKeeper ? (
                                <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-semibold bg-sky-500/10 text-sky-400 border border-sky-500/30">
                                  <ShieldCheck className="w-3 h-3" />
                                  KEEPER
                                </span>
                              ) : (
                                <button
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onSetKeeper(group.blake3_hash, copy.primary_path);
                                  }}
                                  title="Click to promote this copy to Protected Keeper"
                                  className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-semibold bg-amber-500/10 hover:bg-sky-500/20 text-amber-400 hover:text-sky-300 border border-amber-500/30 hover:border-sky-500/40 transition-colors"
                                >
                                  QUARANTINE
                                </button>
                              )}
                              <span className="font-mono text-[11px] text-gray-300 truncate">
                                {copy.primary_path}
                              </span>
                            </div>

                            <button
                              onClick={(e) => {
                                e.stopPropagation();
                                revealInFileManager(copy.primary_path);
                              }}
                              title="Reveal in Finder / Explorer"
                              className="p-1 rounded hover:bg-subtle text-gray-500 hover:text-gray-200 transition-colors"
                            >
                              <FolderOpen className="w-3.5 h-3.5" />
                            </button>
                          </div>

                          {/* Show hard links sharing this inode */}
                          {copy.all_paths.length > 1 && (
                            <div className="pl-6 space-y-1">
                              {copy.all_paths
                                .filter((p) => p !== copy.primary_path)
                                .map((aliasPath) => (
                                  <div
                                    key={aliasPath}
                                    className="p-1.5 rounded bg-subtle/30 border border-border-subtle/50 text-[11px] flex items-center justify-between text-gray-400"
                                  >
                                    <div className="flex items-center gap-1.5 truncate">
                                      <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-gray-800 text-gray-400 border border-gray-700">
                                        HARD-LINK ALIAS (0 B)
                                      </span>
                                      <span className="font-mono text-[10px] text-gray-400 truncate">
                                        {aliasPath}
                                      </span>
                                    </div>
                                    <button
                                      onClick={() => revealInFileManager(aliasPath)}
                                      className="p-0.5 hover:text-white"
                                    >
                                      <FolderOpen className="w-3 h-3 text-gray-500" />
                                    </button>
                                  </div>
                                ))}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Right Inspector: Split-View Comparison */}
      <div className="w-96 bg-surface flex flex-col overflow-y-auto">
        <div className="p-4 border-b border-border-subtle">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-gray-400">
            Duplicate Comparison Inspector
          </h3>
          <p className="text-[11px] text-gray-500 mt-0.5">
            Cryptographic content verification & metadata inspection
          </p>
        </div>

        {selectedGroup && currentKeeper ? (
          <div className="p-4 space-y-5">
            {/* Group Content Hash Card */}
            <div className="p-3 rounded-lg bg-elevated border border-border-subtle space-y-2">
              <div className="flex items-center justify-between text-xs font-medium text-gray-300">
                <span className="flex items-center gap-1.5">
                  <Hash className="w-3.5 h-3.5 text-sky-400" />
                  BLAKE3 Content Hash
                </span>
                <button
                  onClick={() => handleCopyHash(selectedGroup.blake3_hash)}
                  className="text-[10px] text-sky-400 hover:text-sky-300 flex items-center gap-1 transition-colors"
                >
                  {copiedHash ? (
                    <>
                      <Check className="w-3 h-3 text-emerald-400" /> Copied
                    </>
                  ) : (
                    <>
                      <Copy className="w-3 h-3" /> Copy
                    </>
                  )}
                </button>
              </div>
              <div className="p-2 rounded bg-canvas border border-border-subtle font-mono text-[11px] text-gray-300 break-all leading-relaxed">
                {selectedGroup.blake3_hash}
              </div>
              <div className="flex items-center gap-1 text-[10px] text-emerald-400 font-medium">
                <Check className="w-3 h-3" />
                <span>100% Exact Bit-for-Bit Duplicate Verified</span>
              </div>
            </div>

            {/* Side-by-Side: Protected Keeper Card */}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="text-[11px] font-semibold text-sky-400 uppercase tracking-wider flex items-center gap-1">
                  <ShieldCheck className="w-3.5 h-3.5" />
                  Designated Keeper File
                </span>
                <span className="text-[10px] text-gray-400">Preserved</span>
              </div>
              <div className="p-3 rounded-lg bg-sky-500/5 border border-sky-500/20 space-y-2">
                <div>
                  <div className="text-xs font-medium text-gray-200">
                    {getBasename(currentKeeper.primary_path)}
                  </div>
                  <div className="font-mono text-[11px] text-sky-300/80 break-all mt-0.5">
                    {currentKeeper.primary_path}
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-2 text-[11px] pt-1 border-t border-sky-500/10">
                  <div>
                    <span className="text-gray-500">Size:</span>
                    <div className="font-mono text-gray-200 font-medium">
                      {formatBytes(selectedGroup.size)}
                    </div>
                  </div>
                  <div>
                    <span className="text-gray-500">File Inode:</span>
                    <div className="font-mono text-gray-200">{currentKeeper.inode}</div>
                  </div>
                </div>

                <button
                  onClick={() => revealInFileManager(currentKeeper.primary_path)}
                  className="w-full py-1.5 px-2 rounded bg-sky-500/10 hover:bg-sky-500/20 text-sky-300 text-xs font-medium flex items-center justify-center gap-1.5 border border-sky-500/30 transition-colors"
                >
                  <FolderOpen className="w-3.5 h-3.5" />
                  <span>Reveal Keeper in File Manager</span>
                </button>
              </div>
            </div>

            {/* Currently Selected Duplicate Card */}
            {activeSelectedPath && activeSelectedPath !== currentKeeper.primary_path && (
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] font-semibold text-amber-400 uppercase tracking-wider flex items-center gap-1">
                    <AlertCircle className="w-3.5 h-3.5" />
                    Target for Quarantine
                  </span>
                  <span className="text-[10px] text-amber-400 font-medium">Proposed Move</span>
                </div>
                <div className="p-3 rounded-lg bg-amber-500/5 border border-amber-500/20 space-y-2">
                  <div>
                    <div className="text-xs font-medium text-gray-200">
                      {getBasename(activeSelectedPath)}
                    </div>
                    <div className="font-mono text-[11px] text-amber-300/80 break-all mt-0.5">
                      {activeSelectedPath}
                    </div>
                  </div>

                  <div className="p-2 rounded bg-canvas border border-border-subtle text-[10px] font-mono text-gray-400 space-y-1">
                    <div className="text-gray-400">Proposed Quarantine Destination:</div>
                    <div className="text-amber-400 truncate">
                      .../.reclaim/quarantine/{getBasename(activeSelectedPath)}
                    </div>
                  </div>

                  <div className="flex gap-2">
                    <button
                      onClick={() => onSetKeeper(selectedGroup.blake3_hash, activeSelectedPath)}
                      className="flex-1 py-1.5 px-2 rounded bg-subtle hover:bg-sky-500/10 text-gray-200 hover:text-sky-300 text-xs font-medium border border-border-subtle hover:border-sky-500/30 transition-colors"
                    >
                      Make This Keeper
                    </button>
                    <button
                      onClick={() => revealInFileManager(activeSelectedPath)}
                      className="p-1.5 rounded bg-subtle hover:bg-elevated text-gray-400 hover:text-white border border-border-subtle transition-colors"
                      title="Reveal in File Manager"
                    >
                      <FolderOpen className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="p-6 text-center text-xs text-gray-500">
            Select a duplicate group to inspect details
          </div>
        )}
      </div>
    </div>
  );
};
