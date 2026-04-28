// One-line readout for the flight camera state. Subscribes to the handle's
// per-frame snapshots; the handle internally throttles to one fire per RAF
// tick, so there is no extra throttling here.

import { useEffect, useState } from "react";
import * as THREE from "three";
import type {
  FlightCamHandle,
  FlightCamState,
  ProjectionMode,
} from "../lib/flight-camera";

const RAD_TO_DEG = 180 / Math.PI;

const PROJ_LABEL: Record<ProjectionMode, string> = {
  perspective: "persp",
  orthographic: "ortho",
  oblique: "oblique",
};

function formatVec(v: THREE.Vector3): string {
  return `${v.x.toFixed(1)}, ${v.y.toFixed(1)}, ${v.z.toFixed(1)}`;
}

interface Props {
  handle: FlightCamHandle | null;
  /** When true, render only the inline readout `<p>` without the
   *  absolute-positioned wrapper div. Used by `SceneViewer`'s combined
   *  bottom-right panel, which provides its own layout chrome. Defaults
   *  to false (legacy free-floating layout) so existing call sites and
   *  tests stay unchanged. */
  embedded?: boolean;
}

export function FlightCamHud({ handle, embedded = false }: Props) {
  const [snapshot, setSnapshot] = useState<Readonly<FlightCamState> | null>(null);

  useEffect(() => {
    if (!handle) return;
    const off = handle.subscribe((s) => setSnapshot(s));
    return off;
  }, [handle]);

  if (!snapshot) return null;

  const euler = new THREE.Euler().setFromQuaternion(snapshot.quat, "YXZ");
  const yaw = (euler.y * RAD_TO_DEG).toFixed(0);
  const pitch = (euler.x * RAD_TO_DEG).toFixed(0);
  const roll = (euler.z * RAD_TO_DEG).toFixed(0);

  const readout = (
    <p className="text-[11px] text-text-sub font-mono whitespace-nowrap">
      pos: {formatVec(snapshot.pos)}
      {"   "}
      yaw {yaw} deg pitch {pitch} deg roll {roll} deg
      {"   "}
      speed {snapshot.moveSpeed.toFixed(2)}x
      {"   "}
      fov {snapshot.fov.toFixed(0)} deg
      {"   "}
      proj {PROJ_LABEL[snapshot.projectionMode]}
    </p>
  );

  if (embedded) return readout;

  return (
    <div
      className="absolute bottom-2 right-2 px-3 py-1.5 rounded-md bg-bg-alt/90 border border-border z-10 pointer-events-none"
    >
      {readout}
    </div>
  );
}
