import { useCallback, useEffect, useRef } from "react";
import { useAudioStore } from "../stores/audio-store";
import { EntityList } from "../components/audio/entity-list";
import { TriggerList } from "../components/audio/trigger-list";
import { SoundList } from "../components/audio/sound-list";
import { AudioPlayer } from "../components/audio/audio-player";

export function AudioView() {
  const isInitialized = useAudioStore((s) => s.isInitialized);
  const isInitializing = useAudioStore((s) => s.isInitializing);
  const init = useAudioStore((s) => s.init);
  const error = useAudioStore((s) => s.error);

  useEffect(() => {
    if (!isInitialized && !isInitializing) {
      init();
    }
  }, [isInitialized, isInitializing, init]);

  if (!isInitialized) {
    return (
      <div className="flex-1 flex items-center justify-center text-text-dim">
        {isInitializing ? "Building ATL index..." : error ?? "Initializing..."}
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <SearchBar />
      <ErrorBar />
      <div className="flex-1 flex overflow-hidden">
        <EntityList />
        <TriggerList />
        <SoundList />
      </div>
      <AudioPlayer />
    </div>
  );
}

function SearchBar() {
  const searchQuery = useAudioStore((s) => s.searchQuery);
  const searchMode = useAudioStore((s) => s.searchMode);
  const setSearchMode = useAudioStore((s) => s.setSearchMode);
  const search = useAudioStore((s) => s.search);
  const isSearching = useAudioStore((s) => s.isSearching);
  const triggerCount = useAudioStore((s) => s.triggerCount);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(null);

  const onInput = useCallback(
    (value: string) => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => search(value), 250);
    },
    [search],
  );

  return (
    <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-bg-alt">
      <input
        key={searchMode}
        type="text"
        placeholder={searchMode === "entity" ? "Search entities..." : searchMode === "bank" ? "Filter banks..." : "Search triggers..."}
        defaultValue={searchQuery}
        onChange={(e) => onInput(e.target.value)}
        className="flex-1 bg-surface px-3 py-1.5 rounded text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring"
      />
      <div className="flex gap-1">
        {(["bank", "trigger", "entity"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            onClick={() => setSearchMode(mode)}
            className={`px-3 py-1 text-xs rounded transition-colors capitalize ${
              searchMode === mode
                ? "bg-primary text-bg-deep"
                : "bg-surface text-text-dim hover:bg-surface-hi hover:text-text"
            }`}
          >
            {mode}
          </button>
        ))}
      </div>
      {isSearching && <span className="text-xs text-text-dim">Searching...</span>}
      <span className="text-xs text-text-faint">{triggerCount.toLocaleString()} triggers</span>
    </div>
  );
}

function ErrorBar() {
  const error = useAudioStore((s) => s.error);
  if (!error) return null;
  return (
    <div className="px-3 py-1.5 text-xs text-danger bg-danger/10 border-b border-danger/20">
      {error}
    </div>
  );
}
