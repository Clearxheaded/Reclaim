import React, { useState, useEffect, useRef, useMemo } from 'react';
import {
  ShieldCheck,
  FolderOpen,
  ChevronRight,
  ChevronDown,
  Copy,
  Hash,
  Check,
  AlertCircle,
  AlertTriangle,
  Search,
  ArrowUpDown,
  Filter,
  Eye,
  Sparkles,
  Layers,
  CheckCircle2,
  X,
  FileCode,
  FileText,
  FileImage,
  FileArchive,
  FileAudio,
  FileVideo,
  File,
} from 'lucide-react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
  CandidateInvalidatedEvent,
  DuplicateGroupPayload,
  DuplicateSummaryPayload,
  PhysicalCopyPayload,
} from '../types/reclaim';
import {
  formatBytes,
  getBasename,
  getParentDir,
  getFileExtension,
  truncateHash,
} from '../utils/format';
import { checkCowSupport, executeCowDeduplication, isTauri, revealInFileManager } from '../api/ipc';
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
  onSetKeeper: (groupHash: string, keeperPath: string) => void;
  onSummaryUpdate?: (updatedSummary: DuplicateSummaryPayload) => void;
}

interface ToastItem {
  id: string;
  notice: CandidateInvalidatedEvent;
  timestamp: number;
}

type SortOption = 'size_desc' | 'size_asc' | 'copies_desc' | 'name_asc';
type FilterOption = 'all' | 'reclaimable_only' | 'has_hardlinks';

interface FlatRowItem {
  id: string; // unique item id
  groupHash: string;
  groupIdx: number;
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
  onSetKeeper,
  onSummaryUpdate,
}) => {
  // Search & Filter state (Raycast style)
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [sortBy, setSortBy] = useState<SortOption>('size_desc');
  const [filterScope, setFilterScope] = useState<FilterOption>('all');

  // Dynamic Summary State (updated without full rescan on TOCTOU / CoW events)
  const [currentSummary, setCurrentSummary] = useState<DuplicateSummaryPayload | undefined>(summary);
  useEffect(() => {
    setCurrentSummary(summary);
  }, [summary]);

  // Real-Time TOCTOU Defense State
  const [invalidatedFiles, setInvalidatedFiles] = useState<Map<string, CandidateInvalidatedEvent>>(
    new Map()
  );
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  // Copy-on-Write (CoW) Extent Deduplication State
  const [supportsCow, setSupportsCow] = useState<boolean>(false);
  const [cowModalOpen, setCowModalOpen] = useState<boolean>(false);
  const [cowDeduplicatedPaths, setCowDeduplicatedPaths] = useState<Set<string>>(new Set());
  const [isSingleCowRunning, setIsSingleCowRunning] = useState<boolean>(false);
  const [singleCowResult, setSingleCowResult] = useState<{
    path: string;
    bytes: number;
    strategy: string;
  } | null>(null);

  // Check volume capability for CoW extent cloning (APFS or Btrfs/XFS)
  useEffect(() => {
    const rootPath = targetDir || (groups[0]?.physical_copies[0]?.primary_path ?? '');
    if (rootPath) {
      checkCowSupport(rootPath)
        .then((supported) => setSupportsCow(supported))
        .catch(() => setSupportsCow(false));
    }
  }, [targetDir, groups]);

  // Active TOCTOU Defense: Handle continuous kernel filesystem invalidations
  const handleCandidateInvalidated = (notice: CandidateInvalidatedEvent) => {
    // 1. Record invalidated candidate
    setInvalidatedFiles((prev) => {
      const next = new Map(prev);
      next.set(notice.path, notice);
      return next;
    });

    // 2. Spawn animated warning toast
    const toastId = `${Date.now()}-${Math.random()}`;
    setToasts((prev) => [
      ...prev.slice(-3),
      { id: toastId, notice, timestamp: Date.now() },
    ]);

    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== toastId));
    }, 7000);

    // 3. Dynamically update DaisyDisk proportion metrics without manual rescan
    setCurrentSummary((prev) => {
      if (!prev) return prev;
      const isGroupDissolved = notice.remaining_files_in_group <= 1;
      const updated: DuplicateSummaryPayload = {
        ...prev,
        reclaimable_bytes: notice.new_reclaimable_bytes,
        duplicate_files_count: Math.max(0, prev.duplicate_files_count - 1),
        duplicate_groups_count: isGroupDissolved
          ? Math.max(0, prev.duplicate_groups_count - 1)
          : prev.duplicate_groups_count,
      };
      onSummaryUpdate?.(updated);
      return updated;
    });
  };

  // Listen to Tauri "reclaim://candidate-invalidated" stream
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;

    listen<CandidateInvalidatedEvent>('reclaim://candidate-invalidated', (event) => {
      handleCandidateInvalidated(event.payload);
    }).then((unsub) => {
      unlisten = unsub;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // Execute single candidate CoW extent deduplication
  const handleSingleCowDeduplicate = async (
    keeperPath: string,
    candidatePath: string
  ) => {
    setIsSingleCowRunning(true);
    setSingleCowResult(null);

    try {
      const res = await executeCowDeduplication(keeperPath, candidatePath);
      setCowDeduplicatedPaths((prev) => new Set(prev).add(candidatePath));
      setSingleCowResult({
        path: candidatePath,
        bytes: res.bytes_reclaimed,
        strategy: res.strategy,
      });

      // Dynamically adjust summary storage metrics
      setCurrentSummary((prev) => {
        if (!prev) return prev;
        const updated = {
          ...prev,
          reclaimable_bytes: Math.max(0, prev.reclaimable_bytes - res.bytes_reclaimed),
          hard_link_savings_bytes: prev.hard_link_savings_bytes + res.bytes_reclaimed,
        };
        onSummaryUpdate?.(updated);
        return updated;
      });
    } catch (err: any) {
      alert(`CoW Deduplication error: ${err}`);
    } finally {
      setIsSingleCowRunning(false);
    }
  };

  // Handle batch CoW deduplication completion
  const handleBatchCowComplete = (totalReclaimed: number, dedupedPaths: string[]) => {
    setCowDeduplicatedPaths((prev) => {
      const next = new Set(prev);
      dedupedPaths.forEach((p) => next.add(p));
      return next;
    });

    setCurrentSummary((prev) => {
      if (!prev) return prev;
      const updated = {
        ...prev,
        reclaimable_bytes: Math.max(0, prev.reclaimable_bytes - totalReclaimed),
        hard_link_savings_bytes: prev.hard_link_savings_bytes + totalReclaimed,
      };
      onSummaryUpdate?.(updated);
      return updated;
    });
  };

  // Navigation & Selection state
  const [selectedGroupHash, setSelectedGroupHash] = useState<string>(
    groups[0]?.blake3_hash || ''
  );
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(
    new Set(groups.slice(0, 5).map((g) => g.blake3_hash))
  );

  // Quick Look Preview state (Spacebar triggered)
  const [quickLookOpen, setQuickLookOpen] = useState<boolean>(false);
  const [copiedHash, setCopiedHash] = useState<boolean>(false);

  // Refs for keyboard navigation & focus
  const searchInputRef = useRef<HTMLInputElement>(null);
  const tableContainerRef = useRef<HTMLDivElement>(null);

  // Helper to determine active keeper for a group
  const getKeeperForGroup = (group: DuplicateGroupPayload): PhysicalCopyPayload => {
    const override = keeperOverrides.get(group.blake3_hash);
    if (override) {
      const found = group.physical_copies.find((c) => c.primary_path === override);
      if (found) return found;
    }
    return group.physical_copies[0];
  };

  // Filtered & Sorted groups
  const filteredGroups = useMemo(() => {
    let result = [...groups];

    // Search query filter (matches filename, path, or extension)
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

    // Filter scope
    if (filterScope === 'reclaimable_only') {
      result = result.filter((g) => g.reclaimable_bytes > 0);
    } else if (filterScope === 'has_hardlinks') {
      result = result.filter((g) => g.total_path_count > g.total_physical_copies);
    }

    // Sorting
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

  // Keep selected group in sync with filtered list
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

  // Flat list of selectable rows for j/k keyboard navigation (Raycast + Panic style)
  const flatRows = useMemo(() => {
    const rows: FlatRowItem[] = [];

    filteredGroups.forEach((group, gIdx) => {
      const isExpanded = expandedGroups.has(group.blake3_hash);
      const keeper = getKeeperForGroup(group);

      // Group header row
      rows.push({
        id: `group-${group.blake3_hash}`,
        groupHash: group.blake3_hash,
        groupIdx: gIdx,
        isGroupHeader: true,
        copy: keeper,
        isKeeper: true,
      });

      // Child file copy rows (if expanded)
      if (isExpanded) {
        group.physical_copies.forEach((copy) => {
          const isKeeper = copy.primary_path === keeper.primary_path;
          rows.push({
            id: `copy-${group.blake3_hash}-${copy.primary_path}`,
            groupHash: group.blake3_hash,
            groupIdx: gIdx,
            isGroupHeader: false,
            copy,
            isKeeper,
          });

          // Hard link alias rows
          if (copy.all_paths.length > 1) {
            copy.all_paths
              .filter((p) => p !== copy.primary_path)
              .forEach((aliasPath) => {
                rows.push({
                  id: `alias-${group.blake3_hash}-${aliasPath}`,
                  groupHash: group.blake3_hash,
                  groupIdx: gIdx,
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

  // Find index of currently active row
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

  // Scroll active row into view smoothly
  const scrollToActiveRow = (index: number) => {
    const el = document.getElementById(`row-item-${index}`);
    if (el) {
      el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  };

  // Keyboard navigation listener (Raycast & Panic Transmit: j/k, Space, Enter, o, /)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Don't intercept if user is typing in the search input
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
        // Spacebar: Quick Look preview
        e.preventDefault();
        setQuickLookOpen((prev) => !prev);
      } else if (e.key === 'Enter') {
        // Enter: promote currently selected file copy to Protected Keeper
        e.preventDefault();
        const currentRow = flatRows[activeRowIndex];
        if (currentRow && currentRow.copy && !currentRow.isKeeper && !currentRow.isHardLinkAlias) {
          onSetKeeper(currentRow.groupHash, currentRow.copy.primary_path);
        }
      } else if (e.key === 'o' || e.key === 'O') {
        // O: reveal in file manager
        e.preventDefault();
        if (activeSelectedPath) {
          revealInFileManager(activeSelectedPath);
        }
      } else if (e.key === '/' || ((e.metaKey || e.ctrlKey) && e.key === 'f')) {
        // / or Cmd+F: focus filter input
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
        return <FileVideo className="w-3.5 h-3.5 text-violet-400" />;
      case 'mp3':
      case 'wav':
      case 'flac':
        return <FileAudio className="w-3.5 h-3.5 text-cyan-400" />;
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
        return <FileText className="w-3.5 h-3.5 text-blue-400" />;
      default:
        return <File className="w-3.5 h-3.5 text-gray-400" />;
    }
  };

  if (!groups || groups.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-gray-400 select-none">
        <ShieldCheck className="w-12 h-12 text-emerald-400 mb-3" />
        <h3 className="text-base font-semibold text-white">No Duplicate Files Detected</h3>
        <p className="text-xs text-gray-400 max-w-sm mt-1">
          All scanned files have unique cryptographic BLAKE3 content hashes. No cleanup is required.
        </p>
      </div>
    );
  }

  // Selected copy for comparison
  const selectedDuplicateCopy =
    activeGroup?.physical_copies.find((c) => c.primary_path === activeSelectedPath) ||
    activeGroup?.physical_copies.find((c) => c.primary_path !== currentKeeper?.primary_path) ||
    null;

  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-canvas relative">
      {/* Active TOCTOU Defense: Real-Time Floating Invalidation Toast Notifications */}
      {toasts.length > 0 && (
        <div className="absolute top-4 right-4 z-50 space-y-2 pointer-events-auto max-w-sm animate-in fade-in slide-in-from-top-3 duration-300">
          {toasts.map((toast) => (
            <div
              key={toast.id}
              className="p-3 rounded-lg bg-surface/95 border border-amber-500/50 shadow-2xl backdrop-blur-md flex items-start gap-3"
            >
              <div className="p-1 rounded bg-amber-500/20 text-amber-400 mt-0.5 shrink-0 animate-pulse">
                <AlertTriangle className="w-4 h-4" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs font-bold text-amber-300">
                    File modified on disk — candidate invalidated
                  </span>
                  <button
                    onClick={() => setToasts((prev) => prev.filter((t) => t.id !== toast.id))}
                    className="text-gray-400 hover:text-white transition-colors"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
                <div
                  className="text-[11px] font-mono text-gray-200 truncate mt-0.5"
                  title={toast.notice.path}
                >
                  {getBasename(toast.notice.path)}
                </div>
                <div className="text-[10px] text-amber-400/90 font-mono mt-1 flex items-center gap-1.5">
                  <span className="font-semibold uppercase tracking-wider text-[9px] px-1 py-0.2 rounded bg-amber-500/20 text-amber-300">
                    TOCTOU DEFENSE
                  </span>
                  <span>•</span>
                  <span>{toast.notice.remaining_files_in_group} copies remaining</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 1. DaisyDisk-style Proportional Storage Bar in Header (Dynamically updated) */}
      {currentSummary && (
        <StorageProportionBar
          summary={currentSummary}
          totalScannedBytes={totalScannedBytes}
          totalScannedFiles={totalScannedFiles}
        />
      )}

      {/* 2. Raycast-style Filter & Command Bar */}
      <div className="p-2.5 bg-surface border-b border-border-subtle flex items-center justify-between gap-3 select-none">
        {/* Search Bar */}
        <div className="flex-1 max-w-md relative flex items-center">
          <Search className="w-3.5 h-3.5 text-gray-400 absolute left-2.5 pointer-events-none" />
          <input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search duplicates by name, extension, or path... (/)"
            className="w-full bg-elevated border border-border-subtle rounded-md pl-8 pr-12 py-1.5 text-xs text-gray-200 placeholder:text-gray-400 focus:outline-none focus:border-sky-500/60 focus:ring-1 focus:ring-sky-500/30 transition-all font-mono"
          />
          {searchQuery && (
            <button
              onClick={() => setSearchQuery('')}
              className="absolute right-2 text-[10px] text-gray-400 hover:text-white"
            >
              Clear
            </button>
          )}
        </div>

        {/* Filters, CoW Extents Action & Sorting */}
        <div className="flex items-center gap-2">
          {/* Volume CoW Capability Badge & Trigger */}
          {supportsCow && (
            <button
              onClick={() => setCowModalOpen(true)}
              className="px-2.5 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-300 border border-emerald-500/40 text-[11px] font-semibold flex items-center gap-1.5 transition-colors shadow-sm shrink-0"
              title="Deduplicate via CoW Extents (Zero Movement) across duplicate groups"
            >
              <Layers className="w-3.5 h-3.5 text-emerald-400" />
              <span>Deduplicate via CoW Extents (Zero Movement)</span>
            </button>
          )}

          {/* Browser / Prototype TOCTOU simulator */}
          {!isTauri() && (
            <button
              onClick={() => {
                const targetCandidate =
                  groups[0]?.physical_copies[1]?.primary_path ||
                  '/Volumes/Data/Projects/Backup_Copy/Cinematic_Reel_4K_Copy.mov';
                handleCandidateInvalidated({
                  path: targetCandidate,
                  reason: 'Modified',
                  group_hash: groups[0]?.blake3_hash || 'hash1',
                  remaining_files_in_group: 1,
                  new_reclaimable_bytes: Math.max(
                    0,
                    (currentSummary?.reclaimable_bytes || 2806307056) -
                      (groups[0]?.size || 1420584960)
                  ),
                });
              }}
              className="px-2 py-1 rounded text-[10px] font-mono bg-amber-500/20 text-amber-300 border border-amber-500/40 hover:bg-amber-500/30 transition-colors shrink-0"
              title="Simulate active TOCTOU kernel event in browser"
            >
              ⚡ Test TOCTOU Event
            </button>
          )}

          {/* Scope Filter */}
          <div className="flex items-center gap-1 bg-elevated border border-border-subtle rounded-md p-0.5 text-xs">
            <button
              onClick={() => setFilterScope('all')}
              className={`px-2 py-1 rounded text-[11px] font-medium transition-colors ${
                filterScope === 'all'
                  ? 'bg-subtle text-white font-semibold'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              All ({groups.length})
            </button>
            <button
              onClick={() => setFilterScope('reclaimable_only')}
              className={`px-2 py-1 rounded text-[11px] font-medium transition-colors ${
                filterScope === 'reclaimable_only'
                  ? 'bg-subtle text-emerald-400 font-semibold'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              Reclaimable
            </button>
            <button
              onClick={() => setFilterScope('has_hardlinks')}
              className={`px-2 py-1 rounded text-[11px] font-medium transition-colors ${
                filterScope === 'has_hardlinks'
                  ? 'bg-subtle text-purple-400 font-semibold'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              Hard Links
            </button>
          </div>

          {/* Sort Selector */}
          <div className="flex items-center gap-1 bg-elevated border border-border-subtle rounded-md px-2 py-1 text-xs text-gray-400">
            <ArrowUpDown className="w-3 h-3 text-sky-400" />
            <select
              value={sortBy}
              onChange={(e) => setSortBy(e.target.value as SortOption)}
              className="bg-transparent text-gray-200 text-xs focus:outline-none cursor-pointer"
            >
              <option value="size_desc" className="bg-elevated text-gray-200">Size (Largest First)</option>
              <option value="size_asc" className="bg-elevated text-gray-200">Size (Smallest First)</option>
              <option value="copies_desc" className="bg-elevated text-gray-200">Copies (Most First)</option>
              <option value="name_asc" className="bg-elevated text-gray-200">File Name (A-Z)</option>
            </select>
          </div>
        </div>

        {/* Keyboard Shortcut Hints */}
        <div className="hidden lg:flex items-center gap-2 text-[10px] text-gray-400 font-mono">
          <span className="flex items-center gap-0.5">
            <kbd className="px-1 py-0.5 rounded bg-elevated border border-border-subtle text-gray-300">j</kbd>
            <kbd className="px-1 py-0.5 rounded bg-elevated border border-border-subtle text-gray-300">k</kbd>
            <span className="ml-1 text-gray-400">nav</span>
          </span>
          <span>•</span>
          <span className="flex items-center gap-0.5">
            <kbd className="px-1 py-0.5 rounded bg-elevated border border-border-subtle text-gray-300">space</kbd>
            <span className="ml-1 text-gray-400">preview</span>
          </span>
          <span>•</span>
          <span className="flex items-center gap-0.5">
            <kbd className="px-1 py-0.5 rounded bg-elevated border border-border-subtle text-gray-300">enter</kbd>
            <span className="ml-1 text-gray-400">set keeper</span>
          </span>
          <span>•</span>
          <span className="flex items-center gap-0.5">
            <kbd className="px-1 py-0.5 rounded bg-elevated border border-border-subtle text-gray-300">o</kbd>
            <span className="ml-1 text-gray-400">reveal</span>
          </span>
        </div>
      </div>

      {/* 3. Dual-Pane Viewport (Panic Transmit Style) */}
      <div className="flex-1 flex overflow-hidden">
        {/* LEFT PANE: High-Density File Table */}
        <div
          ref={tableContainerRef}
          className="flex-1 flex flex-col border-r border-border-subtle bg-canvas overflow-y-auto"
        >
          {filteredGroups.length === 0 ? (
            <div className="flex-1 flex flex-col items-center justify-center p-8 text-center text-gray-500">
              <Filter className="w-8 h-8 text-gray-600 mb-2" />
              <p className="text-xs">No duplicates match the active filter or search term</p>
            </div>
          ) : (
            <div className="divide-y divide-border-subtle/40">
              {filteredGroups.map((group) => {
                const isGroupSelected = group.blake3_hash === selectedGroupHash;
                const isExpanded = expandedGroups.has(group.blake3_hash);
                const keeper = getKeeperForGroup(group);
                const groupBasename = getBasename(keeper.primary_path);
                const parentDir = getParentDir(keeper.primary_path);

                // Group-level invalidation analysis
                const groupInvalidCount = group.physical_copies.filter((c) =>
                  invalidatedFiles.has(c.primary_path)
                ).length;
                const remainingValidCopies = group.physical_copies.length - groupInvalidCount;
                const isGroupDissolved = remainingValidCopies <= 1 && groupInvalidCount > 0;

                return (
                  <div
                    key={group.blake3_hash}
                    className={`transition-colors ${
                      isGroupSelected ? 'bg-subtle/40' : 'hover:bg-subtle/20'
                    } ${isGroupDissolved ? 'opacity-75' : ''}`}
                  >
                    {/* High-Density Group Row Header */}
                    <div
                      onClick={() => {
                        setSelectedGroupHash(group.blake3_hash);
                        setSelectedFilePath(keeper.primary_path);
                      }}
                      className={`p-2.5 flex items-center justify-between cursor-pointer border-l-2 transition-all ${
                        isGroupSelected && activeSelectedPath === keeper.primary_path
                          ? 'border-l-sky-400 bg-sky-500/10'
                          : 'border-l-transparent'
                      }`}
                    >
                      <div className="flex items-center gap-2.5 min-w-0 flex-1">
                        <button
                          onClick={(e) => toggleGroupExpand(group.blake3_hash, e)}
                          className="p-1 rounded hover:bg-elevated text-gray-400 hover:text-white transition-colors"
                        >
                          {isExpanded ? (
                            <ChevronDown className="w-3.5 h-3.5" />
                          ) : (
                            <ChevronRight className="w-3.5 h-3.5" />
                          )}
                        </button>

                        <div className="p-1 rounded bg-elevated border border-border-subtle">
                          {getFileBadgeIcon(keeper.primary_path)}
                        </div>

                        <div className="min-w-0 flex-1">
                          <div className="text-xs font-semibold text-gray-100 flex items-center gap-2 truncate">
                            <span className="truncate">{groupBasename}</span>
                            <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-elevated text-gray-400 border border-border-subtle shrink-0">
                              {formatBytes(group.size)}
                            </span>
                            {isGroupDissolved && (
                              <span className="text-[9px] font-bold px-1.5 py-0.5 rounded bg-rose-500/20 text-rose-300 border border-rose-500/40 shrink-0">
                                GROUP DISSOLVED (1 COPY LEFT)
                              </span>
                            )}
                          </div>
                          <div className="text-[11px] text-gray-400 truncate font-mono mt-0.5 flex items-center gap-2">
                            <span className="truncate" title={parentDir}>{parentDir}</span>
                            <span>•</span>
                            <span className="text-sky-400 font-medium shrink-0">
                              {group.total_physical_copies} copies
                            </span>
                            <span>•</span>
                            <span className="text-emerald-400 font-medium shrink-0">
                              {formatBytes(group.reclaimable_bytes)} reclaimable
                            </span>
                          </div>
                        </div>
                      </div>

                      {/* Right Hash tag */}
                      <div className="text-right pl-3 shrink-0">
                        <span className="text-[10px] font-mono text-gray-500 bg-canvas px-1.5 py-0.5 rounded border border-border-subtle">
                          #{truncateHash(group.blake3_hash, 4)}
                        </span>
                      </div>
                    </div>

                    {/* High-Density Children: Copies and Aliases */}
                    {isExpanded && (
                      <div className="pl-9 pr-3 pb-2 pt-0.5 space-y-1">
                        {group.physical_copies.map((copy) => {
                          const isKeeper = copy.primary_path === keeper.primary_path;
                          const isRowActive = copy.primary_path === activeSelectedPath;
                          const isInvalidated = invalidatedFiles.has(copy.primary_path);
                          const isCowCloned = cowDeduplicatedPaths.has(copy.primary_path);

                          return (
                            <div key={copy.primary_path} className="space-y-1">
                              {/* File Row */}
                              <div
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setSelectedGroupHash(group.blake3_hash);
                                  setSelectedFilePath(copy.primary_path);
                                }}
                                className={`p-2 rounded-md text-xs flex items-center justify-between border cursor-pointer transition-all ${
                                  isRowActive
                                    ? 'bg-elevated border-sky-500/50 shadow-sm ring-1 ring-sky-500/20'
                                    : isInvalidated
                                    ? 'bg-amber-500/5 border-amber-500/30'
                                    : 'bg-surface/50 border-border-subtle hover:border-border-muted hover:bg-surface'
                                }`}
                              >
                                <div className="flex items-center gap-2.5 min-w-0 flex-1">
                                  {isInvalidated ? (
                                    <span
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-bold bg-amber-500/20 text-amber-300 border border-amber-500/40 shrink-0 animate-pulse"
                                      title="Modified on disk after scan completed. TOCTOU defense triggered."
                                    >
                                      <AlertTriangle className="w-3 h-3 text-amber-400" />
                                      CHANGED ON DISK
                                    </span>
                                  ) : isCowCloned ? (
                                    <span
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-bold bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0"
                                      title="Extent cloned via APFS clonefile(2) / FICLONE. Consumes 0 B physical storage."
                                    >
                                      <Layers className="w-3 h-3" />
                                      COW EXTENT CLONED (0 B)
                                    </span>
                                  ) : isKeeper ? (
                                    <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-bold bg-sky-500/10 text-sky-400 border border-sky-500/30 shrink-0">
                                      <ShieldCheck className="w-3 h-3" />
                                      KEEPER
                                    </span>
                                  ) : (
                                    <button
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        onSetKeeper(group.blake3_hash, copy.primary_path);
                                      }}
                                      title="Click or press Enter to make this file the Protected Keeper"
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-semibold bg-amber-500/10 hover:bg-sky-500/20 text-amber-400 hover:text-sky-300 border border-amber-500/30 hover:border-sky-500/40 transition-colors shrink-0"
                                    >
                                      QUARANTINE
                                    </button>
                                  )}

                                  <div className="min-w-0 flex-1 font-mono text-[11px] text-gray-300 truncate">
                                    <span className="text-gray-400">{getParentDir(copy.primary_path)}/</span>
                                    <span className={`font-semibold ${isInvalidated ? 'text-amber-200 line-through' : 'text-gray-100'}`}>
                                      {getBasename(copy.primary_path)}
                                    </span>
                                  </div>
                                </div>

                                <div className="flex items-center gap-2 shrink-0 pl-2">
                                  <span className="text-[10px] font-mono text-gray-400">
                                    inode: {copy.inode}
                                  </span>

                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      setSelectedFilePath(copy.primary_path);
                                      setQuickLookOpen(true);
                                    }}
                                    title="Quick Look preview (Space)"
                                    className="p-1 rounded hover:bg-subtle text-gray-400 hover:text-sky-300 transition-colors"
                                  >
                                    <Eye className="w-3.5 h-3.5" />
                                  </button>

                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      revealInFileManager(copy.primary_path);
                                    }}
                                    title="Reveal in Finder / Explorer (O)"
                                    className="p-1 rounded hover:bg-subtle text-gray-400 hover:text-white transition-colors"
                                  >
                                    <FolderOpen className="w-3.5 h-3.5" />
                                  </button>
                                </div>
                              </div>

                              {/* Hard-Link Inode Clones (0 B) */}
                              {copy.all_paths.length > 1 && (
                                <div className="pl-6 space-y-1">
                                  {copy.all_paths
                                    .filter((p) => p !== copy.primary_path)
                                    .map((aliasPath) => {
                                      const isAliasInvalidated = invalidatedFiles.has(aliasPath);

                                      return (
                                        <div
                                          key={aliasPath}
                                          onClick={(e) => {
                                            e.stopPropagation();
                                            setSelectedGroupHash(group.blake3_hash);
                                            setSelectedFilePath(aliasPath);
                                          }}
                                          className={`p-1.5 rounded text-[11px] flex items-center justify-between border cursor-pointer transition-colors ${
                                            aliasPath === activeSelectedPath
                                              ? 'bg-elevated border-purple-500/40'
                                              : isAliasInvalidated
                                              ? 'bg-amber-500/10 border-amber-500/30'
                                              : 'bg-subtle/30 border-border-subtle/50 text-gray-400 hover:bg-subtle/50'
                                          }`}
                                        >
                                          <div className="flex items-center gap-2 min-w-0 flex-1">
                                            {isAliasInvalidated ? (
                                              <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-amber-500/20 text-amber-300 border border-amber-500/40 shrink-0">
                                                CHANGED ON DISK
                                              </span>
                                            ) : (
                                              <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 shrink-0">
                                                HARD-LINK ALIAS (0 B)
                                              </span>
                                            )}
                                            <span className={`font-mono text-[10px] truncate ${isAliasInvalidated ? 'text-amber-300 line-through' : 'text-gray-400'}`}>
                                              {aliasPath}
                                            </span>
                                          </div>

                                          <button
                                            onClick={(e) => {
                                              e.stopPropagation();
                                              revealInFileManager(aliasPath);
                                            }}
                                            className="p-1 hover:text-white text-gray-500"
                                            title="Reveal in File Manager"
                                          >
                                            <FolderOpen className="w-3 h-3" />
                                          </button>
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
                );
              })}
            </div>
          )}
        </div>

        {/* RIGHT PANE: Panic Transmit Side-by-Side Comparison Inspector */}
        <div className="w-[440px] bg-surface flex flex-col border-l border-border-subtle overflow-y-auto select-none">
          <div className="p-3.5 border-b border-border-subtle bg-elevated/40 flex items-center justify-between">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-gray-300 flex items-center gap-1.5">
              <Sparkles className="w-3.5 h-3.5 text-sky-400" />
              Side-by-Side Comparison
            </h3>
            <span className="text-[10px] font-mono text-gray-400">
              Panic Transmit Inspector
            </span>
          </div>

          {activeGroup && currentKeeper ? (
            <div className="p-4 space-y-4">
              {/* BLAKE3 Exact Verification Card */}
              <div className="p-3 rounded-lg bg-elevated border border-border-subtle space-y-2">
                <div className="flex items-center justify-between text-xs font-medium text-gray-300">
                  <span className="flex items-center gap-1.5">
                    <Hash className="w-3.5 h-3.5 text-sky-400" />
                    BLAKE3 Cryptographic Hash
                  </span>
                  <button
                    onClick={() => handleCopyHash(activeGroup.blake3_hash)}
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
                <div className="p-2 rounded bg-canvas border border-border-subtle font-mono text-[11px] text-gray-300 break-all leading-relaxed select-all">
                  {activeGroup.blake3_hash}
                </div>
                <div className="flex items-center gap-1.5 text-[10px] text-emerald-400 font-medium">
                  <Check className="w-3 h-3" />
                  <span>100% Exact Bit-for-Bit Duplicate Verified</span>
                </div>
              </div>

              {/* Side-by-Side Comparison: Card 1 - Designated Keeper File */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] font-bold text-sky-400 uppercase tracking-wider flex items-center gap-1">
                    <ShieldCheck className="w-3.5 h-3.5" />
                    Designated Keeper (Surviving File)
                  </span>
                  <span className="text-[10px] font-mono text-sky-400 bg-sky-500/10 px-1.5 py-0.2 rounded border border-sky-500/20">
                    Preserved
                  </span>
                </div>
                <div className="p-3 rounded-lg bg-sky-500/5 border border-sky-500/30 space-y-2.5">
                  <div className="flex items-center gap-2">
                    <div className="p-1 rounded bg-sky-500/10 border border-sky-500/20">
                      {getFileBadgeIcon(currentKeeper.primary_path)}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="text-xs font-semibold text-gray-100 truncate">
                        {getBasename(currentKeeper.primary_path)}
                      </div>
                      <div className="font-mono text-[10px] text-sky-300/80 truncate mt-0.5" title={currentKeeper.primary_path}>
                        {currentKeeper.primary_path}
                      </div>
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-2 text-[11px] pt-2 border-t border-sky-500/15">
                    <div>
                      <span className="text-gray-400 text-[10px]">Size:</span>
                      <div className="font-mono text-gray-200 font-medium">
                        {formatBytes(activeGroup.size)}
                      </div>
                    </div>
                    <div>
                      <span className="text-gray-400 text-[10px]">Physical Inode:</span>
                      <div className="font-mono text-gray-200">
                        {currentKeeper.inode}
                      </div>
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

              {/* Side-by-Side Comparison: Card 2 - Selected Duplicate File */}
              {selectedDuplicateCopy && selectedDuplicateCopy.primary_path !== currentKeeper.primary_path ? (
                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-bold text-amber-400 uppercase tracking-wider flex items-center gap-1">
                      <AlertCircle className="w-3.5 h-3.5" />
                      Candidate Duplicate (Quarantine Target)
                    </span>
                    <span className="text-[10px] font-mono text-amber-400 bg-amber-500/10 px-1.5 py-0.2 rounded border border-amber-500/20">
                      Reclaimable ({formatBytes(activeGroup.size)})
                    </span>
                  </div>

                  {/* Active TOCTOU Defense Warning Badge & Disqualification */}
                  {invalidatedFiles.has(selectedDuplicateCopy.primary_path) ? (
                    <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/40 space-y-2">
                      <div className="flex items-center gap-1.5 text-xs font-bold text-amber-300">
                        <AlertTriangle className="w-4 h-4 text-amber-400" />
                        <span>ACTIVE TOCTOU DEFENSE — CANDIDATE INVALIDATED</span>
                      </div>
                      <p className="text-[11px] text-amber-200/90 leading-relaxed font-mono">
                        File was modified, renamed, or deleted on disk after scanning completed.
                        Reclaim has automatically disqualified this file from cleanup to guarantee zero data loss.
                      </p>
                      <div className="text-[10px] font-mono text-amber-400/80 bg-amber-500/10 p-1.5 rounded border border-amber-500/20">
                        Event: {invalidatedFiles.get(selectedDuplicateCopy.primary_path)?.reason || 'Modified on disk'}
                      </div>
                    </div>
                  ) : (
                    <div className="p-3 rounded-lg bg-amber-500/5 border border-amber-500/30 space-y-2.5">
                      <div className="flex items-center gap-2">
                        <div className="p-1 rounded bg-amber-500/10 border border-amber-500/20">
                          {getFileBadgeIcon(selectedDuplicateCopy.primary_path)}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="text-xs font-semibold text-gray-100 truncate">
                            {getBasename(selectedDuplicateCopy.primary_path)}
                          </div>
                          <div className="font-mono text-[10px] text-amber-300/80 truncate mt-0.5" title={selectedDuplicateCopy.primary_path}>
                            {selectedDuplicateCopy.primary_path}
                          </div>
                        </div>
                      </div>

                      <div className="grid grid-cols-2 gap-2 text-[11px] pt-2 border-t border-amber-500/15">
                        <div>
                          <span className="text-gray-400 text-[10px]">Size:</span>
                          <div className="font-mono text-gray-200 font-medium">
                            {formatBytes(activeGroup.size)}
                          </div>
                        </div>
                        <div>
                          <span className="text-gray-400 text-[10px]">Physical Inode:</span>
                          <div className="font-mono text-gray-200">
                            {selectedDuplicateCopy.inode}
                            {selectedDuplicateCopy.inode === currentKeeper.inode ? (
                              <span className="ml-1 text-[9px] text-purple-400">(Same Hard Link)</span>
                            ) : (
                              <span className="ml-1 text-[9px] text-amber-400">(Distinct)</span>
                            )}
                          </div>
                        </div>
                      </div>

                      {/* Prospective Quarantine Path Preview */}
                      <div className="p-2 rounded bg-canvas border border-border-subtle text-[10px] font-mono text-gray-400 space-y-1">
                        <div className="text-gray-400">Proposed Quarantine Destination:</div>
                        <div className="text-amber-400 truncate">
                          .../.reclaim/quarantine/{truncateHash(activeGroup.blake3_hash, 4)}/{getBasename(selectedDuplicateCopy.primary_path)}
                        </div>
                      </div>

                      {/* Interactive Standard Actions */}
                      <div className="flex gap-2">
                        <button
                          onClick={() => onSetKeeper(activeGroup.blake3_hash, selectedDuplicateCopy.primary_path)}
                          className="flex-1 py-1.5 px-2 rounded bg-sky-500/15 hover:bg-sky-500/25 text-sky-300 text-xs font-semibold border border-sky-500/40 hover:border-sky-500/60 transition-colors flex items-center justify-center gap-1.5 shadow-sm"
                        >
                          <ShieldCheck className="w-3.5 h-3.5" />
                          <span>Promote to Keeper (Enter)</span>
                        </button>
                        <button
                          onClick={() => revealInFileManager(selectedDuplicateCopy.primary_path)}
                          className="p-1.5 rounded bg-subtle hover:bg-elevated text-gray-400 hover:text-white border border-border-subtle transition-colors"
                          title="Reveal duplicate in file manager (O)"
                        >
                          <FolderOpen className="w-3.5 h-3.5" />
                        </button>
                      </div>

                      {/* PRIMARY / ALTERNATE ACTION: Zero-Movement CoW Extent Deduplication */}
                      {supportsCow && (
                        <div className="space-y-2 pt-2 border-t border-border-subtle/50">
                          <div className="flex items-center justify-between text-[11px]">
                            <span className="font-semibold text-emerald-400 flex items-center gap-1">
                              <Layers className="w-3.5 h-3.5" />
                              Zero-Movement CoW Extent Cloning
                            </span>
                            <span className="text-[9px] font-mono px-1.5 py-0.2 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/30">
                              APFS / Btrfs
                            </span>
                          </div>
                          <button
                            onClick={() =>
                              handleSingleCowDeduplicate(
                                currentKeeper.primary_path,
                                selectedDuplicateCopy.primary_path
                              )
                            }
                            disabled={
                              isSingleCowRunning ||
                              cowDeduplicatedPaths.has(selectedDuplicateCopy.primary_path)
                            }
                            className="w-full py-2 px-3 rounded-lg bg-emerald-500/15 hover:bg-emerald-500/25 disabled:bg-subtle disabled:text-gray-500 text-emerald-300 text-xs font-semibold border border-emerald-500/40 hover:border-emerald-500/60 transition-all flex items-center justify-center gap-2 shadow-sm"
                          >
                            <Layers className="w-3.5 h-3.5 text-emerald-400" />
                            <span>
                              {cowDeduplicatedPaths.has(selectedDuplicateCopy.primary_path)
                                ? 'Extents Already Cloned (0 B Physical)'
                                : isSingleCowRunning
                                ? 'Cloning extents via APFS clonefile(2)...'
                                : 'Deduplicate via CoW Extents (Zero Movement)'}
                            </span>
                          </button>

                          {singleCowResult &&
                            singleCowResult.path === selectedDuplicateCopy.primary_path && (
                              <div className="p-2.5 rounded-lg bg-emerald-500/10 border border-emerald-500/30 text-[11px] font-mono text-emerald-300 space-y-1">
                                <div className="flex items-center gap-1 font-semibold text-emerald-400">
                                  <CheckCircle2 className="w-3.5 h-3.5" /> Extent Deduplication
                                  Successful
                                </div>
                                <div>
                                  Reclaimed: {formatBytes(singleCowResult.bytes)} physical storage
                                </div>
                                <div className="text-[10px] text-emerald-400/80">
                                  Strategy: {singleCowResult.strategy}
                                </div>
                              </div>
                            )}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              ) : (
                <div className="p-4 rounded-lg bg-elevated/60 border border-border-subtle text-center text-xs text-gray-400 space-y-1">
                  <p className="font-medium text-gray-300">Designated Keeper Selected</p>
                  <p className="text-[11px] text-gray-400">
                    Select any child duplicate copy in the table to compare side-by-side or perform CoW extent deduplication.
                  </p>
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

      {/* 4. Quick Look Floating Preview (Spacebar overlay) */}
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

      {/* 5. CoW Extent Deduplication Batch Modal */}
      {cowModalOpen && (
        <CowDeduplicationModal
          isOpen={cowModalOpen}
          onClose={() => setCowModalOpen(false)}
          groups={filteredGroups}
          keeperOverrides={keeperOverrides}
          onComplete={handleBatchCowComplete}
        />
      )}
    </div>
  );
};
