// State container for the unified `<ProgressOverlay>`. The parent route
// owns a single reporter per long-running operation and passes the
// update API down to the viewer components that contribute to it (the
// SocSceneViewer instruments fetch + decode; the parent handles backend
// events). When all phases complete -- or the operation is cancelled --
// the reporter is reset and the overlay disappears.
//
// Phases are declared up front by the caller (`begin(phases)`) so the
// overlay's checkpoint list is stable from first paint and the user
// sees the work plan, not a list that grows mid-load.

import { useCallback, useMemo, useState } from "react";
import type { ProgressPhase } from "../components/progress-overlay";

/**
 * The reporter's mutating API. The four callbacks below are stable
 * across renders (each `useCallback` has empty deps), so consumers
 * MUST depend on these individually -- not on the whole reporter
 * object -- when wiring them into `useCallback` / `useEffect` deps.
 *
 * Why: the reporter object identity changes whenever the underlying
 * phases state updates (it has to, for React to re-render the
 * overlay). If a consumer puts the whole reporter in its deps, every
 * progress tick re-creates downstream callbacks; if those callbacks
 * are passed as props to children, the children's effects re-fire on
 * every progress tick. For a SOC load with ~1500 textures emitting
 * ticks at ~6/sec, this triggers thousands of effect re-runs and
 * saturates the Tauri IPC channel.
 *
 * The render-related fields (`phases`, `active`) DO change with
 * state and are intended for use in JSX only.
 */
export interface ProgressReporter {
  /** Snapshot of phases for rendering. Changes every progress tick. */
  phases: ProgressPhase[];
  /** True if any phase is `pending` or `active`. Render-only. */
  active: boolean;
  /** Replace the phase list and mark the reporter active. The first
   *  phase is auto-marked `active`; the rest start `pending`.
   *  STABLE reference. */
  begin: (phases: Omit<ProgressPhase, "status">[]) => void;
  /** Update one phase by id. Patch is shallow-merged. Status
   *  transitions are explicit. STABLE reference. */
  update: (id: string, patch: Partial<Omit<ProgressPhase, "id">>) => void;
  /** Mark a phase complete and optionally activate the next pending
   *  phase. STABLE reference. */
  advance: (completedId: string) => void;
  /** Reset to no phases. STABLE reference. */
  reset: () => void;
}

export function useProgressReporter(): ProgressReporter {
  const [phases, setPhases] = useState<ProgressPhase[]>([]);

  const begin = useCallback(
    (declared: Omit<ProgressPhase, "status">[]) => {
      const next: ProgressPhase[] = declared.map((p, i) => ({
        ...p,
        status: i === 0 ? "active" : "pending",
      }));
      setPhases(next);
    },
    [],
  );

  const update = useCallback(
    (id: string, patch: Partial<Omit<ProgressPhase, "id">>) => {
      setPhases((prev) => {
        let changed = false;
        const next = prev.map((phase) => {
          if (phase.id !== id) return phase;
          const merged = { ...phase, ...patch };
          // Skip the state update if nothing actually changed -- a
          // common case during XHR `progress` events that fire many
          // times per second with the same fraction (rounded). Keeps
          // React from re-rendering the overlay unnecessarily.
          if (
            merged.fraction === phase.fraction &&
            merged.detail === phase.detail &&
            merged.status === phase.status &&
            merged.label === phase.label &&
            merged.weight === phase.weight
          ) {
            return phase;
          }
          changed = true;
          return merged;
        });
        return changed ? next : prev;
      });
    },
    [],
  );

  const advance = useCallback((completedId: string) => {
    setPhases((prev) => {
      let foundCompleted = false;
      let activatedNext = false;
      const next = prev.map((phase) => {
        if (phase.id === completedId && phase.status !== "complete") {
          foundCompleted = true;
          return { ...phase, status: "complete" as const, fraction: 1 };
        }
        if (
          foundCompleted &&
          !activatedNext &&
          phase.status === "pending"
        ) {
          activatedNext = true;
          return { ...phase, status: "active" as const };
        }
        return phase;
      });
      return foundCompleted ? next : prev;
    });
  }, []);

  const reset = useCallback(() => {
    setPhases([]);
  }, []);

  const active =
    phases.length > 0 && phases.some((p) => p.status !== "complete");

  // Memoise the returned object so its identity only changes when
  // `phases` (and therefore `active`) changes. `begin` / `update` /
  // `advance` / `reset` are stable across renders, so adding them to
  // the dep array is a no-op past the first render. This is defensive
  // -- consumers are expected to depend on individual stable callbacks
  // rather than the whole object -- but the memo avoids cascades when
  // a consumer accidentally captures the whole `progress` value.
  return useMemo(
    () => ({ phases, active, begin, update, advance, reset }),
    [phases, active, begin, update, advance, reset],
  );
}
