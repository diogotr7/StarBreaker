import { useEffect, useRef, useState } from "react";
import { previewDds } from "../lib/commands";
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
          <p className="text-red-400 text-sm font-medium">
            Failed to load texture
          </p>
          <p className="text-text-dim text-xs mt-1 font-mono break-all">
            {error}
          </p>
        </div>
      </div>
    );
  }

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
              className="w-24 accent-primary"
            />
            <span className="tabular-nums">
              {currentMip}/{mipCount - 1}
            </span>
          </>
        )}
        {loading && (
          <span className="text-text-dim animate-pulse">loading...</span>
        )}
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
