import React from 'react';
import { AlertOctagon, ShieldAlert, X } from 'lucide-react';
import { UndoConflictDetail } from '../types/reclaim';
import { revealInFileManager } from '../api/ipc';

interface ConflictModalProps {
  conflicts: UndoConflictDetail[];
  onClose: () => void;
}

export const ConflictModal: React.FC<ConflictModalProps> = ({ conflicts, onClose }) => {
  return (
    <div className="fixed inset-0 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4 z-50 select-none">
      <div className="bg-surface border border-rose-500/30 rounded-lg max-w-xl w-full shadow-2xl flex flex-col max-h-[80vh] overflow-hidden">
        {/* Header */}
        <div className="p-3.5 border-b border-border-subtle flex items-center justify-between bg-rose-500/10">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded bg-rose-500/20 text-rose-400 flex items-center justify-center">
              <AlertOctagon className="w-4 h-4" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-white">Undo Refused: Destination Conflict</h3>
              <p className="text-[10px] text-gray-400">Reclaim will never silently overwrite user files</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-gray-400 hover:text-white hover:bg-elevated transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Conflict Explanations */}
        <div className="p-4 space-y-3.5 overflow-y-auto text-xs">
          <div className="p-3 rounded bg-canvas border border-white/5 text-gray-300 space-y-1.5">
            <div className="font-semibold text-rose-400 flex items-center gap-1.5">
              <ShieldAlert className="w-3.5 h-3.5" />
              <span>Core Safety Invariant: No Silent Overwrites</span>
            </div>
            <p className="text-[11px] text-gray-400 leading-relaxed">
              During restoration, Reclaim verified the original duplicate destination path. Another
              file now occupies that location. To prevent catastrophic data loss, Reclaim refused to
              overwrite it. Your quarantined file remains safely preserved.
            </p>
          </div>

          <div className="space-y-1.5">
            <span className="text-xs font-semibold text-gray-300">
              Conflicted Actions ({conflicts.length}):
            </span>
            <div className="border border-white/5 rounded divide-y divide-white/5 bg-canvas/40">
              {conflicts.map((c) => (
                <div key={c.action_id} className="p-2.5 space-y-1.5 font-mono text-[11px] tabular-nums">
                  <div>
                    <span className="text-rose-400 font-semibold block mb-0.5">
                      Reason: {c.reason}
                    </span>
                    <span className="text-gray-300 break-all">{c.message}</span>
                  </div>

                  <div className="p-2 rounded bg-elevated/70 border border-white/5 text-[10px] space-y-0.5">
                    <div className="text-gray-500">Destination Path (Occupied):</div>
                    <div className="text-gray-200 break-all">{c.source_path}</div>
                    <div className="text-gray-500 mt-1">Preserved in Quarantine:</div>
                    <div className="text-amber-400 break-all">{c.quarantine_path}</div>
                  </div>

                  <div className="flex justify-end gap-2 pt-0.5">
                    <button
                      onClick={() => revealInFileManager(c.quarantine_path)}
                      className="px-2 py-1 rounded bg-elevated hover:bg-subtle text-gray-300 text-[10px] transition-colors border border-white/5"
                    >
                      Inspect in Quarantine
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="p-3 border-t border-border-subtle bg-elevated/40 flex justify-end">
          <button
            onClick={onClose}
            className="px-3.5 py-1.5 rounded bg-elevated hover:bg-subtle text-white text-xs font-medium transition-colors border border-white/10"
          >
            Acknowledge & Close
          </button>
        </div>
      </div>
    </div>
  );
};
