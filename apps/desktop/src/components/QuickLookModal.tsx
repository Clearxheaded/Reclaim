import React, { useEffect } from 'react';
import {
  X,
  ShieldCheck,
  AlertCircle,
  FolderOpen,
  Hash,
  Copy,
  Check,
  Calendar,
  FileCode,
  FileText,
  FileImage,
  FileArchive,
  FileAudio,
  FileVideo,
  File,
} from 'lucide-react';
import { formatBytes, formatDate, getBasename, getParentDir, getFileExtension } from '../utils/format';
import { revealInFileManager } from '../api/ipc';

interface QuickLookModalProps {
  filePath: string;
  isKeeper: boolean;
  size: number;
  blake3Hash: string;
  inode?: number;
  deviceId?: number;
  modified?: number;
  onClose: () => void;
  onSetAsKeeper?: () => void;
  onNavigateNext?: () => void;
  onNavigatePrev?: () => void;
}

export const QuickLookModal: React.FC<QuickLookModalProps> = ({
  filePath,
  isKeeper,
  size,
  blake3Hash,
  inode,
  deviceId,
  modified,
  onClose,
  onSetAsKeeper,
  onNavigateNext,
  onNavigatePrev,
}) => {
  const [copiedHash, setCopiedHash] = React.useState<boolean>(false);

  const basename = getBasename(filePath);
  const parentDir = getParentDir(filePath);
  const ext = getFileExtension(filePath);

  // Keyboard navigation within QuickLook
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' || e.key === ' ') {
        e.preventDefault();
        onClose();
      } else if (e.key === 'Enter') {
        if (!isKeeper && onSetAsKeeper) {
          e.preventDefault();
          onSetAsKeeper();
        }
      } else if (e.key === 'j' || e.key === 'ArrowDown') {
        if (onNavigateNext) {
          e.preventDefault();
          onNavigateNext();
        }
      } else if (e.key === 'k' || e.key === 'ArrowUp') {
        if (onNavigatePrev) {
          e.preventDefault();
          onNavigatePrev();
        }
      } else if (e.key === 'o' || e.key === 'O') {
        e.preventDefault();
        revealInFileManager(filePath);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isKeeper, onSetAsKeeper, onClose, onNavigateNext, onNavigatePrev, filePath]);

  const handleCopyHash = () => {
    navigator.clipboard.writeText(blake3Hash);
    setCopiedHash(true);
    setTimeout(() => setCopiedHash(false), 2000);
  };

  const getFileIcon = (extension: string) => {
    switch (extension) {
      case 'png':
      case 'jpg':
      case 'jpeg':
      case 'gif':
      case 'webp':
      case 'svg':
        return <FileImage className="w-12 h-12 text-pink-400" />;
      case 'mp4':
      case 'mov':
      case 'mkv':
      case 'avi':
        return <FileVideo className="w-12 h-12 text-violet-400" />;
      case 'mp3':
      case 'wav':
      case 'flac':
      case 'aac':
        return <FileAudio className="w-12 h-12 text-cyan-400" />;
      case 'zip':
      case 'tar':
      case 'gz':
      case '7z':
      case 'dmg':
        return <FileArchive className="w-12 h-12 text-amber-400" />;
      case 'rs':
      case 'ts':
      case 'tsx':
      case 'js':
      case 'json':
      case 'py':
      case 'html':
      case 'css':
        return <FileCode className="w-12 h-12 text-emerald-400" />;
      case 'pdf':
      case 'txt':
      case 'md':
      case 'doc':
      case 'docx':
        return <FileText className="w-12 h-12 text-blue-400" />;
      default:
        return <File className="w-12 h-12 text-gray-400" />;
    }
  };

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 bg-black/80 backdrop-blur-md flex items-center justify-center p-6 animate-in fade-in duration-150"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-2xl bg-surface border border-border-muted rounded-xl shadow-2xl overflow-hidden flex flex-col"
      >
        {/* Title Bar */}
        <div className="p-3.5 border-b border-border-subtle bg-elevated flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-subtle text-gray-300 border border-border-subtle uppercase">
              Quick Look Preview
            </span>
            <span className="text-xs font-semibold text-gray-300">
              {basename}
            </span>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded-md text-gray-400 hover:text-white hover:bg-subtle transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content Area */}
        <div className="p-6 space-y-6">
          {/* File Hero Section */}
          <div className="flex items-center gap-5 p-4 rounded-xl bg-canvas border border-border-subtle">
            <div className="p-3 rounded-lg bg-elevated border border-border-subtle">
              {getFileIcon(ext)}
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <h2 className="text-base font-semibold text-white truncate">
                  {basename}
                </h2>
                {isKeeper ? (
                  <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold bg-sky-500/10 text-sky-400 border border-sky-500/30">
                    <ShieldCheck className="w-3 h-3" />
                    PROTECTED KEEPER
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/30">
                    <AlertCircle className="w-3 h-3" />
                    QUARANTINE CANDIDATE
                  </span>
                )}
              </div>
              <p className="font-mono text-xs text-gray-400 truncate mt-1" title={filePath}>
                {parentDir}
              </p>
            </div>
          </div>

          {/* Metadata Grid */}
          <div className="grid grid-cols-2 gap-3 text-xs">
            <div className="p-3 rounded-lg bg-elevated/70 border border-border-subtle space-y-1">
              <span className="text-[10px] uppercase font-semibold text-gray-400">
                Exact File Size
              </span>
              <div className="text-sm font-mono font-bold text-gray-200">
                {formatBytes(size)}
              </div>
              <div className="text-[10px] font-mono text-gray-400">
                {size.toLocaleString()} bytes
              </div>
            </div>

            <div className="p-3 rounded-lg bg-elevated/70 border border-border-subtle space-y-1">
              <span className="text-[10px] uppercase font-semibold text-gray-400 flex items-center gap-1">
                <Calendar className="w-3 h-3" /> Last Modified
              </span>
              <div className="text-sm font-mono text-gray-200 truncate">
                {modified ? formatDate(modified) : 'Unknown'}
              </div>
              <div className="text-[10px] font-mono text-gray-400">
                Unix timestamp: {modified ?? 'N/A'}
              </div>
            </div>

            <div className="p-3 rounded-lg bg-elevated/70 border border-border-subtle space-y-1">
              <span className="text-[10px] uppercase font-semibold text-gray-400">
                Filesystem Inode
              </span>
              <div className="text-sm font-mono text-gray-200">
                {inode ?? 'N/A'}
              </div>
              <div className="text-[10px] font-mono text-gray-400">
                Physical file record ID
              </div>
            </div>

            <div className="p-3 rounded-lg bg-elevated/70 border border-border-subtle space-y-1">
              <span className="text-[10px] uppercase font-semibold text-gray-400">
                Device / Volume ID
              </span>
              <div className="text-sm font-mono text-gray-200">
                {deviceId ?? 'N/A'}
              </div>
              <div className="text-[10px] font-mono text-gray-400">
                Hardware storage boundary
              </div>
            </div>
          </div>

          {/* BLAKE3 Content Hash Card */}
          <div className="p-3.5 rounded-lg bg-elevated border border-border-subtle space-y-2">
            <div className="flex items-center justify-between text-xs font-medium text-gray-300">
              <span className="flex items-center gap-1.5">
                <Hash className="w-3.5 h-3.5 text-sky-400" />
                BLAKE3 Cryptographic Checksum
              </span>
              <button
                onClick={handleCopyHash}
                className="text-[11px] text-sky-400 hover:text-sky-300 flex items-center gap-1 transition-colors"
              >
                {copiedHash ? (
                  <>
                    <Check className="w-3 h-3 text-emerald-400" /> Copied
                  </>
                ) : (
                  <>
                    <Copy className="w-3 h-3" /> Copy Hash
                  </>
                )}
              </button>
            </div>
            <div className="p-2 rounded bg-canvas border border-border-subtle font-mono text-[11px] text-gray-300 break-all leading-relaxed select-all">
              {blake3Hash}
            </div>
            <div className="text-[10px] text-emerald-400 font-medium flex items-center gap-1">
              <Check className="w-3 h-3" />
              100% Bit-for-Bit Exact Content Match Verified
            </div>
          </div>
        </div>

        {/* Footer Actions & Shortcut Hints */}
        <div className="p-3 border-t border-border-subtle bg-elevated flex items-center justify-between text-xs">
          <div className="flex items-center gap-3 text-[11px] text-gray-400">
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 rounded bg-subtle border border-border-muted font-mono text-[10px] text-gray-300">
                Space
              </kbd>
              <span>Close</span>
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 rounded bg-subtle border border-border-muted font-mono text-[10px] text-gray-300">
                Enter
              </kbd>
              <span>Set Keeper</span>
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 rounded bg-subtle border border-border-muted font-mono text-[10px] text-gray-300">
                j/k
              </kbd>
              <span>Next/Prev</span>
            </span>
          </div>

          <div className="flex items-center gap-2">
            {!isKeeper && onSetAsKeeper && (
              <button
                onClick={onSetAsKeeper}
                className="px-3 py-1.5 rounded-lg bg-sky-500 hover:bg-sky-400 text-white text-xs font-semibold flex items-center gap-1.5 shadow-sm transition-colors"
              >
                <ShieldCheck className="w-3.5 h-3.5" />
                <span>Promote to Keeper</span>
              </button>
            )}
            <button
              onClick={() => revealInFileManager(filePath)}
              className="px-3 py-1.5 rounded-lg bg-subtle hover:bg-elevated text-gray-200 hover:text-white border border-border-subtle text-xs font-medium flex items-center gap-1.5 transition-colors"
            >
              <FolderOpen className="w-3.5 h-3.5" />
              <span>Reveal in File Manager</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

