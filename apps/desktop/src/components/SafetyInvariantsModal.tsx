import React from 'react';
import {
  ShieldCheck,
  Lock,
  FileCheck,
  AlertTriangle,
  History,
  HardDrive,
  Database,
  XCircle,
  FileCode,
} from 'lucide-react';

export const SafetyInvariantsModal: React.FC = () => {
  const invariants = [
    {
      num: 1,
      title: 'No Mutation Without Approval',
      desc: 'Scanning, duplicate detection, and plan preview are strictly read-only. Zero filesystem mutations occur without explicit, deliberate user approval.',
      icon: Lock,
    },
    {
      num: 2,
      title: 'No Silent Overwrites',
      desc: 'Reclaim uses kernel-level atomic non-replacing syscalls (renameat2 on macOS / MoveFileExW on Windows). If a destination exists, Reclaim fails closed rather than overwriting.',
      icon: ShieldCheck,
    },
    {
      num: 3,
      title: 'No Unexpected Path Traversal',
      desc: 'Never traverses or follows symbolic links, junctions, or reparse points into unapproved filesystem locations.',
      icon: XCircle,
    },
    {
      num: 4,
      title: 'No Trust in Stale Metadata (TOCTOU Defense)',
      desc: 'Files may change between scan time and execution. Before moving any file, Reclaim revalidates regular file status, file sizes, and cryptographic BLAKE3 hashes.',
      icon: AlertTriangle,
    },
    {
      num: 5,
      title: 'Reversible Quarantine Over Permanent Deletion',
      desc: 'Reclaim moves duplicate candidates into a local .reclaim/quarantine folder on the same disk rather than executing permanent deletion.',
      icon: HardDrive,
    },
    {
      num: 6,
      title: 'Preserve Verified Content & Inode Awareness',
      desc: 'Never quarantines a candidate unless its surviving keeper file is physically verified on disk. Hard links are recognized to prevent misleading disk space claims.',
      icon: FileCheck,
    },
    {
      num: 7,
      title: 'Honest Conditional Undo',
      desc: 'Undo validates destination vacancy before restoration. If another file occupies the original path, Reclaim reports an explicit conflict and preserves both files.',
      icon: History,
    },
    {
      num: 8,
      title: 'Crash Recovery via Dual Reconciliation',
      desc: 'After power loss or unexpected kill, Reclaim inspects both the SQLite journal and physical disk state at source and quarantine before determining action outcomes.',
      icon: Database,
    },
    {
      num: 9,
      title: 'Honest Durability Claims',
      desc: 'SQLite transactions guarantee relational audit integrity, but do not make filesystem renames atomic. Reclaim accounts for crash windows transparently.',
      icon: FileCode,
    },
    {
      num: 10,
      title: 'Fail Closed Architecture',
      desc: 'If any filesystem capability is unsupported, permissions are missing, or device boundaries are crossed, Reclaim refuses the operation safely.',
      icon: Lock,
    },
  ];

  return (
    <div className="flex-1 flex flex-col p-6 overflow-y-auto bg-canvas select-none">
      <div className="max-w-4xl w-full mx-auto space-y-5">
        <div>
          <div className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 text-xs font-medium mb-2">
            <ShieldCheck className="w-3.5 h-3.5" />
            <span>Core Engineering Standard</span>
          </div>
          <h2 className="text-xl font-bold text-white tracking-tight">
            Filesystem Safety Invariants
          </h2>
          <p className="text-xs text-gray-400 mt-1">
            Reclaim prioritizes filesystem safety above raw performance or convenience.
            These 10 invariants govern every layer of the Rust core and desktop interface.
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {invariants.map((inv) => {
            const Icon = inv.icon;
            return (
              <div
                key={inv.num}
                className="p-3.5 rounded bg-surface border border-white/5 hover:border-white/10 transition-colors space-y-1.5"
              >
                <div className="flex items-center gap-2.5">
                  <div className="w-6 h-6 rounded bg-elevated border border-white/5 flex items-center justify-center text-emerald-400 shrink-0">
                    <Icon className="w-3.5 h-3.5" />
                  </div>
                  <div className="text-xs font-semibold text-gray-200">
                    <span className="text-emerald-400 font-mono tabular-nums mr-1.5">#{inv.num}</span>
                    {inv.title}
                  </div>
                </div>
                <p className="text-[11px] text-gray-400 leading-relaxed pl-8">
                  {inv.desc}
                </p>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
};
