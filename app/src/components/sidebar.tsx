import { useAppStore, type AppMode } from "../stores/app-store";

interface ModeButton {
  id: AppMode;
  label: string;
  icon: string;
}

const modes: ModeButton[] = [
  { id: "p4k", label: "P4k Browser", icon: "📦" },
  { id: "datacore", label: "DataCore", icon: "🗃️" },
  { id: "export", label: "3D Export", icon: "🧊" },
  { id: "audio", label: "Audio", icon: "🔊" },
];

export function Sidebar() {
  const mode = useAppStore((s) => s.mode);
  const setMode = useAppStore((s) => s.setMode);
  const entryCount = useAppStore((s) => s.entryCount);
  const hasData = useAppStore((s) => s.hasData);

  return (
    <aside className="w-[180px] flex flex-col bg-bg-alt border-r border-border shrink-0">
      <div className="px-4 py-5 border-b border-border">
        <h1 className="text-lg font-bold text-primary tracking-tight">
          StarBreaker
        </h1>
        <p className="text-xs text-text-dim mt-0.5">SC Data Explorer</p>
      </div>

      <nav className="flex-1 py-2 px-2 flex flex-col gap-1">
        {modes.map((m) => (
          <button
            key={m.id}
            onClick={() => setMode(m.id)}
            disabled={!hasData}
            className={`
              flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium
              transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed
              ${
                mode === m.id
                  ? "bg-primary/15 text-primary"
                  : "text-text-sub hover:text-text hover:bg-surface/50"
              }
            `}
          >
            <span className="text-base">{m.icon}</span>
            {m.label}
          </button>
        ))}
      </nav>

      {hasData && (
        <div className="px-4 py-3 border-t border-border">
          <p className="text-xs text-text-dim">
            {entryCount.toLocaleString()} entries
          </p>
        </div>
      )}
    </aside>
  );
}
