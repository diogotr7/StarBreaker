import { Archive, Database, Box, Volume2, type LucideIcon } from "lucide-react";
import { useAppStore, type AppMode } from "../stores/app-store";

interface ModeButton {
  id: AppMode;
  label: string;
  icon: LucideIcon;
}

function formatBytes(bytes: number): string {
  if (bytes >= 1e12) return `${(bytes / 1e12).toFixed(1)} TB`;
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
  if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(1)} KB`;
  return `${bytes} B`;
}

const modes: ModeButton[] = [
  { id: "p4k", label: "P4k Browser", icon: Archive },
  { id: "datacore", label: "DataCore", icon: Database },
  { id: "export", label: "3D Export", icon: Box },
  { id: "audio", label: "Audio", icon: Volume2 },
];

export function Sidebar() {
  const mode = useAppStore((s) => s.mode);
  const setMode = useAppStore((s) => s.setMode);
  const entryCount = useAppStore((s) => s.entryCount);
  const totalBytes = useAppStore((s) => s.totalBytes);
  const hasData = useAppStore((s) => s.hasData);
  const p4kSource = useAppStore((s) => s.p4kSource);
  const p4kPath = useAppStore((s) => s.p4kPath);

  const fileName = p4kPath?.split(/[/\\]/).pop() ?? null;

  return (
    <aside className="w-[180px] flex flex-col bg-bg-alt border-r border-border shrink-0">
      {hasData && (
        <div className="px-3 flex flex-col justify-center border-b border-border" style={{ height: "var(--toolbar-height)" }}>
          <p className="text-xs font-medium text-text-sub truncate" title={p4kPath ?? undefined}>
            {p4kSource === "custom" ? fileName : (p4kSource ?? fileName)}
          </p>
          <p className="text-[11px] text-text-dim mt-0.5">
            {entryCount.toLocaleString()} files &middot; {formatBytes(totalBytes)}
          </p>
        </div>
      )}

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
                  ? "bg-primary/15 text-text"
                  : "text-text-sub hover:text-text hover:bg-surface/50"
              }
            `}
          >
            <m.icon size={16} strokeWidth={1.75} />
            {m.label}
          </button>
        ))}
      </nav>
    </aside>
  );
}
