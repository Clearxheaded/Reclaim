import { Component, ErrorInfo, ReactNode } from 'react';
import { AlertOctagon, RotateCcw } from 'lucide-react';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
    };
  }

  static getDerivedStateFromError(error: Error): State {
    return {
      hasError: true,
      error,
      errorInfo: null,
    };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error('Unhandled UI exception in Reclaim desktop:', error, errorInfo);
    this.setState({ errorInfo });
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null, errorInfo: null });
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-screen w-screen items-center justify-center bg-canvas text-gray-100 p-8 select-none">
          <div className="max-w-lg w-full bg-surface border border-rose-500/30 rounded-lg p-6 space-y-4 shadow-2xl">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded bg-rose-500/10 border border-rose-500/20 flex items-center justify-center text-rose-400 shrink-0">
                <AlertOctagon className="w-5 h-5" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-white">Rendering Error Encountered</h2>
                <p className="text-xs text-zinc-400">
                  The interface intercepted an unexpected UI error without crashing.
                </p>
              </div>
            </div>

            {this.state.error && (
              <div className="bg-canvas border border-white/5 rounded p-3 font-mono text-xs text-rose-300 break-words whitespace-pre-wrap max-h-48 overflow-y-auto">
                {this.state.error.message || String(this.state.error)}
              </div>
            )}

            <div className="flex justify-end pt-2">
              <button
                onClick={this.handleReset}
                className="px-3.5 py-1.5 rounded bg-sky-500 hover:bg-sky-400 text-white text-xs font-semibold flex items-center gap-2 transition-colors"
              >
                <RotateCcw className="w-3.5 h-3.5" />
                <span>Reload Application</span>
              </button>
            </div>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
