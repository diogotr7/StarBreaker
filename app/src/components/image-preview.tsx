import { useEffect, useRef, useState } from "react";
import { readP4kFile } from "../lib/commands";
import { ImagePanZoom } from "./image-pan-zoom";

interface Props {
  path: string;
}

const MIME_TYPES: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".bmp": "image/bmp",
  ".tga": "image/x-tga",
};

function mimeForPath(path: string): string {
  const lower = path.toLowerCase();
  for (const [ext, mime] of Object.entries(MIME_TYPES)) {
    if (lower.endsWith(ext)) return mime;
  }
  return "application/octet-stream";
}

export function ImagePreview({ path }: Props) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const generationRef = useRef(0);

  useEffect(() => {
    const gen = ++generationRef.current;
    setLoading(true);
    setError(null);

    setObjectUrl((prev) => {
      if (prev) URL.revokeObjectURL(prev);
      return null;
    });

    readP4kFile(path)
      .then((buffer) => {
        if (gen !== generationRef.current) return;
        const blob = new Blob([buffer], { type: mimeForPath(path) });
        setObjectUrl(URL.createObjectURL(blob));
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
  }, [path]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-text-dim">
        <span className="text-xs">Loading image...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center px-8">
          <p className="text-danger text-sm font-medium">Failed to load image</p>
          <p className="text-text-dim text-xs mt-1 font-mono break-all">{error}</p>
        </div>
      </div>
    );
  }

  if (!objectUrl) return null;

  return (
    <ImagePanZoom src={objectUrl} alt={path.split("/").pop() ?? "image"} />
  );
}
