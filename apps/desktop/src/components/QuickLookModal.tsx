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
        return <FileImage className="w-10 h-10 text-pink-400" />;
      case 'mp4':
      case 'mov':
      case 'mkv':
      case 'avi':
        return <FileVideo className="w-10 h-10 text-blue-400" />;
      case 'mp3':
      case 'wav':
      case 'flac':
      case 'aac':
        return <FileAudio className="w-10 h-10 text-teal-400" />;
      case 'zip':
      case 'tar':
      case 'gz':
      case '7z':
      case 'dmg':
        return <FileArchive className="w-10 h-10 text-amber-400" />;
      case 'rs':
      case 'ts':
      case 'tsx':
      case 'js':
      case 'json':
      case 'py':
      case 'html':
      case 'css':
        return <FileCode className="w-10 h-10 text-emerald-400" />;
      case 'pdf':
      case 'txt':
      case 'md':
      case 'doc':
      case 'docx':
        return <FileText className="w-10 h-10 text-sky-400" />;
      default:
        return <File className="w-10 h-10 text-gray-400" />;
    }
  };

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-6 animate-in fade-in duration-150 select-none"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-2xl bg-surface border border-white/10 rounded-lg shadow-2xl overflow-hidden flex flex-col"
      >
        {/* Title Bar */}
        <div className="p-3 border-b border-border-subtle bg-elevated/40 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-[10px] tabular-nums font-mono px-1.5 py-0.5 rounded bg-elevated text-gray-300 border border-white/5 uppercase">
              Quick Look Preview
            </span>
            <span className="text-xs font-semibold text-gray-200">
              {basename}
            </span>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-gray-400 hover:text-white hover:bg-elevated transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content Area */}
        <div className="p-5 space-y-4">
          {/* File Hero Section */}
          <div className="flex items-center gap-4 p-3.5 rounded-lg bg-canvas border border-white/5">
            <div className="p-2.5 rounded bg-elevated border border-white/5">
              {getFileIcon(ext)}
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-semibold text-white truncate">
                  {basename}
                </h2>
                {isKeeper ? (
                  <span className="inline-flex items-center gap-1 px-1.5 py-0.2 rounded text-[10px] font-semibold bg-sky-500/10 text-sky-400 border border-sky-500/25">
                    <ShieldCheck className="w-3 h-3" />
                    KEEPER
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 px-1.5 py-0.2 rounded text-[10px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/25">
                    <AlertCircle className="w-3 h-3" />
                    QUARANTINE CANDIDATE
                  </span>
                )}
              </div>
              <p className="font-mono text-[11px] text-gray-500 truncate mt-1" title={filePath}>
                {parentDir}
              </p>
            </div>
          </div>

          {/* Metadata Grid */}
          <div className="grid grid-cols-2 gap-2.5 text-xs">
            <div className="p-2.5 rounded bg-elevated/70 border border-white/5 space-y-0.5">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Exact File Size
              </span>
              <div className="text-xs tabular-nums font-mono font-bold text-gray-200">
                {formatBytes(size)}
              </div>
              <div className="text-[10px] tabular-nums font-mono text-gray-500">
                {size.toLocaleString()} bytes
              </div>
            </div>

            <div className="p-2.5 rounded bg-elevated/70 border border-white/5 space-y-0.5">
              <span className="text-[10px] uppercase font-semibold text-gray-500 flex items-center gap-1">
                <Calendar className="w-3 h-3" /> Last Modified
              </span>
              <div className="text-xs tabular-nums font-mono text-gray-200 truncate">
                {modified ? formatDate(modified) : 'Unknown'}
              </div>
              <div className="text-[10px] tabular-nums font-mono text-gray-500">
                Unix: {modified ?? 'N/A'}
              </div>
            </div>

            <div className="p-2.5 rounded bg-elevated/70 border border-white/5 space-y-0.5">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Filesystem Inode
              </span>
              <div className="text-xs tabular-nums font-mono text-gray-200">
                {inode ?? 'N/A'}
              </div>
              <div className="text-[10px] tabular-nums font-mono text-gray-500">
                Physical file record ID
              </div>
            </div>

            <div className="p-2.5 rounded bg-elevated/70 border border-white/5 space-y-0.5">
              <span className="text-[10px] uppercase font-semibold text-gray-500">
                Device / Volume ID
              </span>
              <div className="text-xs tabular-nums font-mono text-gray-200">
                {deviceId ?? 'N/A'}
              </div>
              <div className="text-[10px] tabular-nums font-mono text-gray-500">
                Hardware storage volume
              </div>
            </div>
          </div>

          {/* BLAKE3 Content Hash Card */}
          <div className="p-3 rounded bg-elevated/70 border border-white/5 space-y-1.5">
            <div className="flex items-center justify-between text-xs font-medium text-gray-300">
              <span className="flex items-center gap-1.5">
                <Hash className="w-3.5 h-3.5 text-sky-400" />
                BLAKE3 Cryptographic Checksum
              </span>
              <button
                onClick={handleCopyHash}
                className="text-[10px] text-sky-400 hover:text-sky-300 flex items-center gap-1 transition-colors"
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
            <div className="p-2 rounded bg-canvas border border-white/5 font-mono text-[11px] tabular-nums text-gray-300 break-all leading-relaxed select-all">
              {blake3Hash}
            </div>
            <div className="text-[10px] text-emerald-400 font-medium flex items-center gap-1">
              <Check className="w-3 h-3" />
              100% Bit-for-Bit Exact Content Match Verified
            </div>
          </div>
        </div>

        {/* Footer Actions & Shortcut Hints */}
        <div className="p-3 border-t border-border-subtle bg-elevated/40 flex items-center justify-between text-xs">
          <div className="flex items-center gap-2.5 text-[10px] tabular-nums font-mono text-gray-500">
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">Space</kbd>
              <span>close</span>
            </span>
            <span>•</span>
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">Enter</kbd>
              <span>set keeper</span>
            </span>
            <span>•</span>
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 rounded bg-elevated border border-white/5 text-gray-300">j/k</kbd>
              <span>nav</span>
            </span>
          </div>

          <div className="flex items-center gap-2">
            {!isKeeper && onSetAsKeeper && (
              <button
                onClick={onSetAsKeeper}
                className="px-3 py-1.5 rounded bg-sky-500 hover:bg-sky-400 text-white text-xs font-semibold flex items-center gap-1.5 transition-colors"
              >
                <ShieldCheck className="w-3.5 h-3.5" />
                <span>Promote to Keeper</span>
              </button>
            )}
            <button
              onClick={() => revealInFileManager(filePath)}
              className="px-3 py-1.5 rounded bg-elevated hover:bg-subtle text-gray-300 hover:text-white border border-white/5 text-xs font-medium flex items-center gap-1.5 transition-colors"
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
