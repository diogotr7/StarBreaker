import { useAudioStore } from "../../stores/audio-store";

export function TriggerList() {
  const triggers = useAudioStore((s) => s.triggers);
  const selectedTrigger = useAudioStore((s) => s.selectedTrigger);
  const selectTrigger = useAudioStore((s) => s.selectTrigger);
  const searchMode = useAudioStore((s) => s.searchMode);
  const selectedEntity = useAudioStore((s) => s.selectedEntity);

  const selectedBank = useAudioStore((s) => s.selectedBank);
  const showEmpty = (searchMode === "entity" && !selectedEntity) || (searchMode === "bank" && !selectedBank);

  return (
    <div className="w-[32%] min-w-[200px] flex flex-col border-r border-border overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Triggers {triggers.length > 0 && `(${triggers.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {triggers.map((trigger) => (
          <button
            key={trigger.trigger_name}
            type="button"
            onClick={() => selectTrigger(trigger.trigger_name)}
            className={`w-full text-left px-3 py-1.5 text-sm transition-colors ${
              selectedTrigger === trigger.trigger_name
                ? "bg-primary/15 text-primary"
                : "text-text-sub hover:bg-surface/50 hover:text-text"
            }`}
          >
            <div className="truncate">{trigger.trigger_name}</div>
            <div className="flex items-center gap-2 text-xs text-text-faint">
              <span className="bg-surface px-1.5 py-0.5 rounded">{trigger.bank_name}</span>
              {trigger.sound_count > 0 && <span>{trigger.sound_count} sounds</span>}
            </div>
          </button>
        ))}
        {triggers.length === 0 && (
          <div className="px-3 py-4 text-xs text-text-faint text-center">
            {showEmpty ? "Select an item to see triggers" : "No triggers found"}
          </div>
        )}
      </div>
    </div>
  );
}
