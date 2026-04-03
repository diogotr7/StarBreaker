import { useEffect, useState } from "react";
import { onFolderExtractProgress, type FolderExtractProgress } from "../lib/commands";

interface ExtractProgressProps {
  active: boolean;
  onDone?: (count: number) => void;
}

export function ExtractProgress({ active, onDone }: ExtractProgressProps) {
  const [progress, setProgress] = useState<FolderExtractProgress | null>(null);

  useEffect(() => {
    if (!active) {
      setProgress(null);
      return;
    }
    const unlisten = onFolderExtractProgress((p) => {
      setProgress(p);
      if (p.current >= p.total) {
        onDone?.(p.total);
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, [active, onDone]);

  if (!active || !progress) return null;

  const fraction = progress.total > 0 ? progress.current / progress.total : 0;

  return (
    <div className="absolute inset-0 z-10 bg-bg/80 backdrop-blur-sm flex items-center justify-center">
      <div className="w-[320px] bg-bg-alt border border-border rounded-lg p-5 flex flex-col gap-3 shadow-lg">
        <h3 className="text-sm font-medium text-text">Extracting...</h3>
        <div className="w-full bg-surface rounded-full h-1.5 overflow-hidden">
          <div
            className="bg-primary h-full rounded-full transition-all duration-150"
            style={{ width: `${fraction * 100}%` }}
          />
        </div>
        <div className="flex items-center justify-between">
          <p className="text-[11px] text-text-dim truncate flex-1">{progress.name}</p>
          <span className="text-[11px] text-text-faint tabular-nums ml-2 shrink-0">
            {progress.current}/{progress.total}
          </span>
        </div>
      </div>
    </div>
  );
}
