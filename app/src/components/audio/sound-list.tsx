import { useAudioStore } from "../../stores/audio-store";

export function SoundList() {
  const sounds = useAudioStore((s) => s.sounds);
  const currentSound = useAudioStore((s) => s.currentSound);
  const playSound = useAudioStore((s) => s.playSound);
  const selectedTrigger = useAudioStore((s) => s.selectedTrigger);

  return (
    <div className="flex-1 min-w-[200px] flex flex-col overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Sounds {sounds.length > 0 && `(${sounds.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {sounds.map((sound, index) => {
          const isActive = currentSound?.media_id === sound.media_id;
          return (
            <div
              key={`${sound.media_id}-${index}`}
              className={`flex items-center gap-2 px-3 py-1.5 text-sm ${
                isActive
                  ? "bg-primary/15 text-primary"
                  : "text-text-sub hover:bg-surface/50"
              }`}
            >
              <button
                type="button"
                onClick={() => playSound(sound)}
                className="shrink-0 w-6 h-6 flex items-center justify-center rounded bg-surface hover:bg-surface-hi transition-colors text-xs"
                title={`Play ${sound.media_id}`}
              >
                {isActive ? "||" : "▶"}
              </button>
              <span className="font-mono text-xs">{sound.media_id}</span>
              <span
                className={`text-xs px-1.5 py-0.5 rounded ${
                  sound.source_type === "Embedded"
                    ? "bg-success/15 text-success"
                    : "bg-warning/15 text-warning"
                }`}
              >
                {sound.source_type}
              </span>
            </div>
          );
        })}
        {sounds.length === 0 && (
          <div className="px-3 py-4 text-xs text-text-faint text-center">
            {selectedTrigger ? "No sounds resolved" : "Select a trigger to see sounds"}
          </div>
        )}
      </div>
    </div>
  );
}
