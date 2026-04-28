// Quaternion-based 6DOF flight camera. No damping, no inertia, no smoothing:
// when a key releases, motion stops on the next frame.
//
// Public surface:
//   - `useFlightCamera`      - React hook that wires renderer/camera/scene refs
//                              and returns a handle (or null until refs settle).
//   - `FlightCamHandle`      - imperative handle: resetToScene / focusOnPoint /
//                              dispose / getState / subscribe.
//   - Pure helpers           - `getCameraDirections`, `rotateCameraLocal`,
//                              `applyOrbitFromTarget`, `applyOrbitAroundTarget`,
//                              `syncOrbitTargetToView`, `clampFov`,
//                              `adjustMoveSpeed`, `advanceState`. These are the
//                              testable math; they take and return plain values.
//
// Input mapping (idle WASDQE / IJKL / UO / arrows / [] / Numpad+/-):
//   Translation: W/S forward/back, A/D strafe left/right, E/Q up/down. Each
//   tick advances both `pos` and `orbitTarget` so the orbit pivot follows.
//   Rotation:    I/K pitch up/down, J/L yaw left/right, U/O roll CCW/CW.
//                Local-frame; multiplied onto the current quaternion in
//                yaw -> pitch -> roll order. After rotation, orbitTarget is
//                re-synced to `pos + fwd*orbitDist` so the pivot orb stays
//                screen-centred along the new view direction.
//   Orbit:       Arrow keys swing the camera around `orbitTarget` on a
//                sphere of radius `orbitDist`. Up/Down pitch, Left/Right yaw.
//                The orb stays put in world space; the camera repositions to
//                `orbitTarget - newFwd * orbitDist`.
//   Cart:        ] forward, [ backward along the local fwd axis. Actively
//                shrinks (clamped at 1) / grows orbitDist so the orbit pivot
//                still tracks the camera.
//   FoV:         Numpad+/- step the perspective FoV by one degree per
//                key DOWN event (not continuous). Clamped 10..120.
//
// Mouse on the renderer canvas:
//   Left drag    pan; translates pos and orbitTarget by -right*dx + up*dy.
//   Right drag   orbit; rotates the camera about orbitTarget with pitch=dy,
//                yaw=-dx, then re-anchors pos at orbitTarget - fwd*orbitDist.
//   Middle drag  look; same rotation, no orbit re-anchor; orbitTarget tracks
//                pos + fwd*orbitDist.
//   Wheel        adjusts the WASDQE move-speed multiplier only. Wheel never
//                zooms or dollies the view.
//
// Click-vs-drag is gated by 5px / 350ms thresholds matching the existing
// picker, so the picker keeps firing on relaxed left clicks.

import { useEffect, useState } from "react";
import type React from "react";
import * as THREE from "three";

// ---------- Public types ----------

/** Three projection modes the active camera can be in:
 *   - perspective: the scene's THREE.PerspectiveCamera (FoV-driven).
 *   - orthographic: a parallel-projection camera whose frustum is sized
 *     from the current orbitDist + viewport aspect.
 *   - oblique:     orthographic frustum, then a fixed cabinet-style shear
 *     (45 deg, half-depth) post-multiplied onto the projection matrix. */
export type ProjectionMode = "perspective" | "orthographic" | "oblique";

export const PROJECTION_MODES: readonly ProjectionMode[] = [
  "perspective",
  "orthographic",
  "oblique",
] as const;

/** Named camera presets bound to the Numpad keys. The presets all aim
 *  at the scene centroid; only the camera position differs. Distance is
 *  computed from the scene AABB by `computeFramingDistance` so the model
 *  fills ~`PRESET_FILL_FRACTION` of the viewport. */
export type ViewPreset =
  | "overhead"
  | "perspective2"
  | "side"
  | "fore"
  | "aft"
  | "perspective";

export const VIEW_PRESETS: readonly ViewPreset[] = [
  "overhead",
  "perspective2",
  "side",
  "fore",
  "aft",
  "perspective",
] as const;

export interface FlightCamState {
  pos: THREE.Vector3;
  quat: THREE.Quaternion;
  orbitDist: number;
  /** Multiplier on the per-frame translation/rotation step. */
  moveSpeed: number;
  /** Perspective FoV in degrees, clamped 10..120. */
  fov: number;
  /** Active projection mode. Cycled via P, set via UI. Mode changes are
   *  instant; no animation/damping. */
  projectionMode: ProjectionMode;
}

export interface FlightCamHandle {
  /** Frame the scene into view: position camera at the AABB centroid plus
   *  diagonal/(2*tan(fov/2))*1.1, looking at the centroid. */
  resetToScene(sceneRoot: THREE.Object3D): void;
  /** Look at a specific point at the given distance (default 15). */
  focusOnPoint(target: THREE.Vector3, distance?: number): void;
  /** Detach all listeners and dispose the pivot orb. Called automatically
   *  by the hook on unmount; safe to call manually too. */
  dispose(): void;
  /** Read the current state. The returned object is a snapshot - mutating
   *  it has no effect on the camera. */
  getState(): Readonly<FlightCamState>;
  /** Subscribe to state changes. The listener fires at most once per
   *  animation frame even when many keys / mouse events arrive in the
   *  same tick. Returns an unsubscribe fn. */
  subscribe(listener: (state: Readonly<FlightCamState>) => void): () => void;
  /** Advance projection mode: perspective -> orthographic -> oblique ->
   *  perspective. Bound to the P key and the UI cycle button. */
  cycleProjectionMode(): void;
  /** Set projection mode directly. Bound to the dropdown selector. */
  setProjectionMode(mode: ProjectionMode): void;
  /** Snap to a named preset framing the given sceneRoot. Position is
   *  computed from the scene AABB (centred on the centroid) at a
   *  distance that fills `PRESET_FILL_FRACTION` of the viewport, looking
   *  at the centroid. No damping; the swap is instant. */
  setView(preset: ViewPreset, sceneRoot: THREE.Object3D): void;
  /** Read the camera the renderer / picker should use this frame. Returns
   *  the perspective camera in "perspective" mode and the internal
   *  orthographic camera in "orthographic" or "oblique". The returned
   *  reference is stable across frames; only its matrices update. */
  getActiveCamera(): THREE.Camera;
}

// ---------- Public constants ----------

export const SPEED_MIN = 0.05;
export const SPEED_MAX = 50;
export const SPEED_STEP = 1.15;
export const FOV_MIN = 10;
export const FOV_MAX = 120;

/** Fraction of the viewport's narrowest dimension the framed AABB should
 *  cover. 0.8 = leave ~10% margin on every side of the entity. Shared by
 *  `resetToScene` and `setView` so R and the Numpad keys agree on scale. */
export const PRESET_FILL_FRACTION = 0.8;
/** Fallback distance when the scene AABB is empty or degenerate. */
export const FRAMING_FALLBACK_DISTANCE = 80;

/** Per-frame translation magnitude = PAN_BASE * moveSpeed. */
const PAN_BASE = 0.5;
/** Per-frame rotation magnitude (radians) = ROT_BASE * moveSpeed. */
const ROT_BASE = 0.02;
/** Mouse pan sensitivity (world units per pixel, scaled by FoV-independent factor). */
const MOUSE_PAN_SENS = 0.15;
/** Mouse orbit/look sensitivity (radians per pixel). */
const MOUSE_ROT_SENS = 0.005;
/** Click-vs-drag thresholds, mirrored from the existing picker. */
const CLICK_PIX_THRESHOLD = 5;
const CLICK_TIME_MS = 350;

// ---------- Pure helpers (testable) ----------

/** Local-frame direction vectors for the camera at the given quaternion.
 *  Three.js convention: camera looks down -Z, right is +X, up is +Y. */
export function getCameraDirections(quat: THREE.Quaternion): {
  fwd: THREE.Vector3;
  right: THREE.Vector3;
  up: THREE.Vector3;
} {
  return {
    fwd: new THREE.Vector3(0, 0, -1).applyQuaternion(quat),
    right: new THREE.Vector3(1, 0, 0).applyQuaternion(quat),
    up: new THREE.Vector3(0, 1, 0).applyQuaternion(quat),
  };
}

/** Rotate a quaternion in its own local frame. Order is yaw -> pitch -> roll
 *  (matching the order most users expect: heading, then attitude, then bank).
 *  All deltas are radians; positive pitch tilts the camera down (rotates
 *  about local +X), positive yaw turns left (about local +Y), positive roll
 *  is CCW (about local +Z). */
export function rotateCameraLocal(
  quat: THREE.Quaternion,
  pitchDelta: number,
  yawDelta: number,
  rollDelta: number,
): THREE.Quaternion {
  const out = quat.clone();
  if (yawDelta !== 0) {
    const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), yawDelta);
    out.multiply(q);
  }
  if (pitchDelta !== 0) {
    const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), pitchDelta);
    out.multiply(q);
  }
  if (rollDelta !== 0) {
    const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), rollDelta);
    out.multiply(q);
  }
  out.normalize();
  return out;
}

/** Place the camera at `orbitTarget - fwd * orbitDist` so it looks at the
 *  pivot from `orbitDist` away. The state's quaternion is read but not
 *  modified - call after any rotation update that should re-anchor. */
export function applyOrbitFromTarget(
  state: FlightCamState,
  orbitTarget: THREE.Vector3,
): { pos: THREE.Vector3 } {
  const { fwd } = getCameraDirections(state.quat);
  const pos = orbitTarget.clone().addScaledVector(fwd, -state.orbitDist);
  return { pos };
}

/** Re-anchor the orbit target so it sits at `pos + fwd * orbitDist` for
 *  the given pose. Call this any time the camera rotates (or moves
 *  along a non-orbit-preserving path) to keep the pivot orb visually
 *  centred on the screen along the new look direction. Returns a new
 *  Vector3; does not mutate inputs. */
export function syncOrbitTargetToView(
  pos: THREE.Vector3,
  quat: THREE.Quaternion,
  orbitDist: number,
): THREE.Vector3 {
  const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(quat);
  return pos.clone().addScaledVector(fwd, orbitDist);
}

/** Reposition the camera so it stays at distance `orbitDist` from a
 *  fixed `orbitTarget`, looking along the state's current quaternion.
 *  This is the math behind the arrow-key orbit: the caller rotates
 *  `state.quat` first (changing fwd), then this helper drops `pos` at
 *  `orbitTarget - newFwd * orbitDist`. The orbit target is left
 *  untouched so the pivot orb stays put in world space.
 *
 *  Returns a fresh `{ pos, quat }`; does not mutate inputs. The quat is
 *  passed through unchanged (cloned) for symmetry with other helpers. */
export function applyOrbitAroundTarget(
  state: FlightCamState,
  orbitTarget: THREE.Vector3,
): { pos: THREE.Vector3; quat: THREE.Quaternion } {
  const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(state.quat);
  const pos = orbitTarget.clone().addScaledVector(fwd, -state.orbitDist);
  return { pos, quat: state.quat.clone() };
}

export function clampFov(value: number): number {
  if (value < FOV_MIN) return FOV_MIN;
  if (value > FOV_MAX) return FOV_MAX;
  return value;
}

/** Geometric move-speed adjust. Negative deltaY (scroll up) speeds up by
 *  SPEED_STEP; positive deltaY slows down by the same factor. Clamps to
 *  SPEED_MIN..SPEED_MAX. deltaY === 0 returns `current` unchanged. */
export function adjustMoveSpeed(current: number, wheelDeltaY: number): number {
  if (wheelDeltaY === 0) return current;
  if (wheelDeltaY < 0) return Math.min(SPEED_MAX, current * SPEED_STEP);
  return Math.max(SPEED_MIN, current / SPEED_STEP);
}

/** Compute an orthographic frustum sized to match the apparent scale of
 *  the perspective camera at `orbitDist` units away. The frustum height
 *  is `orbitDist` world-units (so what you see in perspective at the
 *  pivot stays roughly the same scale in ortho); width follows aspect.
 *
 *  Edge cases:
 *  - `orbitDist <= 0` returns a degenerate (zero-size) frustum rather
 *    than crashing. Three.js will produce a singular projection matrix
 *    in that case; callers should clamp orbitDist if they need a finite
 *    image, but a zero-size frustum is preferable to NaN.
 *  - `viewportHeight <= 0` (window minimised, hidden tab) is treated as
 *    aspect = 1 to keep the frustum finite; the renderer can resize once
 *    the viewport reappears.
 */
export function computeOrthoFrustum(
  orbitDist: number,
  viewportWidth: number,
  viewportHeight: number,
): { left: number; right: number; top: number; bottom: number } {
  const aspect = viewportHeight > 0 ? viewportWidth / viewportHeight : 1;
  const halfH = orbitDist * 0.5;
  return {
    top: halfH,
    bottom: -halfH,
    right: halfH * aspect,
    left: -halfH * aspect,
  };
}

/** Cabinet projection shear, post-multiplied onto an orthographic
 *  projection matrix to produce a 45-deg, half-depth oblique view.
 *
 *  Returns a fresh Matrix4 every call. Three.js's `Matrix4.set` takes
 *  args in row-major order even though the underlying storage is
 *  column-major, so this layout reads naturally:
 *
 *      | 1 0 cos(a)*s 0 |
 *      | 0 1 sin(a)*s 0 |
 *      | 0 0    1     0 |
 *      | 0 0    0     1 |
 *
 *  with a = pi/4 and s = 0.5.
 */
export function obliqueCabinetShear(): THREE.Matrix4 {
  const angle = Math.PI / 4;
  const scale = 0.5;
  const m = new THREE.Matrix4();
  m.set(
    1, 0, scale * Math.cos(angle), 0,
    0, 1, scale * Math.sin(angle), 0,
    0, 0, 1, 0,
    0, 0, 0, 1,
  );
  return m;
}

/** Post-multiply the shear matrix onto the projection matrix in place,
 *  matching Three.js convention where `m.multiply(other)` means
 *  `this = this * other`. The mutation is intentional so callers can
 *  apply this directly to `camera.projectionMatrix`; the same matrix
 *  is also returned for chainability. */
export function applyObliqueShear(
  projectionMatrix: THREE.Matrix4,
  shear: THREE.Matrix4,
): THREE.Matrix4 {
  return projectionMatrix.multiply(shear);
}

/** Step through the projection-mode cycle (perspective -> orthographic
 *  -> oblique -> perspective). Pure helper so the cycle order is
 *  testable independently of the hook. */
export function nextProjectionMode(current: ProjectionMode): ProjectionMode {
  const i = PROJECTION_MODES.indexOf(current);
  return PROJECTION_MODES[(i + 1) % PROJECTION_MODES.length];
}

/** Compute the camera distance that frames an AABB at the given
 *  perspective FoV so the entity fills `fillFraction` of the viewport's
 *  narrowest dimension.
 *
 *  The dominant axis is the largest of the three AABB extents; we fit
 *  THAT extent into both the vertical and horizontal half-FoVs and take
 *  the larger of the two distances so neither axis crops. The result is
 *  divided by `fillFraction` to back the camera away an extra step,
 *  leaving a margin (fillFraction = 0.8 => ~10% margin per side).
 *
 *  Edge cases:
 *  - Degenerate AABB (extent_max <= 0) returns FRAMING_FALLBACK_DISTANCE
 *    instead of dividing by zero.
 *  - viewportAspect <= 0 is treated as 1 (square viewport) so we don't
 *    produce a negative or NaN horizontal half-FoV.
 *  - fillFraction <= 0 is treated as PRESET_FILL_FRACTION so a caller
 *    that fat-fingers the argument still gets a finite distance.
 */
export function computeFramingDistance(
  aabbMin: THREE.Vector3,
  aabbMax: THREE.Vector3,
  fovDegrees: number,
  viewportAspect: number,
  fillFraction: number,
): number {
  const dx = aabbMax.x - aabbMin.x;
  const dy = aabbMax.y - aabbMin.y;
  const dz = aabbMax.z - aabbMin.z;
  const extent_max = Math.max(dx, dy, dz);
  if (!Number.isFinite(extent_max) || extent_max <= 0) {
    return FRAMING_FALLBACK_DISTANCE;
  }
  const aspect = viewportAspect > 0 && Number.isFinite(viewportAspect) ? viewportAspect : 1;
  const fill = fillFraction > 0 ? fillFraction : PRESET_FILL_FRACTION;
  const vfov = (fovDegrees * Math.PI) / 180;
  const hfov = 2 * Math.atan(Math.tan(vfov / 2) * aspect);
  const dV = (extent_max / 2) / Math.tan(vfov / 2);
  const dH = (extent_max / 2) / Math.tan(hfov / 2);
  return Math.max(dV, dH) / fill;
}

/** Map a keyboard event `code` to the matching `ViewPreset`. Returns
 *  null for codes that are not bound to a preset. The mapping mirrors
 *  the kaboos-loot Numpad layout: Numpad0 = overhead, Numpad1 =
 *  perspective2, Numpad2 = side, Numpad3 = fore, Numpad4 = aft,
 *  Numpad5 = perspective. Pure helper so the dispatch table is
 *  testable without a DOM. */
export function viewPresetForKeyCode(code: string): ViewPreset | null {
  switch (code) {
    case "Numpad0": return "overhead";
    case "Numpad1": return "perspective2";
    case "Numpad2": return "side";
    case "Numpad3": return "fore";
    case "Numpad4": return "aft";
    case "Numpad5": return "perspective";
    default: return null;
  }
}

/** Top-level dispatch for the SceneViewer keyboard shortcuts that
 *  affect the flight camera or the HUD. Pure (no DOM, no THREE), so
 *  the routing table is unit-testable.
 *
 *  Behaviour:
 *  - `R`           => calls `handle.resetToScene(sceneRoot)`. The
 *                     repeat flag is ignored (held R refreshes).
 *  - `H`           => calls `toggleHud()`. Suppressed on `repeat` so a
 *                     held H does not strobe.
 *  - `Numpad0..5`  => calls `handle.setView(preset, sceneRoot)`.
 *                     Suppressed on `repeat`.
 *  - anything else => returns `false` (caller decides).
 *
 *  Returns `true` if the event matched a binding (caller should call
 *  preventDefault); `false` otherwise. */
export function dispatchViewerHotkey(
  e: { code: string; repeat: boolean },
  handle: Pick<FlightCamHandle, "resetToScene" | "setView"> | null,
  sceneRoot: THREE.Object3D | null,
  toggleHud: () => void,
): boolean {
  if (e.code === "KeyR") {
    if (handle && sceneRoot) handle.resetToScene(sceneRoot);
    return true;
  }
  if (e.code === "KeyH") {
    if (e.repeat) return true;
    toggleHud();
    return true;
  }
  const preset = viewPresetForKeyCode(e.code);
  if (preset) {
    if (e.repeat) return true;
    if (handle && sceneRoot) handle.setView(preset, sceneRoot);
    return true;
  }
  return false;
}

/** World-space camera offset (relative to the look target, in Y-up
 *  Three.js basis) for each named preset. `dist` is the framing distance
 *  computed by `computeFramingDistance`. The returned vector should be
 *  added to the look target to get the camera's world position.
 *
 *  Conventions used here (matching kaboos-loot's setView, then mapped
 *  from kaboos's Z-up basis to Three.js Y-up by swapping y<->z and
 *  flipping signs where needed so visual semantics match):
 *  - overhead: directly above the target, looking down. Camera at
 *    (0, +dist, 0), since up = +Y.
 *  - side:     to the +X side at the target's height.
 *  - fore:     in front (kaboos uses cy - d for "fore"). In Y-up basis
 *    we treat +Z toward the viewer as "fore", so camera at (0, 0, +dist).
 *  - aft:      behind (-Z).
 *  - perspective: 3/4 view above + side, looking at target. Kaboos
 *    placed it at (cx + off, cy - off, cz + off); in Y-up that maps to
 *    (+off, +off, +off) so the camera sits in the +X / +Y / +Z octant.
 *  - perspective2: rotated 90 degrees from perspective; we negate X so
 *    the camera ends up in the -X / +Y / +Z octant.
 *
 *  All of these aim at the (caller-provided) target, so the returned
 *  offset only encodes direction + distance; the look quaternion is
 *  derived from it via Matrix4.lookAt at call time.
 */
export function viewPresetOffset(preset: ViewPreset, dist: number): THREE.Vector3 {
  const off = dist * 0.5;
  switch (preset) {
    case "overhead":
      return new THREE.Vector3(0, dist, 0);
    case "side":
      return new THREE.Vector3(dist, 0, 0);
    case "fore":
      return new THREE.Vector3(0, 0, dist);
    case "aft":
      return new THREE.Vector3(0, 0, -dist);
    case "perspective":
      return new THREE.Vector3(off, off, off);
    case "perspective2":
      return new THREE.Vector3(-off, off, off);
  }
}

/** Per-frame step from a set of held key codes. Returns a NEW state and a
 *  NEW orbitTarget; never mutates the inputs. Pure - no DOM, no THREE.Camera.
 *  This is the load-bearing logic for the keyboard half of the controller. */
export function advanceState(
  state: FlightCamState,
  orbitTarget: THREE.Vector3,
  heldKeys: Set<string>,
): { state: FlightCamState; orbitTarget: THREE.Vector3 } {
  const pan = PAN_BASE * state.moveSpeed;
  const rot = ROT_BASE * state.moveSpeed;

  let pos = state.pos.clone();
  let quat = state.quat.clone();
  let orbitDist = state.orbitDist;
  let target = orbitTarget.clone();
  const dirs = getCameraDirections(quat);

  // Translation: WASDQE - both pos and orbitTarget advance, so the pivot follows.
  if (heldKeys.has("KeyW")) {
    const d = dirs.fwd.clone().multiplyScalar(pan);
    pos.add(d); target.add(d);
  }
  if (heldKeys.has("KeyS")) {
    const d = dirs.fwd.clone().multiplyScalar(-pan);
    pos.add(d); target.add(d);
  }
  if (heldKeys.has("KeyA")) {
    const d = dirs.right.clone().multiplyScalar(-pan);
    pos.add(d); target.add(d);
  }
  if (heldKeys.has("KeyD")) {
    const d = dirs.right.clone().multiplyScalar(pan);
    pos.add(d); target.add(d);
  }
  if (heldKeys.has("KeyE")) {
    const d = dirs.up.clone().multiplyScalar(pan);
    pos.add(d); target.add(d);
  }
  if (heldKeys.has("KeyQ")) {
    const d = dirs.up.clone().multiplyScalar(-pan);
    pos.add(d); target.add(d);
  }

  // Rotation: IJKL pitch/yaw, UO roll. Local-frame. After rotating, we
  // re-anchor orbitTarget below so the pivot orb stays screen-centred
  // along the new look direction.
  let rotatedView = false;
  let pitch = 0, yaw = 0, roll = 0;
  if (heldKeys.has("KeyI")) { pitch -= rot; rotatedView = true; }
  if (heldKeys.has("KeyK")) { pitch += rot; rotatedView = true; }
  if (heldKeys.has("KeyJ")) { yaw   += rot; rotatedView = true; }
  if (heldKeys.has("KeyL")) { yaw   -= rot; rotatedView = true; }
  if (heldKeys.has("KeyU")) { roll  += rot; rotatedView = true; }
  if (heldKeys.has("KeyO")) { roll  -= rot; rotatedView = true; }
  if (rotatedView) {
    quat = rotateCameraLocal(quat, pitch, yaw, roll);
  }

  // Cart: dolly along forward via orbitDist. ] in (clamped at 1), [ out.
  // Pos moves along fwd while orbitDist shrinks/grows by the same amount,
  // so the orb stays put in world space (we re-sync below to be safe).
  let cartChanged = false;
  if (heldKeys.has("BracketRight")) {
    const fwdNow = new THREE.Vector3(0, 0, -1).applyQuaternion(quat);
    pos.add(fwdNow.multiplyScalar(pan));
    orbitDist = Math.max(1, orbitDist - pan);
    cartChanged = true;
  }
  if (heldKeys.has("BracketLeft")) {
    const fwdNow = new THREE.Vector3(0, 0, -1).applyQuaternion(quat);
    pos.add(fwdNow.multiplyScalar(-pan));
    orbitDist = orbitDist + pan;
    cartChanged = true;
  }

  // Re-sync orbitTarget so it tracks the look direction after IJKL/UO
  // rotation or the cart. WASDQE already moves both pos and target by
  // the same delta, so syncing there is a no-op (target == pos + fwd*od
  // already). We still call it unconditionally on view change to keep the
  // invariant `target == pos + fwd*orbitDist` from drifting.
  if (rotatedView || cartChanged) {
    target = syncOrbitTargetToView(pos, quat, orbitDist);
  }

  // Arrow keys: orbit around the (world-fixed) target. The camera quat
  // rotates first; then pos repositions to `target - newFwd*orbitDist`,
  // so the orb stays put in world space and the camera swings around it.
  // Signs: ArrowUp = pitch up (-rot), ArrowLeft = yaw left (+rot).
  // NOTE: the orbit path bypasses the IJKL syncOrbitTargetToView
  // call above by design - target must NOT be re-synced to fwd*orbitDist
  // here, otherwise a held arrow key would precess the orb forward.
  let arrowPitch = 0, arrowYaw = 0;
  let orbited = false;
  if (heldKeys.has("ArrowUp"))    { arrowPitch -= rot; orbited = true; }
  if (heldKeys.has("ArrowDown"))  { arrowPitch += rot; orbited = true; }
  if (heldKeys.has("ArrowLeft"))  { arrowYaw   += rot; orbited = true; }
  if (heldKeys.has("ArrowRight")) { arrowYaw   -= rot; orbited = true; }
  if (orbited) {
    quat = rotateCameraLocal(quat, arrowPitch, arrowYaw, 0);
    const o = applyOrbitAroundTarget(
      {
        pos,
        quat,
        orbitDist,
        moveSpeed: state.moveSpeed,
        fov: state.fov,
        projectionMode: state.projectionMode,
      },
      target,
    );
    pos = o.pos;
  }

  return {
    state: {
      pos,
      quat,
      orbitDist,
      moveSpeed: state.moveSpeed,
      fov: state.fov,
      projectionMode: state.projectionMode,
    },
    orbitTarget: target,
  };
}

// ---------- Hook ----------

interface HookArgs {
  rendererRef: React.RefObject<THREE.WebGLRenderer | null>;
  cameraRef: React.RefObject<THREE.PerspectiveCamera | null>;
  sceneRef: React.RefObject<THREE.Scene | null>;
}

export function useFlightCamera(args: HookArgs): FlightCamHandle | null {
  const { rendererRef, cameraRef, sceneRef } = args;
  const [handle, setHandle] = useState<FlightCamHandle | null>(null);

  useEffect(() => {
    const renderer = rendererRef.current;
    const camera = cameraRef.current;
    const scene = sceneRef.current;
    if (!renderer || !camera || !scene) {
      // Refs are not populated yet on the first effect pass. Bump a tick
      // counter so a follow-up render attaches us. We rely on the parent
      // signalling readiness via a state update that re-renders this
      // component; if the parent never does, the hook stays null.
      return;
    }

    const dom = renderer.domElement;

    // Initial state: identity quaternion at the camera's current position.
    const state: FlightCamState = {
      pos: camera.position.clone(),
      quat: camera.quaternion.clone(),
      orbitDist: 40,
      moveSpeed: 1.0,
      fov: clampFov(camera.fov),
      projectionMode: "perspective",
    };
    const orbitTarget = new THREE.Vector3()
      .copy(state.pos)
      .addScaledVector(new THREE.Vector3(0, 0, -1).applyQuaternion(state.quat), state.orbitDist);
    camera.fov = state.fov;
    camera.updateProjectionMatrix();

    // Internal orthographic camera. Pose is mirrored from the perspective
    // camera every frame; frustum is recomputed from orbitDist + the
    // renderer canvas aspect so the apparent scene scale stays close to
    // the perspective view at the pivot. We share near/far with the
    // perspective camera so culling is consistent across modes.
    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, camera.near, camera.far);
    ortho.position.copy(camera.position);
    ortho.quaternion.copy(camera.quaternion);
    // Pre-built shear matrix; reused every oblique-mode update to avoid
    // allocating a Matrix4 per frame.
    const obliqueShear = obliqueCabinetShear();

    /** Update the active camera's projection-side matrices for `state`. */
    const applyProjection = (): void => {
      const w = renderer.domElement.clientWidth || 1;
      const h = renderer.domElement.clientHeight || 1;
      if (state.projectionMode === "perspective") {
        camera.aspect = w / h;
        camera.fov = state.fov;
        camera.updateProjectionMatrix();
        return;
      }
      // Both ortho and oblique start from the same frustum.
      const f = computeOrthoFrustum(state.orbitDist, w, h);
      ortho.left = f.left;
      ortho.right = f.right;
      ortho.top = f.top;
      ortho.bottom = f.bottom;
      ortho.near = camera.near;
      ortho.far = camera.far;
      ortho.updateProjectionMatrix();
      if (state.projectionMode === "oblique") {
        applyObliqueShear(ortho.projectionMatrix, obliqueShear);
        ortho.projectionMatrixInverse.copy(ortho.projectionMatrix).invert();
      }
    };
    applyProjection();

    // Pivot orb. Rendered on top, depth-test off, scaled with orbitDist.
    const orbGeom = new THREE.SphereGeometry(1, 16, 12);
    const orbMat = new THREE.MeshBasicMaterial({
      color: 0xffaa00,
      transparent: true,
      opacity: 0.55,
      depthTest: false,
      depthWrite: false,
    });
    const orb = new THREE.Mesh(orbGeom, orbMat);
    orb.name = "flight_cam_pivot";
    orb.renderOrder = 1000;
    scene.add(orb);

    // Subscriptions: at most one fire per RAF tick.
    const listeners = new Set<(s: Readonly<FlightCamState>) => void>();
    let dirty = false;
    const markDirty = () => { dirty = true; };

    // ---- Keyboard ----
    const heldKeys = new Set<string>();
    const isTypingTarget = (): boolean => {
      const ae = document.activeElement;
      if (!ae) return false;
      const tag = ae.tagName;
      return tag === "INPUT" || tag === "TEXTAREA";
    };
    const onKeyDown = (e: KeyboardEvent): void => {
      if (isTypingTarget()) return;
      heldKeys.add(e.code);
      // Single-step FoV: process on key down, do not enter the per-frame loop.
      if (e.code === "NumpadAdd") {
        state.fov = clampFov(state.fov + 1);
        camera.fov = state.fov;
        camera.updateProjectionMatrix();
        heldKeys.delete(e.code);
        markDirty();
      } else if (e.code === "NumpadSubtract") {
        state.fov = clampFov(state.fov - 1);
        camera.fov = state.fov;
        camera.updateProjectionMatrix();
        heldKeys.delete(e.code);
        markDirty();
      } else if (e.code === "KeyP") {
        // Single-press cycle. We delete from heldKeys immediately so the
        // per-frame translation loop never sees P (it's not a movement
        // key) and a held P does not re-fire on every frame. Browsers
        // still send key-repeat keydown events though, so a long press
        // will cycle once per OS-repeat-tick rather than once per frame
        // - not double-firing on a single press is what we test for.
        // If `repeat` is true, suppress: only fresh presses cycle.
        if (!e.repeat) {
          state.projectionMode = nextProjectionMode(state.projectionMode);
          applyProjection();
          markDirty();
        }
        heldKeys.delete(e.code);
      }
    };
    const onKeyUp = (e: KeyboardEvent): void => {
      heldKeys.delete(e.code);
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);

    // ---- Mouse ----
    type Btn = "left" | "right" | "mid";
    const buttons: Record<Btn, boolean> = { left: false, right: false, mid: false };
    let lastX = 0, lastY = 0;
    let downX = 0, downY = 0, downT = 0;
    let dragged = false;

    const onPointerDown = (e: PointerEvent): void => {
      if (e.button === 0) buttons.left = true;
      else if (e.button === 1) { buttons.mid = true; e.preventDefault(); }
      else if (e.button === 2) buttons.right = true;
      lastX = e.clientX; lastY = e.clientY;
      downX = e.clientX; downY = e.clientY;
      downT = performance.now();
      dragged = false;
    };
    // Up listens on window so a release outside the canvas still clears the button.
    const onPointerUp = (e: PointerEvent): void => {
      if (e.button === 0) buttons.left = false;
      else if (e.button === 1) buttons.mid = false;
      else if (e.button === 2) buttons.right = false;
    };
    const onPointerMove = (e: PointerEvent): void => {
      if (!buttons.left && !buttons.right && !buttons.mid) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      lastX = e.clientX;
      lastY = e.clientY;

      // Track click-vs-drag against the original down position. Once we
      // cross the click thresholds we stop fighting the picker; the
      // picker has its own thresholds and will simply not fire.
      if (!dragged) {
        const tdx = e.clientX - downX;
        const tdy = e.clientY - downY;
        const tdt = performance.now() - downT;
        if (tdx * tdx + tdy * tdy > CLICK_PIX_THRESHOLD * CLICK_PIX_THRESHOLD || tdt > CLICK_TIME_MS) {
          dragged = true;
        }
      }
      // Suppress sub-threshold motion entirely so a clean click never nudges the camera.
      if (!dragged) return;

      if (buttons.left) {
        const { right, up } = getCameraDirections(state.quat);
        const dr = right.clone().multiplyScalar(-dx * MOUSE_PAN_SENS);
        const du = up.clone().multiplyScalar(dy * MOUSE_PAN_SENS);
        state.pos.add(dr).add(du);
        orbitTarget.add(dr).add(du);
        markDirty();
      }
      if (buttons.right) {
        // Rotate then re-anchor pos at orbitTarget - fwd*orbitDist.
        state.quat = rotateCameraLocal(state.quat, dy * MOUSE_ROT_SENS, -dx * MOUSE_ROT_SENS, 0);
        const { fwd } = getCameraDirections(state.quat);
        state.pos.copy(orbitTarget).addScaledVector(fwd, -state.orbitDist);
        markDirty();
      }
      if (buttons.mid) {
        // Look without re-anchor; keep orbitTarget locked at pos + fwd*orbitDist.
        state.quat = rotateCameraLocal(state.quat, dy * MOUSE_ROT_SENS, -dx * MOUSE_ROT_SENS, 0);
        const { fwd } = getCameraDirections(state.quat);
        orbitTarget.copy(state.pos).addScaledVector(fwd, state.orbitDist);
        markDirty();
      }
    };
    const onContextMenu = (e: MouseEvent): void => { e.preventDefault(); };
    const onWheel = (e: WheelEvent): void => {
      state.moveSpeed = adjustMoveSpeed(state.moveSpeed, e.deltaY);
      markDirty();
    };

    dom.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("pointerup", onPointerUp);
    dom.addEventListener("pointermove", onPointerMove);
    dom.addEventListener("contextmenu", onContextMenu);
    dom.addEventListener("wheel", onWheel, { passive: true });

    // ---- Per-frame loop ----
    let rafId = 0;
    let disposed = false;
    const tick = (): void => {
      if (disposed) return;
      rafId = requestAnimationFrame(tick);

      if (heldKeys.size > 0) {
        const before = state.pos.clone();
        const beforeQ = state.quat.clone();
        const beforeOd = state.orbitDist;
        const next = advanceState(state, orbitTarget, heldKeys);
        state.pos.copy(next.state.pos);
        state.quat.copy(next.state.quat);
        state.orbitDist = next.state.orbitDist;
        orbitTarget.copy(next.orbitTarget);
        if (
          !before.equals(state.pos) ||
          !beforeQ.equals(state.quat) ||
          beforeOd !== state.orbitDist
        ) {
          markDirty();
        }
      }

      // Push the latest state to the camera every frame; cheap, and keeps
      // us in sync if anything else nudged the THREE.Camera in between.
      camera.position.copy(state.pos);
      camera.quaternion.copy(state.quat);
      // Mirror pose into the ortho camera so a switch to ortho/oblique
      // is seamless. Cheap (vec3 + quat copy) and avoids a one-frame
      // pop on cycle.
      ortho.position.copy(state.pos);
      ortho.quaternion.copy(state.quat);

      // Refresh the active camera's projection. In perspective mode this
      // is a no-op vs. what's already on the camera unless the canvas
      // resized; in ortho/oblique it tracks orbitDist as the user
      // dollies in/out via [/].
      if (state.projectionMode !== "perspective") {
        applyProjection();
      }

      // Scale the pivot orb with orbitDist; clamp the floor so it stays
      // visible at point-blank dolly-in.
      const r = Math.max(0.1, state.orbitDist * 0.01);
      orb.position.copy(orbitTarget);
      orb.scale.setScalar(r);

      if (dirty && listeners.size > 0) {
        const snapshot: Readonly<FlightCamState> = {
          pos: state.pos.clone(),
          quat: state.quat.clone(),
          orbitDist: state.orbitDist,
          moveSpeed: state.moveSpeed,
          fov: state.fov,
          projectionMode: state.projectionMode,
        };
        for (const l of listeners) l(snapshot);
      }
      dirty = false;
    };
    rafId = requestAnimationFrame(tick);

    // ---- Handle ----
    const h: FlightCamHandle = {
      resetToScene(sceneRoot: THREE.Object3D): void {
        sceneRoot.updateMatrixWorld(true);
        const box = new THREE.Box3().setFromObject(sceneRoot);
        // Orbit target is fixed at world origin (per the project's view
        // convention for ship exports, which are emitted centred near
        // origin). The framing distance is sized from the AABB so the
        // entity fills PRESET_FILL_FRACTION of the viewport.
        const target = new THREE.Vector3(0, 0, 0);
        let dist: number;
        if (box.isEmpty()) {
          dist = FRAMING_FALLBACK_DISTANCE;
        } else {
          const w = renderer.domElement.clientWidth || 1;
          const h = renderer.domElement.clientHeight || 1;
          dist = computeFramingDistance(
            box.min,
            box.max,
            state.fov,
            w / h,
            PRESET_FILL_FRACTION,
          );
        }
        // Default vantage: 3/4 view in the +X / +Y / +Z octant, looking
        // back at the origin. Direction is normalised then scaled by
        // dist so the camera's distance to target equals dist.
        const dir = new THREE.Vector3(1, 0.6, 1).normalize();
        state.pos.copy(target).addScaledVector(dir, dist);
        const m = new THREE.Matrix4();
        m.lookAt(state.pos, target, new THREE.Vector3(0, 1, 0));
        state.quat.setFromRotationMatrix(m);
        state.orbitDist = state.pos.distanceTo(target);
        orbitTarget.copy(target);
        // Also tighten near/far so the depth precision matches the ship size.
        if (Number.isFinite(dist) && dist > 0) {
          camera.near = Math.max(dist / 1000, 0.05);
          camera.far = dist * 100;
          camera.updateProjectionMatrix();
        }
        markDirty();
      },
      focusOnPoint(target: THREE.Vector3, distance?: number): void {
        const dist = distance ?? 15;
        const { fwd } = getCameraDirections(state.quat);
        state.pos.copy(target).addScaledVector(fwd, -dist);
        const m = new THREE.Matrix4();
        m.lookAt(state.pos, target, new THREE.Vector3(0, 1, 0));
        state.quat.setFromRotationMatrix(m);
        state.orbitDist = dist;
        orbitTarget.copy(target);
        markDirty();
      },
      dispose(): void {
        if (disposed) return;
        disposed = true;
        cancelAnimationFrame(rafId);
        window.removeEventListener("keydown", onKeyDown);
        window.removeEventListener("keyup", onKeyUp);
        window.removeEventListener("pointerup", onPointerUp);
        dom.removeEventListener("pointerdown", onPointerDown);
        dom.removeEventListener("pointermove", onPointerMove);
        dom.removeEventListener("contextmenu", onContextMenu);
        dom.removeEventListener("wheel", onWheel);
        scene.remove(orb);
        orbGeom.dispose();
        orbMat.dispose();
        listeners.clear();
      },
      getState(): Readonly<FlightCamState> {
        return {
          pos: state.pos.clone(),
          quat: state.quat.clone(),
          orbitDist: state.orbitDist,
          moveSpeed: state.moveSpeed,
          fov: state.fov,
          projectionMode: state.projectionMode,
        };
      },
      subscribe(listener): () => void {
        listeners.add(listener);
        // Prime the listener so it can render an initial readout.
        listener({
          pos: state.pos.clone(),
          quat: state.quat.clone(),
          orbitDist: state.orbitDist,
          moveSpeed: state.moveSpeed,
          fov: state.fov,
          projectionMode: state.projectionMode,
        });
        return () => listeners.delete(listener);
      },
      cycleProjectionMode(): void {
        state.projectionMode = nextProjectionMode(state.projectionMode);
        applyProjection();
        markDirty();
      },
      setProjectionMode(mode: ProjectionMode): void {
        if (state.projectionMode === mode) return;
        state.projectionMode = mode;
        applyProjection();
        markDirty();
      },
      setView(preset: ViewPreset, sceneRoot: THREE.Object3D): void {
        sceneRoot.updateMatrixWorld(true);
        const box = new THREE.Box3().setFromObject(sceneRoot);
        const target = box.isEmpty()
          ? new THREE.Vector3(0, 0, 0)
          : box.getCenter(new THREE.Vector3());
        let dist: number;
        if (box.isEmpty()) {
          dist = FRAMING_FALLBACK_DISTANCE;
        } else {
          const w = renderer.domElement.clientWidth || 1;
          const h = renderer.domElement.clientHeight || 1;
          dist = computeFramingDistance(
            box.min,
            box.max,
            state.fov,
            w / h,
            PRESET_FILL_FRACTION,
          );
        }
        const offset = viewPresetOffset(preset, dist);
        state.pos.copy(target).add(offset);
        // Three's lookAt picks an arbitrary basis when fwd is parallel to
        // the world-up vector (overhead view). For the overhead preset we
        // use +Z as up so the camera does not snap to a singular basis.
        const worldUp =
          preset === "overhead"
            ? new THREE.Vector3(0, 0, -1)
            : new THREE.Vector3(0, 1, 0);
        const m = new THREE.Matrix4();
        m.lookAt(state.pos, target, worldUp);
        state.quat.setFromRotationMatrix(m);
        state.orbitDist = state.pos.distanceTo(target);
        orbitTarget.copy(target);
        if (Number.isFinite(dist) && dist > 0) {
          camera.near = Math.max(dist / 1000, 0.05);
          camera.far = dist * 100;
          camera.updateProjectionMatrix();
        }
        markDirty();
      },
      getActiveCamera(): THREE.Camera {
        return state.projectionMode === "perspective" ? camera : ortho;
      },
    };
    setHandle(h);

    return () => {
      h.dispose();
      setHandle(null);
    };
    // Refs are populated by the parent's bootstrap effect, which runs
    // before this one (effects fire top-down inside one component). After
    // mount the refs do not change identity, so an empty dep array is safe.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return handle;
}
