// Bottom-right panel showing flight-cam HUD + per-surface stats.
// Used by both Ships and SOC viewers so the chrome, position, and
// hide/show behaviour stay in lockstep. Per-surface content slots in
// via `children` (stat rows beneath the HUD divider) and `actions`
// (a row of buttons rendered alongside the first stat row, e.g. the
// Ships viewer's DBG fallback toggle).
//
// Hide/show is owned internally; the parent doesn't need to thread
// state. Click the "hide" pill to collapse to a tiny "show stats"
// re-expand affordance.

import { useState, type ReactNode } from "react";
import { FlightCamHud } from "./flight-cam-hud";
import type { FlightCamHandle } from "../lib/flight-camera";

export interface SceneInfoPanelProps {
  flightCamHandle: FlightCamHandle | null;
  /** Stat rows rendered beneath the flight-cam HUD divider. Should be
   *  null/undefined when the surface has no stats yet (e.g. mid-load);
   *  the divider and content area are then suppressed and the panel
   *  shows just the HUD. */
  children?: ReactNode;
  /** Optional element rendered inline with the first stat row (right
   *  side). Used by the Ships viewer for its DBG-fallback toggle. */
  actions?: ReactNode;
  /** Tooltip on the actions slot row. The Ships DBG toggle has a long
   *  explanation; this prop just propagates it. */
  actionsTitle?: string;
  /** When true, the panel renders without its own absolute
   *  positioning. The caller is expected to position it (e.g. inside
   *  a bottom-right flex column alongside the NavWidget). When
   *  false (default) the panel pins itself to bottom-2 right-2. */
  embedded?: boolean;
}

export function SceneInfoPanel({
  flightCamHandle,
  children,
  actions,
  actionsTitle,
  embedded = false,
}: SceneInfoPanelProps) {
  const [visible, setVisible] = useState(true);

  const positioning = embedded ? "" : "absolute bottom-2 right-2 z-10";

  if (!visible) {
    const collapsedClass = embedded
      ? "text-[10px] px-2 py-1 rounded border border-border bg-bg-alt/90 text-text-sub hover:bg-bg-alt hover:text-text font-mono shadow"
      : `${positioning} text-[10px] px-2 py-1 rounded border border-border bg-bg-alt/90 text-text-sub hover:bg-bg-alt hover:text-text font-mono shadow`;
    return (
      <button
        type="button"
        onClick={() => setVisible(true)}
        title="Show stats panel"
        aria-label="Show stats panel"
        className={collapsedClass}
      >
        show stats
      </button>
    );
  }

  const expandedClass = embedded
    ? "max-w-md rounded-md bg-bg-alt/95 border border-border shadow"
    : `${positioning} max-w-md rounded-md bg-bg-alt/95 border border-border shadow`;

  return (
    <div className={expandedClass}>
      <div className="flex items-start justify-between gap-2 px-3 py-1.5">
        <FlightCamHud handle={flightCamHandle} embedded />
        <button
          type="button"
          onClick={() => setVisible(false)}
          title="Hide stats panel"
          aria-label="Hide stats panel"
          className="text-[10px] px-1.5 py-0.5 rounded border border-border bg-bg/50 text-text-sub hover:bg-bg/80 hover:text-text font-mono shrink-0"
        >
          hide
        </button>
      </div>
      {children && (
        <div className="border-t border-border px-3 py-1.5">
          {actions ? (
            <div
              className="flex items-center justify-between gap-2"
              title={actionsTitle}
            >
              <div className="flex-1 min-w-0">{children}</div>
              <div className="shrink-0">{actions}</div>
            </div>
          ) : (
            children
          )}
        </div>
      )}
    </div>
  );
}
