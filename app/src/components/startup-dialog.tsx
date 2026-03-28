import { useEffect, useRef } from "react";
import { useAppStore } from "../stores/app-store";
import { discoverP4k, openP4k, browseP4k, onLoadProgress } from "../lib/commands";

export function StartupScreen() {
  const loading = useAppStore((s) => s.loading);
  const loadingProgress = useAppStore((s) => s.loadingProgress);
  const loadingMessage = useAppStore((s) => s.loadingMessage);
  const error = useAppStore((s) => s.error);
  const discoveries = useAppStore((s) => s.discoveries);
  const setDiscoveries = useAppStore((s) => s.setDiscoveries);
  const setLoading = useAppStore((s) => s.setLoading);
  const setProgress = useAppStore((s) => s.setProgress);
  const setLoaded = useAppStore((s) => s.setLoaded);
  const setError = useAppStore((s) => s.setError);
  const clearError = useAppStore((s) => s.clearError);

  const hasDiscovered = useRef(false);

  useEffect(() => {
    if (hasDiscovered.current) return;
    hasDiscovered.current = true;
    discoverP4k().then(setDiscoveries);
  }, [setDiscoveries]);

  const handleLoad = async (path: string, source: string) => {
    clearError();
    setLoading(true);
    setProgress(0, "Opening P4k...");

    const unlisten = await onLoadProgress((progress) => {
      setProgress(progress.fraction, progress.message);
    });

    try {
      const count = await openP4k(path);
      setLoaded(path, source, count);
    } catch (err) {
      setError(String(err));
    } finally {
      unlisten();
    }
  };

  const handleBrowse = async () => {
    clearError();
    const path = await browseP4k();
    if (path) {
      await handleLoad(path, "custom");
    }
  };

  return (
    <div className="flex-1 flex items-center justify-center bg-bg">
      <div className="w-[480px]">
        {/* Header */}
        <div className="text-center mb-8">
          <h1 className="text-2xl font-bold text-primary mb-2">StarBreaker</h1>
          <p className="text-sm text-text-dim">Star Citizen Data Explorer</p>
        </div>

        {/* Error message */}
        {error && (
          <div className="mb-4 p-3 bg-danger/10 border border-danger/30 rounded-lg">
            <p className="text-sm text-danger">{error}</p>
          </div>
        )}

        {/* Loading state */}
        {loading && (
          <div className="bg-bg-alt border border-border rounded-lg p-6 mb-4">
            <div className="flex justify-between text-xs text-text-dim mb-2">
              <span>{loadingMessage || "Loading..."}</span>
              <span>{Math.round(loadingProgress * 100)}%</span>
            </div>
            <div className="h-2 bg-surface rounded-full overflow-hidden">
              <div
                className="h-full bg-primary rounded-full transition-all duration-150"
                style={{ width: `${loadingProgress * 100}%` }}
              />
            </div>
          </div>
        )}

        {/* Channel buttons */}
        {!loading && (
          <div className="flex flex-col gap-3">
            {discoveries.length > 0 && (
              <>
                <p className="text-xs text-text-dim uppercase tracking-wider font-semibold">
                  Detected installations
                </p>
                {discoveries.map((d) => (
                  <button
                    key={d.source}
                    onClick={() => handleLoad(d.path, d.source)}
                    className="flex items-center gap-4 p-4 bg-bg-alt border border-border rounded-lg
                               hover:border-primary/50 hover:bg-primary/5 transition-colors
                               cursor-pointer text-left group"
                  >
                    <div
                      className="w-10 h-10 rounded-md bg-primary/15 flex items-center justify-center
                                    text-primary font-bold text-sm shrink-0"
                    >
                      {d.source.charAt(0)}
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-sm font-medium text-text group-hover:text-primary transition-colors">
                        {d.source}
                      </p>
                      <p className="text-xs text-text-dim truncate mt-0.5">
                        {d.path}
                      </p>
                    </div>
                    <span className="text-text-faint text-xs">&rarr;</span>
                  </button>
                ))}
              </>
            )}

            {discoveries.length === 0 && (
              <div className="p-4 bg-bg-alt border border-border rounded-lg text-center">
                <p className="text-sm text-text-dim">
                  No Star Citizen installations detected.
                </p>
              </div>
            )}

            {/* Browse custom */}
            <button
              onClick={handleBrowse}
              className="flex items-center justify-center gap-2 p-3 border border-dashed border-surface-hi
                         rounded-lg text-sm text-text-sub hover:text-text hover:border-primary/50
                         hover:bg-primary/5 transition-colors cursor-pointer"
            >
              Browse for Data.p4k...
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
