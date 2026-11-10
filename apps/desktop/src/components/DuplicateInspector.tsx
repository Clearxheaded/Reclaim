import React, { useState, useEffect, useRef, useMemo } from 'react';
import {
  ShieldCheck,
  FolderOpen,
  ChevronRight,
  ChevronDown,
  Copy,
  Hash,
  Check,
  AlertTriangle,
  Search,
  ArrowUpDown,
  Filter,
  Eye,
  Layers,
  X,
  FileCode,
  FileText,
  FileImage,
  FileArchive,
  FileAudio,
  FileVideo,
  File,
} from 'lucide-react';
import {
  CandidateInvalidatedEvent,
  DuplicateGroupPayload,
  DuplicateSummaryPayload,
  InvalidationToastItem,
  PhysicalCopyPayload,
} from '../types/reclaim';
import {
  formatBytes,
  getBasename,
  getParentDir,
  getFileExtension,
  truncateHash,
} from '../utils/format';
import { checkCowSupport, executeCowDeduplication, revealInFileManager } from '../api/ipc';
import { StorageProportionBar } from './StorageProportionBar';
import { QuickLookModal } from './QuickLookModal';
import { CowDeduplicationModal } from './CowDeduplicationModal';

interface DuplicateInspectorProps {
  groups: DuplicateGroupPayload[];
  summary?: DuplicateSummaryPayload;
  totalScannedBytes?: number;
  totalScannedFiles?: number;
  targetDir?: string | null;
  keeperOverrides: Map<string, string>; // group.blake3_hash -> keeper primary_path
  invalidatedFiles: Map<string, CandidateInvalidatedEvent>;
  cowDeduplicatedPaths: Set<string>;
  invalidationToasts: InvalidationToastItem[];
  onDismissToast: (toastId: string) => void;
  onSetKeeper: (groupHash: string, keeperPath: string) => void;
  onCowDeduplicated: (dedupedPaths: string[], totalReclaimed: number) => void;
}

type SortOption = 'size_desc' | 'size_asc' | 'copies_desc' | 'name_asc';
type FilterOption = 'all' | 'reclaimable_only' | 'has_hardlinks';

interface FlatRowItem {
  id: string;
  groupHash: string;
  isGroupHeader: boolean;
  copy?: PhysicalCopyPayload;
  isKeeper: boolean;
  isHardLinkAlias?: boolean;
  aliasPath?: string;
}

export const DuplicateInspector: React.FC<DuplicateInspectorProps> = ({
  groups,
  summary,
  totalScannedBytes,
  totalScannedFiles,
  targetDir,
  keeperOverrides,
  invalidatedFiles,
  cowDeduplicatedPaths,
  invalidationToasts,
  onDismissToast,
  onSetKeeper,
  onCowDeduplicated,
}) => {
  // Search & Filter state
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [sortBy, setSortBy] = useState<SortOption>('size_desc');
  const [filterScope, setFilterScope] = useState<FilterOption>('all');

  // Copy-on-Write (CoW) Extent Deduplication State
  const [supportsCow, setSupportsCow] = useState<boolean>(false);
  const [cowModalOpen, setCowModalOpen] = useState<boolean>(false);
  const [isSingleCowRunning, setIsSingleCowRunning] = useState<boolean>(false);
  const [singleCowResult, setSingleCowResult] = useState<{
    path: string;
    bytes: number;
    strategy: string;
  } | null>(null);

  const isCopyInvalidated = (copy: PhysicalCopyPayload): boolean =>
    invalidatedFiles.has(copy.primary_path) ||
    copy.all_paths.some((p) => invalidatedFiles.has(p));

  // Check volume capability for CoW extent cloning (APFS or Btrfs)
  useEffect(() => {
    const rootPath = targetDir || (groups[0]?.physical_copies[0]?.primary_path ?? '');
    if (rootPath) {
      checkCowSupport(rootPath)
        .then((supported) => setSupportsCow(supported))
        .catch(() => setSupportsCow(false));
    }
  }, [targetDir, groups]);

  // Single candidate CoW extent deduplication
  const handleSingleCowDeduplicate = async (
    keeperPath: string,
    candidatePath: string
  ) => {
    setIsSingleCowRunning(true);
    setSingleCowResult(null);

    try {
      const res = await executeCowDeduplication(keeperPath, candidatePath);
      setSingleCowResult({
        path: candidatePath,
        bytes: res.bytes_reclaimed,
        strategy: res.strategy,
      });
      onCowDeduplicated([candidatePath], res.bytes_reclaimed);
    } catch (err: any) {
      alert(`CoW Deduplication error: ${err}`);
    } finally {
      setIsSingleCowRunning(false);
    }
  };

  // Batch CoW deduplication completion
  const handleBatchCowComplete = (totalReclaimed: number, dedupedPaths: string[]) => {
    onCowDeduplicated(dedupedPaths, totalReclaimed);
  };

  // Selection & Navigation
  const [selectedGroupHash, setSelectedGroupHash] = useState<string>(
    groups[0]?.blake3_hash || ''
  );
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(
    new Set(groups.slice(0, 5).map((g) => g.blake3_hash))
  );

  const [quickLookOpen, setQuickLookOpen] = useState<boolean>(false);
  const [copiedHash, setCopiedHash] = useState<boolean>(false);

  const searchInputRef = useRef<HTMLInputElement>(null);
  const tableContainerRef = useRef<HTMLDivElement>(null);

  const getKeeperForGroup = (group: DuplicateGroupPayload): PhysicalCopyPayload => {
    const override = keeperOverrides.get(group.blake3_hash);
    if (override) {
      const found = group.physical_copies.find((c) => c.primary_path === override);
      if (found) return found;
    }
    return group.physical_copies[0];
  };

  const filteredGroups = useMemo(() => {
    let result = [...groups];

    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase().trim();
      result = result.filter((g) => {
        const matchesHash = g.blake3_hash.toLowerCase().includes(q);
        const matchesFile = g.physical_copies.some(
          (c) =>
            c.primary_path.toLowerCase().includes(q) ||
            c.all_paths.some((p) => p.toLowerCase().includes(q))
        );
        return matchesHash || matchesFile;
      });
    }

    if (filterScope === 'reclaimable_only') {
      result = result.filter((g) => g.reclaimable_bytes > 0);
    } else if (filterScope === 'has_hardlinks') {
      result = result.filter((g) => g.total_path_count > g.total_physical_copies);
    }

    result.sort((a, b) => {
      switch (sortBy) {
        case 'size_desc':
          return b.size - a.size;
        case 'size_asc':
          return a.size - b.size;
        case 'copies_desc':
          return b.total_physical_copies - a.total_physical_copies;
        case 'name_asc': {
          const nameA = getBasename(a.physical_copies[0]?.primary_path || '');
          const nameB = getBasename(b.physical_copies[0]?.primary_path || '');
          return nameA.localeCompare(nameB);
        }
        default:
          return 0;
      }
    });

    return result;
  }, [groups, searchQuery, sortBy, filterScope]);

  useEffect(() => {
    if (filteredGroups.length > 0) {
      const stillExists = filteredGroups.some((g) => g.blake3_hash === selectedGroupHash);
      if (!stillExists) {
        setSelectedGroupHash(filteredGroups[0].blake3_hash);
        setSelectedFilePath(getKeeperForGroup(filteredGroups[0]).primary_path);
      }
    }
  }, [filteredGroups, selectedGroupHash]);

  const activeGroup = useMemo(() => {
    return (
      filteredGroups.find((g) => g.blake3_hash === selectedGroupHash) ||
      filteredGroups[0] ||
      groups[0]
    );
  }, [filteredGroups, selectedGroupHash, groups]);

  const currentKeeper = activeGroup ? getKeeperForGroup(activeGroup) : null;
  const activeSelectedPath =
    selectedFilePath || (currentKeeper ? currentKeeper.primary_path : null);

  // Flat list for keyboard navigation
  const flatRows = useMemo(() => {
    const rows: FlatRowItem[] = [];

    filteredGroups.forEach((group) => {
      const isExpanded = expandedGroups.has(group.blake3_hash);
      const keeper = getKeeperForGroup(group);

      rows.push({
        id: `group-${group.blake3_hash}`,
        groupHash: group.blake3_hash,
        isGroupHeader: true,
        copy: keeper,
        isKeeper: true,
      });

      if (isExpanded) {
        group.physical_copies.forEach((copy) => {
          const isKeeper = copy.primary_path === keeper.primary_path;
          rows.push({
            id: `copy-${group.blake3_hash}-${copy.primary_path}`,
            groupHash: group.blake3_hash,
            isGroupHeader: false,
            copy,
            isKeeper,
          });

          if (copy.all_paths.length > 1) {
            copy.all_paths
              .filter((p) => p !== copy.primary_path)
              .forEach((aliasPath) => {
                rows.push({
                  id: `alias-${group.blake3_hash}-${aliasPath}`,
                  groupHash: group.blake3_hash,
                  isGroupHeader: false,
                  copy,
                  isKeeper: false,
                  isHardLinkAlias: true,
                  aliasPath,
                });
              });
          }
        });
      }
    });

    return rows;
  }, [filteredGroups, expandedGroups, keeperOverrides]);

  const activeRowIndex = useMemo(() => {
    if (!activeSelectedPath) return 0;
    const idx = flatRows.findIndex(
      (r) =>
        r.copy?.primary_path === activeSelectedPath ||
        r.aliasPath === activeSelectedPath ||
        (r.isGroupHeader && r.groupHash === selectedGroupHash)
    );
    return idx >= 0 ? idx : 0;
  }, [flatRows, activeSelectedPath, selectedGroupHash]);

  const scrollToActiveRow = (index: number) => {
    const el = document.getElementById(`row-item-${index}`);
    if (el) {
      el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  };

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (document.activeElement === searchInputRef.current) {
        if (e.key === 'Escape') {
          searchInputRef.current?.blur();
        }
        return;
      }

      if (e.key === 'j' || e.key === 'ArrowDown') {
        e.preventDefault();
        const nextIdx = Math.min(activeRowIndex + 1, flatRows.length - 1);
        const item = flatRows[nextIdx];
        if (item) {
          setSelectedGroupHash(item.groupHash);
          setSelectedFilePath(item.aliasPath || item.copy?.primary_path || null);
          scrollToActiveRow(nextIdx);
        }
      } else if (e.key === 'k' || e.key === 'ArrowUp') {
        e.preventDefault();
        const prevIdx = Math.max(activeRowIndex - 1, 0);
        const item = flatRows[prevIdx];
        if (item) {
          setSelectedGroupHash(item.groupHash);
          setSelectedFilePath(item.aliasPath || item.copy?.primary_path || null);
          scrollToActiveRow(prevIdx);
        }
      } else if (e.key === ' ') {
        e.preventDefault();
        setQuickLookOpen((prev) => !prev);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const currentRow = flatRows[activeRowIndex];
        if (currentRow && currentRow.copy && !currentRow.isKeeper && !currentRow.isHardLinkAlias) {
          onSetKeeper(currentRow.groupHash, currentRow.copy.primary_path);
        }
      } else if (e.key === 'o' || e.key === 'O') {
        e.preventDefault();
        if (activeSelectedPath) {
          revealInFileManager(activeSelectedPath);
        }
      } else if (e.key === '/' || ((e.metaKey || e.ctrlKey) && e.key === 'f')) {
        e.preventDefault();
        searchInputRef.current?.focus();
      } else if (e.key === 'Escape') {
        if (quickLookOpen) {
          setQuickLookOpen(false);
        } else if (searchQuery) {
          setSearchQuery('');
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [
    activeRowIndex,
    flatRows,
    quickLookOpen,
    searchQuery,
    activeSelectedPath,
    onSetKeeper,
  ]);

  const toggleGroupExpand = (hash: string, e: React.MouseEvent) => {
    e.stopPropagation();
    const next = new Set(expandedGroups);
    if (next.has(hash)) {
      next.delete(hash);
    } else {
      next.add(hash);
    }
    setExpandedGroups(next);
  };

  const handleCopyHash = (hash: string) => {
    navigator.clipboard.writeText(hash);
    setCopiedHash(true);
    setTimeout(() => setCopiedHash(false), 2000);
  };

  const getFileBadgeIcon = (path: string) => {
    const ext = getFileExtension(path);
    switch (ext) {
      case 'png':
      case 'jpg':
      case 'jpeg':
      case 'gif':
      case 'webp':
      case 'svg':
        return <FileImage className="w-3.5 h-3.5 text-pink-400" />;
      case 'mp4':
      case 'mov':
      case 'mkv':
        return <FileVideo className="w-3.5 h-3.5 text-blue-400" />;
      case 'mp3':
      case 'wav':
      case 'flac':
        return <FileAudio className="w-3.5 h-3.5 text-teal-400" />;
      case 'zip':
      case 'tar':
      case 'gz':
      case '7z':
      case 'dmg':
        return <FileArchive className="w-3.5 h-3.5 text-amber-400" />;
      case 'rs':
      case 'ts':
      case 'tsx':
      case 'js':
      case 'json':
      case 'py':
        return <FileCode className="w-3.5 h-3.5 text-emerald-400" />;
      case 'pdf':
      case 'txt':
      case 'md':
        return <FileText className="w-3.5 h-3.5 text-sky-400" />;
      default:
        return <File className="w-3.5 h-3.5 text-zinc-500" />;
    }
  };

  if (!groups || groups.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-zinc-500 select-none bg-canvas">
        <ShieldCheck className="w-10 h-10 text-emerald-400 mb-2 opacity-80" />
        <h3 className="text-sm font-semibold text-zinc-200">No Duplicates Detected</h3>
        <p className="text-xs text-zinc-500 max-w-sm mt-1">
          All scanned files have distinct cryptographic hashes.
        </p>
      </div>
    );
  }

  const selectedDuplicateCopy =
    activeGroup?.physical_copies.find((c) => c.primary_path === activeSelectedPath) ||
    activeGroup?.physical_copies.find((c) => c.primary_path !== currentKeeper?.primary_path) ||
    null;

  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-canvas relative">
      {/* Real-Time Floating Invalidation Toast Notifications */}
      {invalidationToasts.length > 0 && (
        <div className="absolute top-3 right-3 z-50 space-y-2 pointer-events-auto max-w-sm animate-in fade-in duration-150">
          {invalidationToasts.map((toast) => (
            <div
              key={toast.id}
              className="p-2.5 rounded bg-surface border border-amber-500/30 shadow-2xl flex items-start gap-2.5 text-xs font-mono"
            >
              <span className="w-2 h-2 rounded-full bg-amber-400 mt-1 shrink-0 animate-pulse" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center justify-between">
                  <span className="text-amber-300 font-semibold text-[11px]">
                    Candidate Invalidated ({toast.notice.reason})
                  </span>
                  <button
                    onClick={() => onDismissToast(toast.id)}
                    className="text-zinc-500 hover:text-white"
                  >
                    <X className="w-3 h-3" />
                  </button>
                </div>
                <div className="text-zinc-300 truncate mt-0.5" title={toast.notice.path}>
                  {getBasename(toast.notice.path)}
                </div>
                <div className="text-[10px] text-zinc-500 mt-0.5">
                  Remaining in group: {toast.notice.remaining_files_in_group}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 1. Storage Breakdown Bar */}
      {summary && (
        <StorageProportionBar
          summary={summary}
          totalScannedBytes={totalScannedBytes}
          totalScannedFiles={totalScannedFiles}
        />
      )}

      {/* 2. Sleek Filter & Search Bar */}
      <div className="px-3 py-1.5 bg-surface border-b border-border-subtle flex items-center justify-between gap-3 select-none">
        {/* Search Input */}
        <div className="flex-1 max-w-md relative flex items-center">
          <Search className="w-3.5 h-3.5 text-zinc-500 absolute left-2.5 pointer-events-none" />
          <input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search duplicates... (/)"
            className="w-full bg-elevated border border-white/5 rounded pl-8 pr-10 py-1 text-xs text-zinc-200 placeholder:text-zinc-500 focus:outline-none focus:border-white/20 transition-colors font-mono"
          />
          {searchQuery && (
            <button
              onClick={() => setSearchQuery('')}
              className="absolute right-2 text-[10px] text-zinc-500 hover:text-zinc-200"
            >
              Clear
            </button>
          )}
        </div>

        {/* Filter & Sort Controls */}
        <div className="flex items-center gap-2">
          {supportsCow && (
            <button
              onClick={() => setCowModalOpen(true)}
              className="px-2 py-1 rounded bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-300 border border-emerald-500/25 text-xs font-medium flex items-center gap-1.5 transition-colors shrink-0"
              title="Deduplicate via CoW Extents (Zero Movement)"
            >
              <Layers className="w-3.5 h-3.5 text-emerald-400" />
              <span>Deduplicate via CoW Extents</span>
            </button>
          )}

          {/* Filter Scope Pills */}
          <div className="flex items-center gap-0.5 bg-elevated border border-white/5 rounded p-0.5 text-xs">
            <button
              onClick={() => setFilterScope('all')}
              className={`px-2 py-0.5 rounded text-[11px] transition-colors ${
                filterScope === 'all'
                  ? 'bg-subtle text-zinc-100 font-medium'
                  : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              All ({groups.length})
            </button>
            <button
              onClick={() => setFilterScope('reclaimable_only')}
              className={`px-2 py-0.5 rounded text-[11px] transition-colors ${
                filterScope === 'reclaimable_only'
                  ? 'bg-subtle text-emerald-400 font-medium'
                  : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              Reclaimable
            </button>
            <button
              onClick={() => setFilterScope('has_hardlinks')}
              className={`px-2 py-0.5 rounded text-[11px] transition-colors ${
                filterScope === 'has_hardlinks'
                  ? 'bg-subtle text-zinc-300 font-medium'
                  : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              Hard Links
            </button>
          </div>

          {/* Sort Selector */}
          <div className="flex items-center gap-1 bg-elevated border border-white/5 rounded px-2 py-1 text-xs text-zinc-400">
            <ArrowUpDown className="w-3 h-3 text-zinc-500" />
            <select
              value={sortBy}
              onChange={(e) => setSortBy(e.target.value as SortOption)}
              className="bg-transparent text-zinc-300 text-xs focus:outline-none cursor-pointer"
            >
              <option value="size_desc" className="bg-elevated text-zinc-200">Size (Largest)</option>
              <option value="size_asc" className="bg-elevated text-zinc-200">Size (Smallest)</option>
              <option value="copies_desc" className="bg-elevated text-zinc-200">Copies (Most)</option>
              <option value="name_asc" className="bg-elevated text-zinc-200">File Name (A-Z)</option>
            </select>
          </div>
        </div>
      </div>

      {/* 3. Dual-Pane Master-Detail Layout */}
      <div className="flex-1 flex overflow-hidden">
        {/* LEFT PANE: High-Density Table (32px row height) */}
        <div
          ref={tableContainerRef}
          className="flex-1 flex flex-col border-r border-border-subtle bg-canvas overflow-y-auto"
        >
          {filteredGroups.length === 0 ? (
            <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-zinc-500">
              <Filter className="w-6 h-6 text-zinc-600 mb-2" />
              <p className="text-xs">No duplicates match active filter</p>
            </div>
          ) : (
            <div className="divide-y divide-border-subtle">
              {filteredGroups.map((group) => {
                const isGroupSelected = group.blake3_hash === selectedGroupHash;
                const isExpanded = expandedGroups.has(group.blake3_hash);
                const keeper = getKeeperForGroup(group);
                const groupBasename = getBasename(keeper.primary_path);
                const parentDir = getParentDir(keeper.primary_path);

                const groupInvalidCount = group.physical_copies.filter((c) =>
                  isCopyInvalidated(c)
                ).length;
                const remainingValidCopies = group.physical_copies.length - groupInvalidCount;
                const isGroupDissolved = remainingValidCopies <= 1 && groupInvalidCount > 0;

                return (
                  <div
                    key={group.blake3_hash}
                    className={`transition-colors ${
                      isGroupSelected ? 'bg-subtle/30' : 'hover:bg-subtle/15'
                    } ${isGroupDissolved ? 'opacity-60' : ''}`}
                  >
                    {/* Group Header Row */}
                    <div
                      onClick={() => {
                        setSelectedGroupHash(group.blake3_hash);
                        setSelectedFilePath(keeper.primary_path);
                      }}
                      className={`h-8 px-3 flex items-center justify-between cursor-pointer border-l-2 transition-all ${
                        isGroupSelected && activeSelectedPath === keeper.primary_path
                          ? 'border-l-sky-400 bg-sky-500/10'
                          : 'border-l-transparent'
                      }`}
                    >
                      <div className="flex items-center gap-2 min-w-0 flex-1">
                        <button
                          onClick={(e) => toggleGroupExpand(group.blake3_hash, e)}
                          className="p-0.5 rounded hover:bg-elevated text-zinc-500 hover:text-white transition-colors"
                        >
                          {isExpanded ? (
                            <ChevronDown className="w-3 h-3" />
                          ) : (
                            <ChevronRight className="w-3 h-3" />
                          )}
                        </button>

                        <div className="p-0.5 rounded bg-elevated border border-white/5 shrink-0">
                          {getFileBadgeIcon(keeper.primary_path)}
                        </div>

                        <span className="text-xs font-semibold text-zinc-100 truncate">
                          {groupBasename}
                        </span>

                        <span className="text-[10px] tabular-nums font-mono px-1 py-0.2 rounded bg-elevated text-zinc-400 border border-white/5 shrink-0">
                          {formatBytes(group.size)}
                        </span>

                        {isGroupDissolved && (
                          <span className="text-[9px] tabular-nums font-mono font-semibold px-1 py-0.2 rounded bg-rose-500/10 text-rose-300 border border-rose-500/20 shrink-0">
                            GROUP DISSOLVED (1 COPY LEFT)
                          </span>
                        )}

                        <span className="text-[11px] text-zinc-500 truncate font-mono hidden md:inline" title={parentDir}>
                          {parentDir}
                        </span>
                      </div>

                      {/* Right Meta Info */}
                      <div className="flex items-center gap-3 pl-2 shrink-0 text-[11px] tabular-nums font-mono">
                        <span className="text-zinc-500">
                          {group.total_physical_copies} copies
                        </span>
                        <span className="text-emerald-400 font-medium">
                          {formatBytes(group.reclaimable_bytes)}
                        </span>
                        <span className="text-[10px] text-zinc-500">
                          #{truncateHash(group.blake3_hash, 4)}
                        </span>
                      </div>
                    </div>

                    {/* Children File Rows (Strict 32px height) */}
                    {isExpanded && (
                      <div className="border-l border-white/5 ml-4 pl-3 py-0.5 space-y-0.5">
                        {group.physical_copies.map((copy) => {
                          const isKeeper = copy.primary_path === keeper.primary_path;
                          const isRowActive = copy.primary_path === activeSelectedPath;
                          const isInvalidated = isCopyInvalidated(copy);
                          const isCowCloned = cowDeduplicatedPaths.has(copy.primary_path);

                          return (
                            <div key={copy.primary_path} className="space-y-0.5">
                              <div
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setSelectedGroupHash(group.blake3_hash);
                                  setSelectedFilePath(copy.primary_path);
                                }}
                                className={`h-8 px-2 rounded text-xs flex items-center justify-between border cursor-pointer transition-colors ${
                                  isRowActive
                                    ? 'bg-elevated border-sky-500/40 text-white'
                                    : isInvalidated
                                    ? 'bg-amber-500/5 border-amber-500/20 text-amber-200'
                                    : 'bg-surface/30 border-white/5 hover:border-white/10 hover:bg-surface/70 text-zinc-300'
                                }`}
                              >
                                <div className="flex items-center gap-2 min-w-0 flex-1">
                                  {isInvalidated ? (
                                    <span className="inline-flex items-center gap-1 text-[10px] font-mono text-amber-300 shrink-0">
                                      <span className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
                                      CHANGED
                                    </span>
                                  ) : isCowCloned ? (
                                    <span className="text-[10px] tabular-nums font-mono text-emerald-400 shrink-0">
                                      CoW (0 B)
                                    </span>
                                  ) : isKeeper ? (
                                    <span className="inline-flex items-center gap-1 text-[10px] font-medium text-sky-400 shrink-0">
                                      <Check className="w-3 h-3" />
                                      Keeper
                                    </span>
                                  ) : (
                                    <button
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        onSetKeeper(group.blake3_hash, copy.primary_path);
                                      }}
                                      title="Click to set as keeper"
                                      className="text-[10px] font-medium text-zinc-500 hover:text-sky-300 transition-colors shrink-0"
                                    >
                                      Quarantine
                                    </button>
                                  )}

                                  <div className="min-w-0 flex-1 font-mono text-[11px] truncate">
                                    <span className="text-zinc-600">{getParentDir(copy.primary_path)}/</span>
                                    <span className={`font-medium ${isInvalidated ? 'text-amber-300 line-through' : 'text-zinc-200'}`}>
                                      {getBasename(copy.primary_path)}
                                    </span>
                                  </div>
                                </div>

                                <div className="flex items-center gap-2 shrink-0 pl-2">
                                  <span className="text-[10px] tabular-nums font-mono text-zinc-600">
                                    inode: {copy.inode}
                                  </span>

                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      setSelectedFilePath(copy.primary_path);
                                      setQuickLookOpen(true);
                                    }}
                                    title="Quick Look (Space)"
                                    className="p-1 text-zinc-500 hover:text-zinc-200 transition-colors"
                                  >
                                    <Eye className="w-3 h-3" />
                                  </button>

                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      revealInFileManager(copy.primary_path);
                                    }}
                                    title="Reveal in Finder / Explorer (O)"
                                    className="p-1 text-zinc-500 hover:text-zinc-200 transition-colors"
                                  >
                                    <FolderOpen className="w-3 h-3" />
                                  </button>
                                </div>
                              </div>

                              {/* Hard-Link Inode Clones */}
                              {copy.all_paths.length > 1 && (
                                <div className="pl-5 space-y-0.5">
                                  {copy.all_paths
                                    .filter((p) => p !== copy.primary_path)
                                    .map((aliasPath) => (
                                      <div
                                        key={aliasPath}
                                        onClick={(e) => {
                                          e.stopPropagation();
                                          setSelectedGroupHash(group.blake3_hash);
                                          setSelectedFilePath(aliasPath);
                                        }}
                                        className={`h-6 px-2 rounded text-[10px] font-mono flex items-center justify-between border cursor-pointer transition-colors ${
                                          aliasPath === activeSelectedPath
                                            ? 'bg-elevated border-white/20 text-white'
                                            : 'bg-subtle/20 border-white/5 text-zinc-400 hover:bg-subtle/40'
                                        }`}
                                      >
                                        <div className="flex items-center gap-1.5 min-w-0 flex-1 truncate">
                                          <span className="text-[9px] text-zinc-500">ALIAS (0 B)</span>
                                          <span className="truncate text-zinc-400">{aliasPath}</span>
                                        </div>
                                        <button
                                          onClick={(e) => {
                                            e.stopPropagation();
                                            revealInFileManager(aliasPath);
                                          }}
                                          className="p-0.5 text-zinc-500 hover:text-zinc-200"
                                        >
                                          <FolderOpen className="w-2.5 h-2.5" />
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
          )}
        </div>

        {/* RIGHT PANE: Comparison & Properties Inspector */}
        <div className="w-[420px] bg-surface flex flex-col border-l border-border-subtle overflow-y-auto select-none shrink-0">
          <div className="h-8 px-3 border-b border-border-subtle bg-elevated/40 flex items-center justify-between">
            <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500">
              Inspector
            </span>
            <span className="text-[10px] font-mono text-zinc-600">
              Detail View
            </span>
          </div>

          {activeGroup && currentKeeper ? (
            <div className="p-3.5 space-y-3.5">
              {/* BLAKE3 Cryptographic Hash */}
              <div className="p-2.5 rounded bg-elevated/60 border border-white/5 space-y-1.5">
                <div className="flex items-center justify-between text-xs text-zinc-400">
                  <span className="flex items-center gap-1 text-[11px] font-mono">
                    <Hash className="w-3 h-3 text-zinc-500" />
                    BLAKE3 Hash
                  </span>
                  <button
                    onClick={() => handleCopyHash(activeGroup.blake3_hash)}
                    className="text-[10px] font-mono text-zinc-400 hover:text-white flex items-center gap-1 transition-colors"
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
                <div className="p-1.5 rounded bg-canvas border border-white/5 font-mono text-[10px] tabular-nums text-zinc-300 break-all select-all">
                  {activeGroup.blake3_hash}
                </div>
                <div className="text-[10px] font-mono text-emerald-400 flex items-center gap-1">
                  <Check className="w-3 h-3" />
                  <span>Bit-for-Bit Identical Content Match Verified</span>
                </div>
              </div>

              {/* Keeper File Card */}
              <div className="p-3 rounded bg-elevated/60 border border-white/5 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-mono uppercase tracking-wider text-sky-400 font-semibold flex items-center gap-1">
                    <ShieldCheck className="w-3.5 h-3.5" /> Keeper (Preserved)
                  </span>
                  <span className="text-[10px] font-mono text-zinc-500">
                    inode: {currentKeeper.inode}
                  </span>
                </div>

                <div className="min-w-0">
                  <div className="text-xs font-semibold text-zinc-100 truncate">
                    {getBasename(currentKeeper.primary_path)}
                  </div>
                  <div className="font-mono text-[10px] text-zinc-500 truncate mt-0.5" title={currentKeeper.primary_path}>
                    {currentKeeper.primary_path}
                  </div>
                </div>

                <div className="flex items-center justify-between text-[11px] font-mono pt-1 border-t border-white/5 text-zinc-400">
                  <span>Size: {formatBytes(activeGroup.size)}</span>
                  <button
                    onClick={() => revealInFileManager(currentKeeper.primary_path)}
                    className="text-[10px] text-zinc-400 hover:text-white flex items-center gap-1"
                  >
                    <FolderOpen className="w-3 h-3" /> Reveal in Finder
                  </button>
                </div>
              </div>

              {/* Duplicate Candidate Card */}
              {selectedDuplicateCopy && selectedDuplicateCopy.primary_path !== currentKeeper.primary_path ? (
                <div className="p-3 rounded bg-elevated/60 border border-white/5 space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[10px] font-mono uppercase tracking-wider text-amber-400 font-semibold">
                      Candidate Duplicate
                    </span>
                    <span className="text-[10px] font-mono text-zinc-500">
                      inode: {selectedDuplicateCopy.inode}
                      {selectedDuplicateCopy.inode === currentKeeper.inode && ' (Hard Link)'}
                    </span>
                  </div>

                  {isCopyInvalidated(selectedDuplicateCopy) ? (
                    <div className="p-2 rounded bg-amber-500/10 border border-amber-500/25 space-y-1 text-xs">
                      <div className="font-semibold text-amber-300 text-[11px] flex items-center gap-1">
                        <AlertTriangle className="w-3.5 h-3.5" />
                        Disqualified on Disk
                      </div>
                      <p className="text-[10px] text-zinc-400 font-mono">
                        File was altered after scan. Operation refused to prevent data loss.
                      </p>
                    </div>
                  ) : (
                    <>
                      <div className="min-w-0">
                        <div className="text-xs font-semibold text-zinc-100 truncate">
                          {getBasename(selectedDuplicateCopy.primary_path)}
                        </div>
                        <div className="font-mono text-[10px] text-zinc-500 truncate mt-0.5" title={selectedDuplicateCopy.primary_path}>
                          {selectedDuplicateCopy.primary_path}
                        </div>
                      </div>

                      <div className="p-1.5 rounded bg-canvas border border-white/5 text-[10px] font-mono text-zinc-500 truncate">
                        Quarantine: .../{truncateHash(activeGroup.blake3_hash, 4)}/{getBasename(selectedDuplicateCopy.primary_path)}
                      </div>

                      {/* Action Buttons */}
                      <div className="space-y-1.5 pt-1">
                        {/* CoW Deduplication Button */}
                        {supportsCow && (
                          <button
                            onClick={() =>
                              handleSingleCowDeduplicate(
                                currentKeeper.primary_path,
                                selectedDuplicateCopy.primary_path
                              )
                            }
                            disabled={
                              isSingleCowRunning ||
                              cowDeduplicatedPaths.has(selectedDuplicateCopy.primary_path) ||
                              isCopyInvalidated(currentKeeper)
                            }
                            className="w-full py-1.5 px-2.5 rounded bg-emerald-500/15 hover:bg-emerald-500/25 disabled:bg-subtle disabled:text-zinc-600 text-emerald-300 text-xs font-medium border border-emerald-500/30 transition-colors flex items-center justify-center gap-1.5"
                          >
                            <Layers className="w-3.5 h-3.5" />
                            <span>
                              {cowDeduplicatedPaths.has(selectedDuplicateCopy.primary_path)
                                ? 'Extents Unified (0 B Physical)'
                                : isSingleCowRunning
                                ? 'Cloning extents in place...'
                                : 'Deduplicate Extents in Place (APFS)'}
                            </span>
                          </button>
                        )}

                        <div className="flex gap-2">
                          <button
                            onClick={() => onSetKeeper(activeGroup.blake3_hash, selectedDuplicateCopy.primary_path)}
                            className="flex-1 py-1 px-2 rounded bg-elevated hover:bg-subtle text-zinc-200 text-xs font-medium border border-white/10 transition-colors"
                          >
                            Set as Keeper (Enter)
                          </button>
                          <button
                            onClick={() => revealInFileManager(selectedDuplicateCopy.primary_path)}
                            className="p-1 rounded bg-elevated hover:bg-subtle text-zinc-400 hover:text-white border border-white/5 transition-colors"
                            title="Reveal in Finder / Explorer (O)"
                          >
                            <FolderOpen className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </div>

                      {singleCowResult &&
                        singleCowResult.path === selectedDuplicateCopy.primary_path && (
                          <div className="p-2 rounded bg-emerald-500/10 border border-emerald-500/20 text-[10px] font-mono text-emerald-300">
                            ✓ Unified {formatBytes(singleCowResult.bytes)} via {singleCowResult.strategy}
                          </div>
                        )}
                    </>
                  )}
                </div>
              ) : (
                <div className="p-4 rounded bg-elevated/40 border border-white/5 text-center text-xs text-zinc-500">
                  Select any duplicate copy to inspect or deduplicate
                </div>
              )}
            </div>
          ) : (
            <div className="p-6 text-center text-xs text-zinc-600">
              Select a group to inspect properties
            </div>
          )}
        </div>
      </div>

      {/* Quick Look Modal */}
      {quickLookOpen && activeSelectedPath && activeGroup && (
        <QuickLookModal
          filePath={activeSelectedPath}
          isKeeper={activeSelectedPath === currentKeeper?.primary_path}
          size={activeGroup.size}
          blake3Hash={activeGroup.blake3_hash}
          inode={
            activeGroup.physical_copies.find((c) => c.primary_path === activeSelectedPath)
              ?.inode
          }
          deviceId={
            activeGroup.physical_copies.find((c) => c.primary_path === activeSelectedPath)
              ?.device_id
          }
          onClose={() => setQuickLookOpen(false)}
          onSetAsKeeper={() => {
            onSetKeeper(activeGroup.blake3_hash, activeSelectedPath);
            setQuickLookOpen(false);
          }}
        />
      )}

      {/* Batch CoW Modal */}
      {cowModalOpen && (
        <CowDeduplicationModal
          isOpen={cowModalOpen}
          onClose={() => setCowModalOpen(false)}
          groups={filteredGroups}
          keeperOverrides={keeperOverrides}
          invalidatedFiles={invalidatedFiles}
          cowDeduplicatedPaths={cowDeduplicatedPaths}
          onComplete={handleBatchCowComplete}
        />
      )}
    </div>
  );
};
