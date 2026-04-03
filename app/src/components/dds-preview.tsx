import { useEffect, useRef, useState } from "react";
import { Download } from "lucide-react";
import { previewDds, exportDdsPng } from "../lib/commands";
import { ImagePanZoom } from "./image-pan-zoom";

interface Props {
  path: string;
}

export function DdsPreview({ path }: Props) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [mipLevel, setMipLevel] = useState<number | undefined>(undefined);
  const [mipCount, setMipCount] = useState(0);
  const [dimensions, setDimensions] = useState<[number, number]>([0, 0]);
  const [currentMip, setCurrentMip] = useState(0);
  const generationRef = useRef(0);

  // Reset mip selection when path changes
  useEffect(() => {
    setMipLevel(undefined);
  }, [path]);

  useEffect(() => {
    const gen = ++generationRef.current;
    setLoading(true);
    setError(null);

    setObjectUrl((prev) => {
      if (prev) URL.revokeObjectURL(prev);
      return null;
    });

    previewDds(path, mipLevel)
      .then((result) => {
        if (gen !== generationRef.current) return;
        const blob = new Blob([new Uint8Array(result.png)], {
          type: "image/png",
        });
        setObjectUrl(URL.createObjectURL(blob));
        setMipCount(result.mip_count);
        setDimensions([result.width, result.height]);
        setCurrentMip(result.mip_level);
        setLoading(false);
      })
      .catch((err) => {
        if (gen !== generationRef.current) return;
        setError(String(err));
        setLoading(false);
      });

    return () => {
      setObjectUrl((prev) => {
        if (prev) URL.revokeObjectURL(prev);
        return null;
      });
    };
  }, [path, mipLevel]);

  if (loading && !objectUrl) {
    return (
      <div className="flex items-center justify-center h-full text-text-dim">
        <span className="text-xs">Loading texture...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center px-8">
          <p className="text-danger text-sm font-medium">
            Failed to load texture
          </p>
          <p className="text-text-dim text-xs mt-1 font-mono break-all">
            {error}
          </p>
        </div>
      </div>
    );
  }

  const handleSavePng = async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const stem = path.split(/[\\/]/).pop()?.replace(/\.dds$/i, "") ?? "texture";
    const outputPath = await save({
      title: "Save texture as PNG",
      defaultPath: `${stem}.png`,
      filters: [{ name: "PNG Image", extensions: ["png"] }],
    });
    if (!outputPath) return;
    try {
      await exportDdsPng(path, outputPath, mipLevel);
    } catch (err) {
      console.error("Failed to save PNG:", err);
    }
  };

  if (!objectUrl) return null;

  return (
    <div className="flex flex-col h-full w-full">
      {/* Mip controls */}
      <div className="flex items-center gap-3 px-3 py-1.5 border-b border-border text-xs text-text-dim shrink-0">
        <span>
          {dimensions[0]}x{dimensions[1]}
        </span>
        {mipCount > 1 && (
          <>
            <span className="text-border">|</span>
            <span>Mip</span>
            <input
              type="range"
              min={0}
              max={mipCount - 1}
              value={currentMip}
              onChange={(e) => setMipLevel(Number(e.target.value))}
              className="w-24 accent-accent"
            />
            <span className="tabular-nums">
              {currentMip}/{mipCount - 1}
            </span>
          </>
        )}
        {loading && (
          <span className="text-text-dim animate-pulse">loading...</span>
        )}
        <button
          onClick={handleSavePng}
          title="Save as PNG"
          className="ml-auto flex items-center gap-1.5 px-2 py-1 rounded-md text-text-dim
                     hover:text-text hover:bg-surface/60 transition-colors cursor-pointer"
        >
          <Download size={12} />
          <span>Save PNG</span>
        </button>
      </div>

      {/* Image with pan/zoom */}
      <div className="flex-1 overflow-hidden">
        <ImagePanZoom
          src={objectUrl}
          alt={path.split("/").pop() ?? "texture"}
        />
      </div>
    </div>
  );
}
