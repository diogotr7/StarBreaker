// Pure-math coverage for the flight-camera helpers. The hook itself is
// excluded - it talks to a live renderer and DOM, neither of which are
// healthy to spin up in a Vitest unit run. The math below is the load-
// bearing logic the hook depends on; if these are right, the hook only
// has DOM wiring to get wrong.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  adjustMoveSpeed,
  advanceState,
  applyOrbitAroundTarget,
  applyOrbitFromTarget,
  clampFov,
  computeFramingDistance,
  dispatchViewerHotkey,
  FOV_MAX,
  FOV_MIN,
  FRAMING_FALLBACK_DISTANCE,
  getCameraDirections,
  PRESET_FILL_FRACTION,
  rotateCameraLocal,
  SPEED_MAX,
  SPEED_MIN,
  SPEED_STEP,
  syncOrbitTargetToView,
  viewPresetForKeyCode,
  viewPresetOffset,
  type FlightCamHandle,
  type FlightCamState,
  type ViewPreset,
} from "../flight-camera";

const EPS = 1e-6;

function vecCloseTo(a: THREE.Vector3, b: THREE.Vector3, eps = EPS): void {
  expect(a.x).toBeCloseTo(b.x, 5);
  expect(a.y).toBeCloseTo(b.y, 5);
  expect(a.z).toBeCloseTo(b.z, 5);
  void eps;
}

function makeState(overrides: Partial<FlightCamState> = {}): FlightCamState {
  return {
    pos: new THREE.Vector3(0, 0, 0),
    quat: new THREE.Quaternion(),
    orbitDist: 10,
    moveSpeed: 1.0,
    fov: 60,
    projectionMode: "perspective",
    ...overrides,
  };
}

describe("getCameraDirections", () => {
  it("returns orthonormal fwd/right/up for identity quat", () => {
    const dirs = getCameraDirections(new THREE.Quaternion());
    vecCloseTo(dirs.fwd, new THREE.Vector3(0, 0, -1));
    vecCloseTo(dirs.right, new THREE.Vector3(1, 0, 0));
    vecCloseTo(dirs.up, new THREE.Vector3(0, 1, 0));

    // Pairwise orthogonal
    expect(dirs.fwd.dot(dirs.right)).toBeCloseTo(0, 5);
    expect(dirs.fwd.dot(dirs.up)).toBeCloseTo(0, 5);
    expect(dirs.right.dot(dirs.up)).toBeCloseTo(0, 5);

    // Unit length
    expect(dirs.fwd.length()).toBeCloseTo(1, 5);
    expect(dirs.right.length()).toBeCloseTo(1, 5);
    expect(dirs.up.length()).toBeCloseTo(1, 5);
  });

  it("rotates fwd/right consistently for a +90deg yaw quat", () => {
    // Three.js is right-handed: rotating about +Y by +90deg maps
    //   (1,0,0) -> (0,0,-1)
    //   (0,0,-1) -> (-1,0,0)
    // So the fwd vector swings to old -right (i.e. -X), and right swings
    // to old fwd (-Z). Up is unchanged.
    const yaw = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      Math.PI / 2,
    );
    const dirs = getCameraDirections(yaw);
    vecCloseTo(dirs.fwd, new THREE.Vector3(-1, 0, 0));
    vecCloseTo(dirs.right, new THREE.Vector3(0, 0, -1));
    vecCloseTo(dirs.up, new THREE.Vector3(0, 1, 0));
  });

  it("yaws fwd to +X for a -90deg yaw quat", () => {
    // Symmetric of the above with negated angle: fwd (-Z) goes to +X.
    const yaw = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      -Math.PI / 2,
    );
    const dirs = getCameraDirections(yaw);
    vecCloseTo(dirs.fwd, new THREE.Vector3(1, 0, 0));
  });
});

describe("rotateCameraLocal", () => {
  it("returns an identity-equivalent quat when all deltas are zero", () => {
    const q0 = new THREE.Quaternion(0.1, 0.2, 0.3, 0.9).normalize();
    const q1 = rotateCameraLocal(q0, 0, 0, 0);
    expect(q1.x).toBeCloseTo(q0.x, 5);
    expect(q1.y).toBeCloseTo(q0.y, 5);
    expect(q1.z).toBeCloseTo(q0.z, 5);
    expect(q1.w).toBeCloseTo(q0.w, 5);
  });

  it("does not mutate the input quaternion", () => {
    const q0 = new THREE.Quaternion();
    const before = q0.clone();
    rotateCameraLocal(q0, 0.5, 0.5, 0.5);
    expect(q0.equals(before)).toBe(true);
  });

  it("is order-dependent: yaw-then-pitch != pitch-then-yaw", () => {
    const id = new THREE.Quaternion();

    const yawFirst = rotateCameraLocal(id, 0, Math.PI / 2, 0);
    const yawThenPitch = rotateCameraLocal(yawFirst, Math.PI / 2, 0, 0);
    const fwdA = new THREE.Vector3(0, 0, -1).applyQuaternion(yawThenPitch);

    const pitchFirst = rotateCameraLocal(id, Math.PI / 2, 0, 0);
    const pitchThenYaw = rotateCameraLocal(pitchFirst, 0, Math.PI / 2, 0);
    const fwdB = new THREE.Vector3(0, 0, -1).applyQuaternion(pitchThenYaw);

    // The two orderings should produce visibly different final forwards.
    const diff = fwdA.distanceTo(fwdB);
    expect(diff).toBeGreaterThan(0.5);
  });

  it("rotates fwd correctly for a single-axis yaw applied to identity", () => {
    // +Y rotation by +90deg in a right-handed system takes fwd (-Z) to -X.
    const q = rotateCameraLocal(new THREE.Quaternion(), 0, Math.PI / 2, 0);
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(q);
    vecCloseTo(fwd, new THREE.Vector3(-1, 0, 0));
  });

  it("rotates fwd correctly for a single-axis pitch applied to identity", () => {
    // Positive pitch rotates about local +X, which tilts -Z (fwd) toward +Y (up).
    // Wait: rotating (0,0,-1) about (+1,0,0) by +90deg: right-hand rule, thumb
    // along +X, fingers curl from +Y to +Z. So +Y -> +Z, -Z -> +Y. Therefore
    // fwd (-Z) goes to +Y.
    const q = rotateCameraLocal(new THREE.Quaternion(), Math.PI / 2, 0, 0);
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(q);
    vecCloseTo(fwd, new THREE.Vector3(0, 1, 0));
  });
});

describe("applyOrbitFromTarget", () => {
  it("places camera at target - fwd * orbitDist for identity quat", () => {
    const state = makeState({ orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = applyOrbitFromTarget(state, target);
    // fwd = -Z, so pos = target - (-Z)*10 = +Z*10.
    vecCloseTo(out.pos, new THREE.Vector3(0, 0, 10));
  });

  it("places camera correctly for a non-zero target and a yawed quat", () => {
    const yaw = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      Math.PI / 2,
    );
    const state = makeState({ orbitDist: 5, quat: yaw });
    const target = new THREE.Vector3(10, 20, 30);
    const out = applyOrbitFromTarget(state, target);
    // fwd after +PI/2 yaw = -X (right-handed). pos = target - fwd*orbitDist
    // = (10,20,30) - (-X)*5 = (10,20,30) + (5,0,0) = (15,20,30).
    vecCloseTo(out.pos, new THREE.Vector3(15, 20, 30));
  });

  it("does not mutate the input state.pos or the orbitTarget", () => {
    const state = makeState();
    const posBefore = state.pos.clone();
    const target = new THREE.Vector3(1, 2, 3);
    const targetBefore = target.clone();
    applyOrbitFromTarget(state, target);
    expect(state.pos.equals(posBefore)).toBe(true);
    expect(target.equals(targetBefore)).toBe(true);
  });
});

describe("clampFov", () => {
  it("clamps below FOV_MIN", () => {
    expect(clampFov(5)).toBe(FOV_MIN);
    expect(clampFov(-100)).toBe(FOV_MIN);
    expect(clampFov(0)).toBe(FOV_MIN);
  });

  it("clamps above FOV_MAX", () => {
    expect(clampFov(200)).toBe(FOV_MAX);
    expect(clampFov(121)).toBe(FOV_MAX);
  });

  it("passes values in [FOV_MIN, FOV_MAX] unchanged", () => {
    expect(clampFov(FOV_MIN)).toBe(FOV_MIN);
    expect(clampFov(FOV_MAX)).toBe(FOV_MAX);
    expect(clampFov(60)).toBe(60);
    expect(clampFov(45)).toBe(45);
  });
});

describe("adjustMoveSpeed", () => {
  it("increases on negative deltaY", () => {
    expect(adjustMoveSpeed(1.0, -100)).toBeCloseTo(1.0 * SPEED_STEP, 5);
    expect(adjustMoveSpeed(2.0, -1)).toBeCloseTo(2.0 * SPEED_STEP, 5);
  });

  it("decreases on positive deltaY", () => {
    expect(adjustMoveSpeed(1.0, 100)).toBeCloseTo(1.0 / SPEED_STEP, 5);
    expect(adjustMoveSpeed(2.0, 1)).toBeCloseTo(2.0 / SPEED_STEP, 5);
  });

  it("clamps at SPEED_MIN on the way down", () => {
    let s = SPEED_MIN;
    for (let i = 0; i < 20; i++) s = adjustMoveSpeed(s, 1);
    expect(s).toBe(SPEED_MIN);
  });

  it("clamps at SPEED_MAX on the way up", () => {
    let s = SPEED_MAX;
    for (let i = 0; i < 20; i++) s = adjustMoveSpeed(s, -1);
    expect(s).toBe(SPEED_MAX);
  });

  it("is geometric (factor SPEED_STEP), not additive", () => {
    // Magnitude of deltaY does not matter, only sign.
    expect(adjustMoveSpeed(1.0, -1)).toBeCloseTo(adjustMoveSpeed(1.0, -10000), 5);
    expect(adjustMoveSpeed(1.0, 1)).toBeCloseTo(adjustMoveSpeed(1.0, 10000), 5);
    // A "factor 1.15" check: ratio is exactly SPEED_STEP irrespective of input.
    const s1 = adjustMoveSpeed(0.5, -100);
    expect(s1 / 0.5).toBeCloseTo(SPEED_STEP, 5);
    const s2 = adjustMoveSpeed(8.0, -100);
    expect(s2 / 8.0).toBeCloseTo(SPEED_STEP, 5);
  });

  it("returns current unchanged for deltaY === 0", () => {
    expect(adjustMoveSpeed(1.5, 0)).toBe(1.5);
  });
});

describe("advanceState", () => {
  it("does nothing when no keys are held", () => {
    const state = makeState();
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set());
    vecCloseTo(out.state.pos, state.pos);
    vecCloseTo(out.orbitTarget, target);
    expect(out.state.quat.equals(state.quat)).toBe(true);
    expect(out.state.orbitDist).toBe(state.orbitDist);
  });

  it("advances pos along fwd for KeyW", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["KeyW"]));
    // fwd = -Z, pan = 0.5 * 1.0, so pos = (0, 0, -0.5).
    vecCloseTo(out.state.pos, new THREE.Vector3(0, 0, -0.5));
    // orbitTarget tracks the camera.
    vecCloseTo(out.orbitTarget, new THREE.Vector3(0, 0, -0.5));
  });

  it("advances pos along -fwd for KeyS", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["KeyS"]));
    vecCloseTo(out.state.pos, new THREE.Vector3(0, 0, 0.5));
  });

  it("strafes along right for KeyD and -right for KeyA", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const right = advanceState(state, target, new Set(["KeyD"]));
    vecCloseTo(right.state.pos, new THREE.Vector3(0.5, 0, 0));
    const left = advanceState(state, target, new Set(["KeyA"]));
    vecCloseTo(left.state.pos, new THREE.Vector3(-0.5, 0, 0));
  });

  it("rises for KeyE and falls for KeyQ", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const e = advanceState(state, target, new Set(["KeyE"]));
    vecCloseTo(e.state.pos, new THREE.Vector3(0, 0.5, 0));
    const q = advanceState(state, target, new Set(["KeyQ"]));
    vecCloseTo(q.state.pos, new THREE.Vector3(0, -0.5, 0));
  });

  it("composes WASDQE: holding W and D advances along fwd + right", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["KeyW", "KeyD"]));
    // fwd*0.5 + right*0.5 = (0.5, 0, -0.5).
    vecCloseTo(out.state.pos, new THREE.Vector3(0.5, 0, -0.5));
    vecCloseTo(out.orbitTarget, new THREE.Vector3(0.5, 0, -0.5));
  });

  it("scales translation step with moveSpeed", () => {
    const state = makeState({ moveSpeed: 2.5 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["KeyW"]));
    vecCloseTo(out.state.pos, new THREE.Vector3(0, 0, -1.25));
  });

  it("rotates quat for IJKL and changes fwd accordingly", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const before = new THREE.Vector3(0, 0, -1).applyQuaternion(state.quat);
    const out = advanceState(state, target, new Set(["KeyJ"])); // yaw left
    const after = new THREE.Vector3(0, 0, -1).applyQuaternion(out.state.quat);
    // The forward should have rotated, so the two should differ noticeably.
    expect(before.distanceTo(after)).toBeGreaterThan(0);
    // Pos should be unchanged (no translation key held).
    vecCloseTo(out.state.pos, state.pos);
  });

  it("BracketRight dollies in, shrinks orbitDist, clamps at 1", () => {
    const state = makeState({ orbitDist: 1.2, moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["BracketRight"]));
    // pan = 0.5; orbitDist = max(1, 1.2 - 0.5) = max(1, 0.7) = 1.
    expect(out.state.orbitDist).toBe(1);
    // pos moves along fwd by 0.5: (0, 0, -0.5).
    vecCloseTo(out.state.pos, new THREE.Vector3(0, 0, -0.5));
  });

  it("BracketLeft dollies out, grows orbitDist", () => {
    const state = makeState({ orbitDist: 10, moveSpeed: 1.0 });
    const target = new THREE.Vector3(0, 0, 0);
    const out = advanceState(state, target, new Set(["BracketLeft"]));
    expect(out.state.orbitDist).toBeCloseTo(10.5, 5);
    // pos moves along -fwd by 0.5: (0, 0, +0.5).
    vecCloseTo(out.state.pos, new THREE.Vector3(0, 0, 0.5));
  });

  it("returns a NEW state object, leaving the input untouched", () => {
    const state = makeState({ moveSpeed: 1.0 });
    const posBefore = state.pos.clone();
    const target = new THREE.Vector3(0, 0, 0);
    const targetBefore = target.clone();
    advanceState(state, target, new Set(["KeyW", "KeyD"]));
    expect(state.pos.equals(posBefore)).toBe(true);
    expect(target.equals(targetBefore)).toBe(true);
  });
});

describe("syncOrbitTargetToView", () => {
  it("returns pos + fwd*orbitDist for identity quat", () => {
    const pos = new THREE.Vector3(0, 0, 0);
    const quat = new THREE.Quaternion();
    const out = syncOrbitTargetToView(pos, quat, 10);
    // fwd = -Z, so target = (0, 0, 0) + (-Z)*10 = (0, 0, -10).
    vecCloseTo(out, new THREE.Vector3(0, 0, -10));
  });

  it("moves target to the new fwd direction after a yaw", () => {
    const pos = new THREE.Vector3(0, 0, 0);
    // +90deg yaw: fwd swings from -Z to -X.
    const quat = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      Math.PI / 2,
    );
    const out = syncOrbitTargetToView(pos, quat, 5);
    // target = pos + fwd*orbitDist = (0,0,0) + (-1,0,0)*5 = (-5, 0, 0).
    vecCloseTo(out, new THREE.Vector3(-5, 0, 0));
  });

  it("does not mutate the input pos or quat", () => {
    const pos = new THREE.Vector3(1, 2, 3);
    const posBefore = pos.clone();
    const quat = new THREE.Quaternion(0.1, 0.2, 0.3, 0.9).normalize();
    const quatBefore = quat.clone();
    syncOrbitTargetToView(pos, quat, 7);
    expect(pos.equals(posBefore)).toBe(true);
    expect(quat.equals(quatBefore)).toBe(true);
  });

  it("places target at distance orbitDist from pos along fwd", () => {
    // Property: || target - pos || == orbitDist for any quat.
    const quat = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(1, 1, 0).normalize(),
      0.7,
    );
    const pos = new THREE.Vector3(4, -2, 9);
    const dist = 12.5;
    const target = syncOrbitTargetToView(pos, quat, dist);
    expect(target.distanceTo(pos)).toBeCloseTo(dist, 5);
  });
});

describe("applyOrbitAroundTarget", () => {
  it("places camera at orbitTarget - fwd*orbitDist for identity quat", () => {
    const state = makeState({
      pos: new THREE.Vector3(99, 99, 99), // start somewhere arbitrary
      orbitDist: 10,
    });
    const target = new THREE.Vector3(0, 0, 0);
    const out = applyOrbitAroundTarget(state, target);
    // fwd = -Z, so pos = target - fwd*10 = (0,0,0) - (0,0,-10) = (0,0,10).
    vecCloseTo(out.pos, new THREE.Vector3(0, 0, 10));
  });

  it("places camera at distance orbitDist from target after yaw", () => {
    // Property: after rotating quat, pos must still sit at orbitDist from
    // target along the new fwd.
    const yaw = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      Math.PI / 2,
    );
    const state = makeState({ orbitDist: 8, quat: yaw });
    const target = new THREE.Vector3(10, 0, 0);
    const out = applyOrbitAroundTarget(state, target);
    expect(out.pos.distanceTo(target)).toBeCloseTo(8, 5);
    // After +90deg yaw, fwd = -X. pos = target - fwd*8 = (10,0,0) - (-8,0,0) = (18,0,0).
    vecCloseTo(out.pos, new THREE.Vector3(18, 0, 0));
  });

  it("does not mutate the input state.pos / state.quat / orbitTarget", () => {
    const state = makeState({ pos: new THREE.Vector3(1, 2, 3) });
    const posBefore = state.pos.clone();
    const quatBefore = state.quat.clone();
    const target = new THREE.Vector3(5, 6, 7);
    const targetBefore = target.clone();
    applyOrbitAroundTarget(state, target);
    expect(state.pos.equals(posBefore)).toBe(true);
    expect(state.quat.equals(quatBefore)).toBe(true);
    expect(target.equals(targetBefore)).toBe(true);
  });
});

describe("advanceState - orbit-target tracking after rotation", () => {
  it("KeyJ (yaw) moves orbitTarget so it stays at pos + fwd*orbitDist", () => {
    // Regression test for the original pivot-orb-doesn't-track bug. Yawing
    // the camera with IJKL must reposition the orb in world space so it
    // remains screen-centred along the new look direction.
    const state = makeState({ moveSpeed: 1.0, orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, -10); // initial: pos=origin, fwd=-Z
    const out = advanceState(state, target, new Set(["KeyJ"]));
    // After yaw, target should NOT equal the original (0,0,-10).
    expect(out.orbitTarget.equals(target)).toBe(false);
    // It must satisfy target == pos + fwd*orbitDist for the new quat.
    const newFwd = new THREE.Vector3(0, 0, -1).applyQuaternion(out.state.quat);
    const expected = out.state.pos
      .clone()
      .addScaledVector(newFwd, out.state.orbitDist);
    vecCloseTo(out.orbitTarget, expected);
  });

  it("KeyI (pitch) updates orbitTarget to track the new fwd", () => {
    const state = makeState({ moveSpeed: 1.0, orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, -10);
    const out = advanceState(state, target, new Set(["KeyI"]));
    // Pitch up (KeyI) tilts fwd from -Z toward +Y, so target should shift.
    expect(out.orbitTarget.equals(target)).toBe(false);
    const newFwd = new THREE.Vector3(0, 0, -1).applyQuaternion(out.state.quat);
    const expected = out.state.pos
      .clone()
      .addScaledVector(newFwd, out.state.orbitDist);
    vecCloseTo(out.orbitTarget, expected);
  });

  it("KeyW (translation) keeps target tracking pos+fwd*orbitDist", () => {
    // Pure translation: both pos and target advance together; the
    // invariant should still hold afterward.
    const state = makeState({ moveSpeed: 1.0, orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, -10);
    const out = advanceState(state, target, new Set(["KeyW"]));
    const newFwd = new THREE.Vector3(0, 0, -1).applyQuaternion(out.state.quat);
    const expected = out.state.pos
      .clone()
      .addScaledVector(newFwd, out.state.orbitDist);
    vecCloseTo(out.orbitTarget, expected);
  });

  it("BracketRight (cart in) keeps target tracking pos+fwd*orbitDist", () => {
    const state = makeState({ moveSpeed: 1.0, orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, -10);
    const out = advanceState(state, target, new Set(["BracketRight"]));
    const newFwd = new THREE.Vector3(0, 0, -1).applyQuaternion(out.state.quat);
    const expected = out.state.pos
      .clone()
      .addScaledVector(newFwd, out.state.orbitDist);
    vecCloseTo(out.orbitTarget, expected);
    // Sanity: orbitDist shrank.
    expect(out.state.orbitDist).toBeCloseTo(9.5, 5);
  });
});

describe("advanceState - arrow-key orbit", () => {
  it("ArrowLeft rotates quat AND keeps pos at distance orbitDist from target", () => {
    const orbitDist = 10;
    const state = makeState({ moveSpeed: 1.0, orbitDist });
    // Camera starts at origin looking -Z, target at (0,0,-10).
    const target = new THREE.Vector3(0, 0, -10);
    const out = advanceState(state, target, new Set(["ArrowLeft"]));
    // Quat should have changed (yaw).
    expect(out.state.quat.equals(state.quat)).toBe(false);
    // Pos must now sit at orbitDist from target.
    expect(out.state.pos.distanceTo(out.orbitTarget)).toBeCloseTo(orbitDist, 5);
    // Target must NOT have moved (orbit-mode invariant).
    vecCloseTo(out.orbitTarget, target);
  });

  it("ArrowRight orbits the opposite way; pos moves to a different angle", () => {
    const orbitDist = 10;
    const state = makeState({ moveSpeed: 1.0, orbitDist });
    const target = new THREE.Vector3(0, 0, -10);
    const left = advanceState(state, target, new Set(["ArrowLeft"]));
    const right = advanceState(state, target, new Set(["ArrowRight"]));
    // Both should orbit on the sphere - distance to target == orbitDist.
    expect(left.state.pos.distanceTo(left.orbitTarget)).toBeCloseTo(orbitDist, 5);
    expect(right.state.pos.distanceTo(right.orbitTarget)).toBeCloseTo(orbitDist, 5);
    // And they should land in different places.
    expect(left.state.pos.distanceTo(right.state.pos)).toBeGreaterThan(0);
    // Both leave the target untouched.
    vecCloseTo(left.orbitTarget, target);
    vecCloseTo(right.orbitTarget, target);
  });

  it("ArrowUp pitches up; pos sits on the orbit sphere", () => {
    const orbitDist = 10;
    const state = makeState({ moveSpeed: 1.0, orbitDist });
    const target = new THREE.Vector3(0, 0, -10);
    const out = advanceState(state, target, new Set(["ArrowUp"]));
    expect(out.state.quat.equals(state.quat)).toBe(false);
    expect(out.state.pos.distanceTo(out.orbitTarget)).toBeCloseTo(orbitDist, 5);
    vecCloseTo(out.orbitTarget, target);
    // ArrowUp = pitch up = -ROT pitch sign convention. Camera was at
    // origin looking -Z; pitching up should swing pos in +Y/-Z direction
    // (camera goes "down and behind" the target as it pitches up).
    // We just sanity-check that y went one direction or the other - the
    // exact sign is locked in by the ArrowUp = -rot pitch convention.
    expect(Math.abs(out.state.pos.y)).toBeGreaterThan(0);
  });

  it("ArrowDown is mirror-image of ArrowUp (pitch down)", () => {
    const state = makeState({ moveSpeed: 1.0, orbitDist: 10 });
    const target = new THREE.Vector3(0, 0, -10);
    const up = advanceState(state, target, new Set(["ArrowUp"]));
    const down = advanceState(state, target, new Set(["ArrowDown"]));
    // ArrowUp and ArrowDown should produce mirror-image positions about
    // the y=0 plane (within numerical tolerance) when starting from
    // identity quat with target at -Z.
    expect(up.state.pos.y).toBeCloseTo(-down.state.pos.y, 5);
  });
});

describe("constants", () => {
  it("has SPEED_MIN < SPEED_MAX and SPEED_STEP > 1", () => {
    expect(SPEED_MIN).toBeLessThan(SPEED_MAX);
    expect(SPEED_STEP).toBeGreaterThan(1);
  });
  it("has FOV_MIN < FOV_MAX in valid camera range", () => {
    expect(FOV_MIN).toBeGreaterThan(0);
    expect(FOV_MAX).toBeLessThanOrEqual(180);
    expect(FOV_MIN).toBeLessThan(FOV_MAX);
  });
  it("has PRESET_FILL_FRACTION in (0, 1]", () => {
    expect(PRESET_FILL_FRACTION).toBeGreaterThan(0);
    expect(PRESET_FILL_FRACTION).toBeLessThanOrEqual(1);
  });
  it("has FRAMING_FALLBACK_DISTANCE > 0", () => {
    expect(FRAMING_FALLBACK_DISTANCE).toBeGreaterThan(0);
  });
});

describe("computeFramingDistance", () => {
  it("frames a 100-unit cube at 45deg FoV / 16:9 / 80% fill at the expected distance", () => {
    // extent_max = 100. vfov = 45deg. dV = 50 / tan(22.5deg).
    // hfov  = 2 * atan(tan(22.5deg) * 16/9). dH = 50 / tan(hfov/2).
    // For aspect > 1, hfov > vfov => tan(hfov/2) > tan(vfov/2) => dH < dV.
    // So dist = dV / 0.8.
    const aspect = 16 / 9;
    const fov = 45;
    const min = new THREE.Vector3(-50, -50, -50);
    const max = new THREE.Vector3(50, 50, 50);
    const d = computeFramingDistance(min, max, fov, aspect, 0.8);
    const vfov = (fov * Math.PI) / 180;
    const dV = (100 / 2) / Math.tan(vfov / 2);
    const expected = dV / 0.8;
    expect(d).toBeCloseTo(expected, 3);
    // Sanity: ~150-152 for these inputs.
    expect(d).toBeGreaterThan(140);
    expect(d).toBeLessThan(160);
  });

  it("uses the larger of dV and dH so neither axis crops", () => {
    // For a tall narrow viewport (aspect < 1), hfov < vfov so dH > dV;
    // the function should pick dH.
    const aspect = 0.5;
    const fov = 60;
    const min = new THREE.Vector3(-1, -1, -1);
    const max = new THREE.Vector3(1, 1, 1);
    const d = computeFramingDistance(min, max, fov, aspect, 1.0);
    const vfov = (fov * Math.PI) / 180;
    const hfov = 2 * Math.atan(Math.tan(vfov / 2) * aspect);
    const dV = 1 / Math.tan(vfov / 2);
    const dH = 1 / Math.tan(hfov / 2);
    expect(d).toBeCloseTo(Math.max(dV, dH), 5);
    expect(d).toBeGreaterThanOrEqual(dV);
  });

  it("handles a flat panel (one extent == 0) without producing NaN", () => {
    // Panel: 100 wide in X, 0 thick in Y, 50 deep in Z. extent_max = 100.
    const min = new THREE.Vector3(-50, 0, -25);
    const max = new THREE.Vector3(50, 0, 25);
    const d = computeFramingDistance(min, max, 45, 16 / 9, 0.8);
    expect(Number.isFinite(d)).toBe(true);
    expect(d).toBeGreaterThan(0);
  });

  it("returns the fallback distance when the AABB is degenerate (min == max)", () => {
    const p = new THREE.Vector3(5, 5, 5);
    const d = computeFramingDistance(p, p, 45, 16 / 9, 0.8);
    expect(d).toBe(FRAMING_FALLBACK_DISTANCE);
  });

  it("treats viewportAspect <= 0 as 1 to avoid singular hfov", () => {
    const min = new THREE.Vector3(-1, -1, -1);
    const max = new THREE.Vector3(1, 1, 1);
    const d = computeFramingDistance(min, max, 45, 0, 0.8);
    expect(Number.isFinite(d)).toBe(true);
    expect(d).toBeGreaterThan(0);
  });

  it("treats fillFraction <= 0 as PRESET_FILL_FRACTION", () => {
    const min = new THREE.Vector3(-1, -1, -1);
    const max = new THREE.Vector3(1, 1, 1);
    const dBad = computeFramingDistance(min, max, 45, 1, 0);
    const dDefault = computeFramingDistance(min, max, 45, 1, PRESET_FILL_FRACTION);
    expect(dBad).toBeCloseTo(dDefault, 5);
  });

  it("scales linearly with the largest extent", () => {
    const small = computeFramingDistance(
      new THREE.Vector3(-1, -1, -1),
      new THREE.Vector3(1, 1, 1),
      60,
      1,
      1,
    );
    const big = computeFramingDistance(
      new THREE.Vector3(-10, -10, -10),
      new THREE.Vector3(10, 10, 10),
      60,
      1,
      1,
    );
    // Same FoV / aspect / fill, just 10x extent => 10x distance.
    expect(big / small).toBeCloseTo(10, 5);
  });
});

describe("viewPresetOffset", () => {
  it("overhead places camera directly above target on +Y", () => {
    const off = viewPresetOffset("overhead", 50);
    expect(off.x).toBeCloseTo(0, 5);
    expect(off.z).toBeCloseTo(0, 5);
    expect(off.y).toBeCloseTo(50, 5);
  });

  it("side places camera along +X at y == 0", () => {
    const off = viewPresetOffset("side", 30);
    expect(off.x).toBeCloseTo(30, 5);
    expect(off.y).toBeCloseTo(0, 5);
    expect(off.z).toBeCloseTo(0, 5);
  });

  it("fore places camera along +Z; aft along -Z; mirror image", () => {
    const fore = viewPresetOffset("fore", 20);
    const aft = viewPresetOffset("aft", 20);
    expect(fore.z).toBeCloseTo(20, 5);
    expect(aft.z).toBeCloseTo(-20, 5);
    expect(fore.x).toBeCloseTo(0, 5);
    expect(aft.x).toBeCloseTo(0, 5);
  });

  it("perspective places camera in +X / +Y / +Z octant at half-distance per axis", () => {
    const off = viewPresetOffset("perspective", 40);
    // 3/4 view: each component is dist * 0.5.
    expect(off.x).toBeCloseTo(20, 5);
    expect(off.y).toBeCloseTo(20, 5);
    expect(off.z).toBeCloseTo(20, 5);
  });

  it("perspective2 differs from perspective by a sign flip on X", () => {
    const p1 = viewPresetOffset("perspective", 40);
    const p2 = viewPresetOffset("perspective2", 40);
    expect(p2.x).toBeCloseTo(-p1.x, 5);
    // Y and Z stay the same so the camera ends up 90deg around the
    // vertical axis from the perspective preset.
    expect(p2.y).toBeCloseTo(p1.y, 5);
    expect(p2.z).toBeCloseTo(p1.z, 5);
  });

  it("preserves length close to dist for axial presets", () => {
    // Axial presets (overhead / side / fore / aft) sit at exactly `dist`
    // from the origin so the camera-to-target distance == dist.
    expect(viewPresetOffset("overhead", 17).length()).toBeCloseTo(17, 5);
    expect(viewPresetOffset("side", 17).length()).toBeCloseTo(17, 5);
    expect(viewPresetOffset("fore", 17).length()).toBeCloseTo(17, 5);
    expect(viewPresetOffset("aft", 17).length()).toBeCloseTo(17, 5);
  });
});

describe("viewPresetForKeyCode", () => {
  it("maps Numpad0..5 to the expected presets", () => {
    expect(viewPresetForKeyCode("Numpad0")).toBe("overhead");
    expect(viewPresetForKeyCode("Numpad1")).toBe("perspective2");
    expect(viewPresetForKeyCode("Numpad2")).toBe("side");
    expect(viewPresetForKeyCode("Numpad3")).toBe("fore");
    expect(viewPresetForKeyCode("Numpad4")).toBe("aft");
    expect(viewPresetForKeyCode("Numpad5")).toBe("perspective");
  });

  it("returns null for unbound codes", () => {
    expect(viewPresetForKeyCode("KeyR")).toBeNull();
    expect(viewPresetForKeyCode("KeyH")).toBeNull();
    expect(viewPresetForKeyCode("Numpad6")).toBeNull();
    expect(viewPresetForKeyCode("Digit0")).toBeNull();
    expect(viewPresetForKeyCode("")).toBeNull();
  });
});

describe("dispatchViewerHotkey", () => {
  function makeMockHandle(): {
    handle: Pick<FlightCamHandle, "resetToScene" | "setView">;
    resetCalls: THREE.Object3D[];
    setViewCalls: { preset: ViewPreset; root: THREE.Object3D }[];
  } {
    const resetCalls: THREE.Object3D[] = [];
    const setViewCalls: { preset: ViewPreset; root: THREE.Object3D }[] = [];
    return {
      handle: {
        resetToScene(root) {
          resetCalls.push(root);
        },
        setView(preset, root) {
          setViewCalls.push({ preset, root });
        },
      },
      resetCalls,
      setViewCalls,
    };
  }

  it("KeyR triggers handle.resetToScene with the provided sceneRoot", () => {
    const m = makeMockHandle();
    const root = new THREE.Object3D();
    let toggled = 0;
    const handled = dispatchViewerHotkey(
      { code: "KeyR", repeat: false },
      m.handle,
      root,
      () => { toggled += 1; },
    );
    expect(handled).toBe(true);
    expect(m.resetCalls.length).toBe(1);
    expect(m.resetCalls[0]).toBe(root);
    expect(m.setViewCalls.length).toBe(0);
    expect(toggled).toBe(0);
  });

  it("KeyR with a null handle is a no-op but still claims the event", () => {
    const root = new THREE.Object3D();
    let toggled = 0;
    const handled = dispatchViewerHotkey(
      { code: "KeyR", repeat: false },
      null,
      root,
      () => { toggled += 1; },
    );
    // Still handled (caller will preventDefault); just nothing to call.
    expect(handled).toBe(true);
    expect(toggled).toBe(0);
  });

  it("KeyH toggles HUD on first press but not on repeat", () => {
    const m = makeMockHandle();
    const root = new THREE.Object3D();
    let toggled = 0;
    const handled = dispatchViewerHotkey(
      { code: "KeyH", repeat: false },
      m.handle,
      root,
      () => { toggled += 1; },
    );
    expect(handled).toBe(true);
    expect(toggled).toBe(1);

    // Repeat event must not re-fire the toggle (held H should not strobe).
    const handled2 = dispatchViewerHotkey(
      { code: "KeyH", repeat: true },
      m.handle,
      root,
      () => { toggled += 1; },
    );
    expect(handled2).toBe(true);
    expect(toggled).toBe(1);
  });

  it("Numpad0 triggers handle.setView('overhead', sceneRoot)", () => {
    const m = makeMockHandle();
    const root = new THREE.Object3D();
    const handled = dispatchViewerHotkey(
      { code: "Numpad0", repeat: false },
      m.handle,
      root,
      () => {},
    );
    expect(handled).toBe(true);
    expect(m.setViewCalls.length).toBe(1);
    expect(m.setViewCalls[0].preset).toBe("overhead");
    expect(m.setViewCalls[0].root).toBe(root);
  });

  it("Numpad presets ignore repeat events", () => {
    const m = makeMockHandle();
    const root = new THREE.Object3D();
    const handled = dispatchViewerHotkey(
      { code: "Numpad2", repeat: true },
      m.handle,
      root,
      () => {},
    );
    expect(handled).toBe(true);
    expect(m.setViewCalls.length).toBe(0);
  });

  it("returns false for unbound codes (caller preserves default behaviour)", () => {
    const m = makeMockHandle();
    const root = new THREE.Object3D();
    let toggled = 0;
    const handled = dispatchViewerHotkey(
      { code: "KeyZ", repeat: false },
      m.handle,
      root,
      () => { toggled += 1; },
    );
    expect(handled).toBe(false);
    expect(m.resetCalls.length).toBe(0);
    expect(m.setViewCalls.length).toBe(0);
    expect(toggled).toBe(0);
  });

  it("KeyR with null sceneRoot is a no-op but still handled", () => {
    const m = makeMockHandle();
    const handled = dispatchViewerHotkey(
      { code: "KeyR", repeat: false },
      m.handle,
      null,
      () => {},
    );
    expect(handled).toBe(true);
    // No call placed because there is no sceneRoot to frame.
    expect(m.resetCalls.length).toBe(0);
  });
});
