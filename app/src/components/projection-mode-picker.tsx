// Dropdown for cycling the flight cam's projection mode. By default
// renders as a free-floating top-right pill, but can be embedded in a
// parent flex container by passing `embedded` to drop the absolute
// positioning. Subscribes to the same flight-cam handle the HUD does
// so it shows the live mode (e.g. when the user pressed P from the
// canvas).

import { useEffect, useState } from "react";
import {
  PROJECTION_MODES,
  type FlightCamHandle,
  type FlightCamState,
  type ProjectionMode,
} from "../lib/flight-camera";

const LABELS: Record<ProjectionMode, string> = {
  perspective: "persp",
  orthographic: "ortho",
  oblique: "oblique",
};

interface Props {
  handle: FlightCamHandle | null;
  /** When true, drops the absolute positioning so the picker can live
   *  inside a parent flex row (e.g. the top-right toolbar). */
  embedded?: boolean;
}

export function ProjectionModePicker({ handle, embedded = false }: Props) {
  const [snapshot, setSnapshot] = useState<Readonly<FlightCamState> | null>(null);

  useEffect(() => {
    if (!handle) return;
    const off = handle.subscribe((s) => setSnapshot(s));
    return off;
  }, [handle]);

  if (!handle || !snapshot) return null;

  const baseClasses =
    "flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-bg-alt/90 border border-border text-xs text-text-sub shadow";
  const positioning = embedded ? "" : "absolute top-2 right-2 z-10";
  const className = positioning ? `${positioning} ${baseClasses}` : baseClasses;

  return (
    <label className={className}>
      <span className="text-text-faint">Projection</span>
      <select
        value={snapshot.projectionMode}
        onChange={(e) => handle.setProjectionMode(e.target.value as ProjectionMode)}
        className="bg-transparent outline-none text-text cursor-pointer"
      >
        {PROJECTION_MODES.map((m) => (
          <option key={m} value={m} className="bg-bg-alt text-text">
            {LABELS[m]}
          </option>
        ))}
      </select>
    </label>
  );
}
