import { useEffect, useRef, useCallback } from "react";

interface Props {
  src: string;
  alt?: string;
}

/**
 * Pan & zoom image viewer.
 * - Scroll to zoom toward/away from cursor
 * - Drag to pan (clamped so image can't leave viewport)
 * - Double-click to reset
 */
export function ImagePanZoom({ src, alt }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  // Store transform in a ref to avoid re-renders on every mouse move
  const transform = useRef({ x: 0, y: 0, scale: 1 });
  const dragRef = useRef<{
    startX: number;
    startY: number;
    tx: number;
    ty: number;
  } | null>(null);

  const applyTransform = useCallback(() => {
    const el = imgRef.current;
    if (!el) return;
    const { x, y, scale } = transform.current;
    el.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
    el.style.imageRendering = scale > 2 ? "pixelated" : "auto";
  }, []);

  const clampTranslate = useCallback(() => {
    const container = containerRef.current;
    const img = imgRef.current;
    if (!container || !img) return;

    const { scale } = transform.current;
    const cw = container.clientWidth;
    const ch = container.clientHeight;
    // Natural rendered size of the image (before our scale)
    const iw = img.naturalWidth;
    const ih = img.naturalHeight;

    // Fit the image to the container (object-contain logic)
    const fitScale = Math.min(cw / iw, ch / ih, 1);
    const displayW = iw * fitScale * scale;
    const displayH = ih * fitScale * scale;

    // How far the image can move before leaving the viewport
    const maxX = Math.max(0, (displayW - cw) / 2);
    const maxY = Math.max(0, (displayH - ch) / 2);

    transform.current.x = Math.min(maxX, Math.max(-maxX, transform.current.x));
    transform.current.y = Math.min(maxY, Math.max(-maxY, transform.current.y));
  }, []);

  // Reset on image change
  useEffect(() => {
    transform.current = { x: 0, y: 0, scale: 1 };
    applyTransform();
  }, [src, applyTransform]);

  // Wheel: zoom toward cursor
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const t = transform.current;
      const rect = container.getBoundingClientRect();

      // Mouse position relative to container center
      const mx = e.clientX - rect.left - rect.width / 2;
      const my = e.clientY - rect.top - rect.height / 2;

      const factor = e.deltaY > 0 ? 0.9 : 1.1;
      const newScale = Math.min(Math.max(t.scale * factor, 0.1), 50);
      const ratio = newScale / t.scale;

      // Adjust translate so the point under the cursor stays fixed
      t.x = mx - ratio * (mx - t.x);
      t.y = my - ratio * (my - t.y);
      t.scale = newScale;

      clampTranslate();
      applyTransform();
    };

    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, [applyTransform, clampTranslate]);

  // Drag to pan
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const t = transform.current;
    dragRef.current = { startX: e.clientX, startY: e.clientY, tx: t.x, ty: t.y };
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      transform.current.x = d.tx + (e.clientX - d.startX);
      transform.current.y = d.ty + (e.clientY - d.startY);
      clampTranslate();
      applyTransform();
    };
    const onUp = () => {
      dragRef.current = null;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [applyTransform, clampTranslate]);

  const handleDoubleClick = useCallback(() => {
    transform.current = { x: 0, y: 0, scale: 1 };
    applyTransform();
  }, [applyTransform]);

  return (
    <div
      ref={containerRef}
      className="w-full h-full overflow-hidden cursor-grab active:cursor-grabbing flex items-center justify-center"
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
    >
      <img
        ref={imgRef}
        src={src}
        alt={alt ?? "preview"}
        className="max-w-full max-h-full object-contain select-none"
        draggable={false}
        style={{ transformOrigin: "center center", willChange: "transform" }}
      />
    </div>
  );
}
