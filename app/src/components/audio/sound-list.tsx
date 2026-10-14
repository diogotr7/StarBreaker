import { Download } from "lucide-react";
import { audioExportInfo, audioExportMedia } from "../../lib/commands";
import { useAudioStore } from "../../stores/audio-store";
import type { AudioSoundResult } from "../../lib/commands";

function exportBaseName(sound: AudioSoundResult) {
  return (sound.label || String(sound.media_id))
    .replace(/[^a-z0-9._-]+/gi, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 160) || String(sound.media_id);
}

export function SoundList() {
  const sounds = useAudioStore((s) => s.sounds);
  const currentSound = useAudioStore((s) => s.currentSound);
  const playSound = useAudioStore((s) => s.playSound);
  const selectedTrigger = useAudioStore((s) => s.selectedTrigger);

  const handleExport = async (sound: AudioSoundResult) => {
    let extension = "wem";
    try {
      const info = await audioExportInfo(
        sound.media_id,
        sound.source_type,
        sound.bank_name,
        sound.media_path,
      );
      extension = info.extension;
    } catch (err) {
      useAudioStore.setState({ error: String(err) });
      return;
    }

    const { save } = await import("@tauri-apps/plugin-dialog");
    const outputPath = await save({
      title: `Export ${sound.label}`,
      defaultPath: `${exportBaseName(sound)}.${extension}`,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!outputPath) return;

    try {
      await audioExportMedia(
        sound.media_id,
        sound.source_type,
        sound.bank_name,
        sound.media_path,
        outputPath,
      );
    } catch (err) {
      useAudioStore.setState({ error: String(err) });
    }
  };

  return (
    <div className="flex-1 min-w-[200px] flex flex-col overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Sounds {sounds.length > 0 && `(${sounds.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {sounds.map((sound, index) => {
          const isActive =
            sound.playable
            && currentSound?.media_id === sound.media_id
            && currentSound?.label === sound.label
            && currentSound?.media_path === sound.media_path;
          const sourceClass =
            sound.source_type === "Embedded"
              ? "bg-success/15 text-success"
              : sound.source_type === "ExternalSource"
                ? "bg-info/15 text-info"
              : sound.source_type === "Unavailable"
                ? "bg-danger/15 text-danger"
                : "bg-warning/15 text-warning";
          const mediaTitle = sound.path_description || `${sound.source_type} ${sound.label}`;
          return (
            <div
              key={`${sound.label}-${sound.media_path ?? sound.media_id}-${index}`}
              className={`group flex items-center gap-2 px-3 py-1.5 text-sm ${
                isActive
                  ? "bg-primary/15 text-text"
                  : "text-text-sub hover:bg-surface/50"
              }`}
            >
              <button
                type="button"
                onClick={() => playSound(sound)}
                disabled={!sound.playable}
                className="shrink-0 w-6 h-6 flex items-center justify-center rounded bg-surface hover:bg-surface-hi
                           transition-colors text-xs disabled:opacity-40 disabled:cursor-not-allowed
                           disabled:hover:bg-surface"
                title={sound.playable ? `Play ${sound.label}` : mediaTitle}
              >
                {isActive ? "||" : "▶"}
              </button>
              <span className="font-mono text-xs truncate" title={sound.label}>{sound.label}</span>
              <span
                className={`text-xs px-1.5 py-0.5 rounded ${sourceClass}`}
                title={mediaTitle}
              >
                {sound.source_type}
              </span>
              <button
                type="button"
                onClick={() => handleExport(sound)}
                disabled={!sound.playable}
                title={sound.playable ? `Export ${sound.label}` : mediaTitle}
                className="ml-auto hidden group-hover:flex items-center justify-center w-5 h-5 rounded
                           text-text-dim hover:text-text hover:bg-surface-hi transition-colors
                           disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:text-text-dim disabled:hover:bg-transparent"
              >
                <Download size={12} />
              </button>
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
