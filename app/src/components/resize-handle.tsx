import { useCallback, useEffect, useRef } from "react";

interface ResizeHandleProps {
  /** Current width of the panel being resized. */
  width: number;
  /** Callback when the user drags to a new width. */
  onResize: (width: number) => void;
  /** Which side of the panel this handle sits on. "right" = panel is on the left. */
  side?: "left" | "right";
  /** Minimum allowed width. */
  min?: number;
  /** Maximum allowed width. */
  max?: number;
}

export function ResizeHandle({
  width,
  onResize,
  side = "right",
  min = 120,
  max = 800,
}: ResizeHandleProps) {
  const dragging = useRef(false);
  const startX = useRef(0);
  const startWidth = useRef(0);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      startX.current = e.clientX;
      startWidth.current = width;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [width],
  );

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      const delta = e.clientX - startX.current;
      const newWidth = side === "right"
        ? startWidth.current + delta
        : startWidth.current - delta;
      onResize(Math.max(min, Math.min(max, newWidth)));
    };

    const onMouseUp = () => {
      if (!dragging.current) return;
      dragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [onResize, side, min, max]);

  return (
    <div
      onMouseDown={onMouseDown}
      className="w-0 relative shrink-0 z-10"
    >
      <div className="absolute inset-y-0 -left-1 w-2 cursor-col-resize hover:bg-primary/20 transition-colors" />
    </div>
  );
}
