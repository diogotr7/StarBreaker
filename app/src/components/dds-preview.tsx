import { useEffect, useRef, useState } from "react";
import { previewDds } from "../lib/commands";

interface Props {
  path: string;
}

export function DdsPreview({ path }: Props) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const generationRef = useRef(0);

  useEffect(() => {
    const gen = ++generationRef.current;
    setLoading(true);
    setError(null);

    // Revoke previous URL
    setObjectUrl((prev) => {
      if (prev) URL.revokeObjectURL(prev);
      return null;
    });

    previewDds(path)
      .then((buffer) => {
        if (gen !== generationRef.current) return;
        const blob = new Blob([buffer], { type: "image/png" });
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
        <span className="text-xs">Loading texture...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center px-8">
          <p className="text-red-400 text-sm font-medium">Failed to load texture</p>
          <p className="text-text-dim text-xs mt-1 font-mono break-all">{error}</p>
        </div>
      </div>
    );
  }

  if (!objectUrl) return null;

  return (
    <div className="flex items-center justify-center h-full w-full overflow-auto p-4">
      <img
        src={objectUrl}
        alt={path.split("/").pop() ?? "texture"}
        className="max-w-full max-h-full object-contain"
        style={{ imageRendering: "auto" }}
      />
    </div>
  );
}
