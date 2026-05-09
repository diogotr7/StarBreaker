import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface ContextMenuItem {
  label: string;
  onClick: () => void;
  /** Render with destructive styling. */
  danger?: boolean;
  /** Render the item but disabled (greyed, no click). */
  disabled?: boolean;
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: ContextMenuItem[];
}

/**
 * Floating context menu pinned to a viewport coordinate.
 *
 * Dismisses on: outside click, Escape, scroll, window blur, and after any
 * item is clicked. The parent owns the open/closed state via `useContextMenu`
 * (or any equivalent state) and passes `null` while closed.
 */
export function ContextMenu({
  state,
  onClose,
}: {
  state: ContextMenuState | null;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // Clamp the menu inside the viewport once it's rendered and we know its size.
  useLayoutEffect(() => {
    if (!state) {
      setPos(null);
      return;
    }
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const margin = 4;
    const left = Math.min(state.x, window.innerWidth - rect.width - margin);
    const top = Math.min(state.y, window.innerHeight - rect.height - margin);
    setPos({ left: Math.max(margin, left), top: Math.max(margin, top) });
  }, [state]);

  useEffect(() => {
    if (!state) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onMouseDown);
    window.addEventListener("blur", onClose);
    window.addEventListener("scroll", onClose, true);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("scroll", onClose, true);
    };
  }, [state, onClose]);

  if (!state) return null;

  return (
    <div
      ref={ref}
      role="menu"
      style={{
        position: "fixed",
        left: pos?.left ?? state.x,
        top: pos?.top ?? state.y,
        visibility: pos ? "visible" : "hidden",
        zIndex: 100,
      }}
      className="min-w-[180px] py-1 rounded-md border border-border bg-bg-alt shadow-xl"
    >
      {state.items.map((item, i) => (
        <button
          key={i}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          onClick={() => {
            if (item.disabled) return;
            item.onClick();
            onClose();
          }}
          className={`block w-full text-left px-3 py-1.5 text-sm transition-colors disabled:opacity-50 disabled:pointer-events-none ${
            item.danger
              ? "text-red-400 hover:bg-red-500/10"
              : "text-text hover:bg-surface"
          }`}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

/** Convenience hook: open/close state + an `open(e, items)` helper that takes
 *  a React mouse event (typically a `onContextMenu`) and a menu item array. */
export function useContextMenu() {
  const [state, setState] = useState<ContextMenuState | null>(null);
  return {
    state,
    open: (e: React.MouseEvent, items: ContextMenuItem[]) => {
      e.preventDefault();
      setState({ x: e.clientX, y: e.clientY, items });
    },
    close: () => setState(null),
  };
}
