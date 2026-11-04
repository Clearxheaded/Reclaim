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
    <div className="fixed inset-0 bg-black/75 backdrop-blur-sm flex items-center justify-center p-4 z-50">
      <div className="bg-surface border border-red-500/30 rounded-xl max-w-xl w-full shadow-2xl flex flex-col max-h-[80vh] overflow-hidden">
        {/* Header */}
        <div className="p-4 border-b border-border-subtle flex items-center justify-between bg-red-500/10">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-red-500/20 text-red-400 flex items-center justify-center">
              <AlertOctagon className="w-5 h-5" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-white">Undo Refused: Destination Conflict</h3>
              <p className="text-[11px] text-gray-400">Reclaim will never silently overwrite user files</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-gray-400 hover:text-white hover:bg-subtle"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Conflict Explanations */}
        <div className="p-5 space-y-4 overflow-y-auto text-xs">
          <div className="p-3 rounded-lg bg-canvas border border-border-subtle text-gray-300 space-y-2">
            <div className="font-semibold text-red-400 flex items-center gap-1.5">
              <ShieldAlert className="w-4 h-4" />
              <span>Core Safety Invariant: No Silent Overwrites</span>
            </div>
            <p className="text-[11px] text-gray-400 leading-relaxed">
              During restoration, Reclaim verified the original duplicate destination path. Another
              file now occupies that location. To prevent catastrophic data loss, Reclaim refused to
              overwrite it. Your quarantined file remains safely preserved.
            </p>
          </div>

          <div className="space-y-2">
            <span className="text-xs font-semibold text-gray-300">
              Conflicted Actions ({conflicts.length}):
            </span>
            <div className="border border-border-subtle rounded-lg divide-y divide-border-subtle bg-canvas/40">
              {conflicts.map((c) => (
                <div key={c.action_id} className="p-3 space-y-2 font-mono text-[11px]">
                  <div>
                    <span className="text-red-400 font-semibold block mb-0.5">
                      Reason: {c.reason}
                    </span>
                    <span className="text-gray-300 break-all">{c.message}</span>
                  </div>

                  <div className="p-2 rounded bg-elevated border border-border-subtle text-[10px] space-y-1">
                    <div className="text-gray-500">Destination Path (Occupied):</div>
                    <div className="text-gray-200 break-all">{c.source_path}</div>
                    <div className="text-gray-500 mt-1">Preserved in Quarantine:</div>
                    <div className="text-amber-400 break-all">{c.quarantine_path}</div>
                  </div>

                  <div className="flex justify-end gap-2 pt-1">
                    <button
                      onClick={() => revealInFileManager(c.quarantine_path)}
                      className="px-2 py-1 rounded bg-subtle hover:bg-elevated text-gray-300 text-[10px] transition-colors"
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
        <div className="p-4 border-t border-border-subtle bg-canvas/40 flex justify-end">
          <button
            onClick={onClose}
            className="px-4 py-2 rounded-lg bg-subtle hover:bg-elevated text-white text-xs font-medium transition-colors"
          >
            Acknowledge & Close
          </button>
        </div>
      </div>
    </div>
  );
};

