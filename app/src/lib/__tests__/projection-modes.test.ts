// Pure-math coverage for the projection-mode helpers added alongside
// the flight cam. The hook itself wires these into a live ortho camera
// + the renderer; that path is integration-only. The math below is the
// load-bearing piece - if the frustum and shear are right, the hook
// only has DOM/three-side wiring to get wrong.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  applyObliqueShear,
  computeOrthoFrustum,
  nextProjectionMode,
  obliqueCabinetShear,
  PROJECTION_MODES,
  type ProjectionMode,
} from "../flight-camera";

const EPS = 1e-6;

describe("computeOrthoFrustum", () => {
  it("returns the canonical frustum for orbitDist=10 and 1920x1080", () => {
    const f = computeOrthoFrustum(10, 1920, 1080);
    expect(f.top).toBeCloseTo(5, 6);
    expect(f.bottom).toBeCloseTo(-5, 6);
    expect(f.right).toBeCloseTo((5 * 1920) / 1080, 6);
    expect(f.left).toBeCloseTo((-5 * 1920) / 1080, 6);
  });

  it("scales linearly with orbitDist", () => {
    const a = computeOrthoFrustum(10, 1600, 900);
    const b = computeOrthoFrustum(20, 1600, 900);
    expect(b.top).toBeCloseTo(a.top * 2, 6);
    expect(b.right).toBeCloseTo(a.right * 2, 6);
  });

  it("matches aspect for square viewport", () => {
    const f = computeOrthoFrustum(10, 800, 800);
    expect(f.right).toBeCloseTo(5, 6);
    expect(f.top).toBeCloseTo(5, 6);
  });

  it("returns a degenerate (zero-size) frustum for orbitDist=0", () => {
    // Zero is a sentinel - we choose to return zeros rather than crash
    // or NaN. Three.js will produce a singular projection matrix in this
    // case; callers are expected to clamp orbitDist if they need a
    // finite image, but the helper itself stays defined.
    const f = computeOrthoFrustum(0, 1920, 1080);
    // Use toBeCloseTo to side-step the +0 / -0 distinction that
    // toBe checks via Object.is. The value semantics are what matter.
    expect(f.top).toBeCloseTo(0, 6);
    expect(f.bottom).toBeCloseTo(0, 6);
    expect(f.left).toBeCloseTo(0, 6);
    expect(f.right).toBeCloseTo(0, 6);
  });

  it("does not produce NaN when viewportHeight is 0", () => {
    // Falls back to aspect=1 rather than divide-by-zero. A hidden tab or
    // minimised window can briefly report height 0, and we'd rather keep
    // a finite (square) frustum on the books than poison the projection
    // matrix until the next resize.
    const f = computeOrthoFrustum(10, 1920, 0);
    expect(Number.isFinite(f.left)).toBe(true);
    expect(Number.isFinite(f.right)).toBe(true);
    expect(f.top).toBeCloseTo(5, 6);
    // aspect = 1, so right == top.
    expect(f.right).toBeCloseTo(5, 6);
    expect(f.left).toBeCloseTo(-5, 6);
  });

  it("does not produce NaN when viewportHeight is negative", () => {
    // Defensive: negative is nonsense input, but we still don't want a
    // NaN frustum to leak into Three.js.
    const f = computeOrthoFrustum(10, 1920, -100);
    expect(Number.isFinite(f.left)).toBe(true);
    expect(Number.isFinite(f.right)).toBe(true);
  });
});

describe("obliqueCabinetShear", () => {
  it("encodes 45deg cabinet shear with scale 0.5", () => {
    const m = obliqueCabinetShear();
    // Three.js Matrix4 storage is column-major: elements[i + j*4] is
    // the entry at row i, column j. Shear values land at row 0/1
    // column 2, which is index 8 (j=2, i=0) and 9 (j=2, i=1).
    const expectedX = 0.5 * Math.cos(Math.PI / 4);
    const expectedY = 0.5 * Math.sin(Math.PI / 4);
    expect(m.elements[8]).toBeCloseTo(expectedX, 6);
    expect(m.elements[9]).toBeCloseTo(expectedY, 6);
    // The diagonal stays identity.
    expect(m.elements[0]).toBeCloseTo(1, 6);
    expect(m.elements[5]).toBeCloseTo(1, 6);
    expect(m.elements[10]).toBeCloseTo(1, 6);
    expect(m.elements[15]).toBeCloseTo(1, 6);
    // The remaining off-diagonal entries are zero.
    const zeroIdx = [1, 2, 3, 4, 6, 7, 11, 12, 13, 14];
    for (const i of zeroIdx) {
      expect(Math.abs(m.elements[i])).toBeLessThan(EPS);
    }
  });

  it("returns a fresh matrix every call", () => {
    const a = obliqueCabinetShear();
    const b = obliqueCabinetShear();
    expect(a).not.toBe(b);
    a.identity();
    // Mutating `a` must not affect `b`.
    expect(b.elements[8]).toBeCloseTo(0.5 * Math.cos(Math.PI / 4), 6);
  });
});

describe("applyObliqueShear", () => {
  it("post-multiplies the shear into the input (this = this * shear)", () => {
    // Three.js convention: m.multiply(other) is `this = this * other`.
    // We verify that by starting with identity and confirming the
    // resulting matrix equals the shear itself.
    const proj = new THREE.Matrix4().identity();
    const shear = obliqueCabinetShear();
    const out = applyObliqueShear(proj, shear);
    expect(out).toBe(proj);
    for (let i = 0; i < 16; i += 1) {
      expect(out.elements[i]).toBeCloseTo(shear.elements[i], 6);
    }
  });

  it("does not mutate the shear matrix", () => {
    const proj = new THREE.Matrix4().makeScale(2, 3, 4);
    const shear = obliqueCabinetShear();
    const shearBefore = shear.clone();
    applyObliqueShear(proj, shear);
    for (let i = 0; i < 16; i += 1) {
      expect(shear.elements[i]).toBeCloseTo(shearBefore.elements[i], 6);
    }
  });

  it("composes with a non-identity projection (post-multiply order)", () => {
    // Build a known projection (a scale), apply the shear, and confirm
    // the result equals scale * shear computed manually. If the helper
    // accidentally pre-multiplied (this = other * this) the entries
    // outside row 0/1 col 2 would change.
    const scale = new THREE.Matrix4().makeScale(2, 3, 4);
    const shear = obliqueCabinetShear();
    const expected = scale.clone().multiply(shear);
    const out = applyObliqueShear(scale, shear);
    for (let i = 0; i < 16; i += 1) {
      expect(out.elements[i]).toBeCloseTo(expected.elements[i], 6);
    }
  });
});

describe("nextProjectionMode / cycle", () => {
  it("advances perspective -> orthographic -> oblique -> perspective", () => {
    expect(nextProjectionMode("perspective")).toBe("orthographic");
    expect(nextProjectionMode("orthographic")).toBe("oblique");
    expect(nextProjectionMode("oblique")).toBe("perspective");
  });

  it("returns to perspective after three cycles", () => {
    let m: ProjectionMode = "perspective";
    m = nextProjectionMode(m);
    m = nextProjectionMode(m);
    m = nextProjectionMode(m);
    expect(m).toBe("perspective");
  });

  it("PROJECTION_MODES carries all three values exactly once", () => {
    expect(PROJECTION_MODES).toHaveLength(3);
    expect(new Set(PROJECTION_MODES).size).toBe(3);
    expect(PROJECTION_MODES).toContain("perspective");
    expect(PROJECTION_MODES).toContain("orthographic");
    expect(PROJECTION_MODES).toContain("oblique");
  });
});
