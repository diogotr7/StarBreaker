import { useEffect, useRef, useState, useCallback } from "react";

interface Props {
  src: string;
  alt?: string;
}

/**
 * Pan & zoom image viewer. Scroll to zoom, drag to pan, double-click to reset.
 */
export function ImagePanZoom({ src, alt }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const dragRef = useRef<{ startX: number; startY: number; tx: number; ty: number } | null>(null);

  // Reset transform when image changes
  useEffect(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, [src]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    setScale((s) => Math.min(Math.max(s * delta, 0.1), 50));
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      dragRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        tx: translate.x,
        ty: translate.y,
      };
    },
    [translate],
  );

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      setTranslate({
        x: d.tx + (e.clientX - d.startX),
        y: d.ty + (e.clientY - d.startY),
      });
    };
    const handleMouseUp = () => {
      dragRef.current = null;
    };
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, []);

  const handleDoubleClick = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  return (
    <div
      ref={containerRef}
      className="w-full h-full overflow-hidden cursor-grab active:cursor-grabbing"
      onWheel={handleWheel}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
    >
      <div
        className="w-full h-full flex items-center justify-center"
        style={{
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale})`,
          transformOrigin: "center center",
        }}
      >
        <img
          src={src}
          alt={alt ?? "preview"}
          className="max-w-full max-h-full object-contain"
          draggable={false}
          style={{ imageRendering: scale > 2 ? "pixelated" : "auto" }}
        />
      </div>
    </div>
  );
}
