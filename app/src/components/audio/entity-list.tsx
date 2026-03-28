import { useAudioStore } from "../../stores/audio-store";

export function EntityList() {
  const searchMode = useAudioStore((s) => s.searchMode);

  if (searchMode === "trigger") return null;
  if (searchMode === "bank") return <BankList />;
  return <EntityListInner />;
}

function EntityListInner() {
  const entities = useAudioStore((s) => s.entities);
  const selectedEntity = useAudioStore((s) => s.selectedEntity);
  const selectEntity = useAudioStore((s) => s.selectEntity);

  return (
    <div className="w-[28%] min-w-[180px] flex flex-col border-r border-border overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Entities {entities.length > 0 && `(${entities.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {entities.map((entity) => (
          <button
            key={entity.name}
            type="button"
            onClick={() => selectEntity(entity.name)}
            className={`w-full text-left px-3 py-1.5 text-sm transition-colors ${
              selectedEntity === entity.name
                ? "bg-primary/15 text-primary"
                : "text-text-sub hover:bg-surface/50 hover:text-text"
            }`}
          >
            <div className="truncate">{entity.name}</div>
            <div className="text-xs text-text-faint">
              {entity.trigger_count} trigger{entity.trigger_count !== 1 ? "s" : ""}
            </div>
          </button>
        ))}
        {entities.length === 0 && (
          <div className="px-3 py-4 text-xs text-text-faint text-center">
            Search for an entity to see results
          </div>
        )}
      </div>
    </div>
  );
}

function BankList() {
  const banks = useAudioStore((s) => s.banks);
  const selectedBank = useAudioStore((s) => s.selectedBank);
  const selectBank = useAudioStore((s) => s.selectBank);

  return (
    <div className="w-[28%] min-w-[180px] flex flex-col border-r border-border overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Banks {banks.length > 0 && `(${banks.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {banks.map((bank) => (
          <button
            key={bank.name}
            type="button"
            onClick={() => selectBank(bank.name)}
            className={`w-full text-left px-3 py-1.5 text-sm transition-colors ${
              selectedBank === bank.name
                ? "bg-primary/15 text-primary"
                : "text-text-sub hover:bg-surface/50 hover:text-text"
            }`}
          >
            <div className="truncate font-mono text-xs">{bank.name}</div>
            <div className="text-xs text-text-faint">
              {bank.trigger_count} trigger{bank.trigger_count !== 1 ? "s" : ""}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
