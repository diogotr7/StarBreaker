// A modal-style progress overlay that aggregates many sub-phases into
// one bar. Both viewer surfaces (Ships export, SOC scene load) feed
// into the same component shape so a long-running load has continuous
// feedback from the moment the user clicks until the scene is fully
// textured -- not a backend bar that disappears mid-load while the
// frontend still has tens of seconds of work to do.
//
// The component is presentational. The phase array + aggregate fraction
// are computed by `useProgressReporter` (sibling file) which the
// parent route owns; both backend event handlers and the viewer
// components write into one reporter, and that reporter feeds this
// overlay.

import { type ReactNode } from "react";

export interface ProgressPhase {
  /** Stable identifier; used by the reporter to update one phase
   *  in-place without rebuilding the whole array. */
  id: string;
  /** Human label for the phase, e.g. "Composing scene" or
   *  "Resolving textures". Shown when this phase is the active one. */
  label: string;
  /** Relative weight in the aggregate progress bar. A backend
   *  compose+emit step that runs in ~10s carries less weight than the
   *  texture-decode step that runs in ~50s; the weights make the
   *  overall bar move at a steady visible pace rather than jumping. */
  weight: number;
  /** [0,1] within this phase, or null for an indeterminate / pending
   *  phase. Null phases contribute zero to the aggregate fraction. */
  fraction: number | null;
  /** Optional sub-line shown beneath the phase label, e.g.
   *  "412 / 1482 textures" or "Resolving brushes...". */
  detail?: string;
  status: "pending" | "active" | "complete";
}

export interface ProgressOverlayProps {
  /** Top-line title of the operation, e.g. "Loading scene" or
   *  "Exporting RSI Polaris". */
  title: string;
  /** Phases in display order; usually backend → fetch → decode. The
   *  bar aggregates across all of them by weight. */
  phases: ProgressPhase[];
  /** Optional cancel handler. If provided, a Cancel button renders
   *  beneath the bar. */
  onCancel?: () => void;
  /** Tooltip on the cancel button — context for what cancel actually
   *  does (e.g. backend keeps running, vs. truly aborts). */
  cancelTitle?: string;
  /** Optional ReactNode rendered above the modal body. Use for
   *  per-surface error chrome that should sit alongside the modal. */
  children?: ReactNode;
}

/** Compute the aggregate fraction across all phases, weighted. Pending
 *  phases contribute zero; complete phases contribute their full
 *  weight; active phases contribute weight * fraction. */
export function computeAggregateFraction(phases: ProgressPhase[]): number {
  let totalWeight = 0;
  let earned = 0;
  for (const phase of phases) {
    if (phase.weight <= 0) continue;
    totalWeight += phase.weight;
    if (phase.status === "complete") {
      earned += phase.weight;
    } else if (phase.status === "active") {
      const frac =
        typeof phase.fraction === "number" && phase.fraction >= 0
          ? Math.min(phase.fraction, 1)
          : 0;
      earned += phase.weight * frac;
    }
  }
  if (totalWeight === 0) return 0;
  return Math.min(1, earned / totalWeight);
}

/** Pick the phase whose label + detail should be shown beneath the
 *  aggregate bar. Prefers the first `active` phase; falls back to the
 *  first non-`complete` phase; finally the last phase. */
function pickActivePhase(phases: ProgressPhase[]): ProgressPhase | null {
  if (phases.length === 0) return null;
  for (const p of phases) {
    if (p.status === "active") return p;
  }
  for (const p of phases) {
    if (p.status !== "complete") return p;
  }
  return phases[phases.length - 1];
}

export function ProgressOverlay({
  title,
  phases,
  onCancel,
  cancelTitle,
  children,
}: ProgressOverlayProps) {
  const aggregate = computeAggregateFraction(phases);
  const active = pickActivePhase(phases);

  return (
    <div className="absolute inset-0 z-10 bg-bg/85 backdrop-blur-sm flex items-center justify-center">
      {children}
      <div className="w-[420px] bg-bg-alt border border-border rounded-lg p-6 flex flex-col gap-4 shadow-lg">
        <h3 className="text-sm font-semibold text-text">{title}</h3>

        <div className="flex flex-col gap-1.5">
          <div className="w-full bg-surface rounded-full h-2 overflow-hidden">
            <div
              className="bg-accent h-full rounded-full transition-all duration-300"
              style={{ width: `${aggregate * 100}%` }}
            />
          </div>
          <div className="flex items-center justify-between text-[11px] text-text-dim">
            <span className="truncate">
              {active?.detail ?? active?.label ?? "Working..."}
            </span>
            <span className="tabular-nums">
              {Math.round(aggregate * 100)}%
            </span>
          </div>
        </div>

        {phases.length > 1 && (
          <div className="flex flex-col gap-1 text-[10px] text-text-faint border-t border-border pt-2">
            {phases.map((phase) => {
              const dot =
                phase.status === "complete"
                  ? "bg-success/70"
                  : phase.status === "active"
                    ? "bg-accent"
                    : "bg-border";
              const text =
                phase.status === "complete"
                  ? "text-text-dim"
                  : phase.status === "active"
                    ? "text-text-sub"
                    : "text-text-faint";
              return (
                <div
                  key={phase.id}
                  className={`flex items-center gap-2 ${text}`}
                >
                  <span
                    className={`w-1.5 h-1.5 rounded-full shrink-0 ${dot}`}
                  />
                  <span className="truncate">{phase.label}</span>
                  {phase.status === "active" &&
                    typeof phase.fraction === "number" && (
                      <span className="ml-auto tabular-nums">
                        {Math.round(phase.fraction * 100)}%
                      </span>
                    )}
                </div>
              );
            })}
          </div>
        )}

        {onCancel && (
          <button
            onClick={onCancel}
            className="w-full py-2 rounded-md text-xs font-medium bg-danger/15 text-danger hover:bg-danger/25 transition-colors cursor-pointer"
            title={cancelTitle}
          >
            Cancel
          </button>
        )}
      </div>
    </div>
  );
}
