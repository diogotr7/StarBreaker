// Coverage for the MobiGlas render-style helpers. Math correctness lives
// in `blendTowardCyan`; lifecycle correctness lives in the registry +
// `updateMobiGlasTime`. Both are headless-testable without a live
// WebGLRenderer, since ShaderMaterial accepts uniform writes outside a
// render loop.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  blendTowardCyan,
  clearMobiGlasMaterials,
  isMobiGlasMaterialRegistered,
  makeMobiGlasMaterial,
  updateMobiGlasTime,
} from "../decomposed-loader";

const TARGET_HUE = 0.52;
const HUE_BLEND = 0.15;
const TARGET_SATURATION = 0.7;
const TARGET_LIGHTNESS = 0.55;

/** Convert (h, s, l) to (r, g, b) using THREE.Color so the expected
 *  values come from the same conversion path the production code uses. */
function expectedRgb(h: number): { r: number; g: number; b: number } {
  const c = new THREE.Color().setHSL(h, TARGET_SATURATION, TARGET_LIGHTNESS);
  return { r: c.r, g: c.g, b: c.b };
}

describe("blendTowardCyan", () => {
  it("biases red toward cyan with the documented hue blend", () => {
    // Red has h=0, so blendedHue = 0.52 + (0 - 0.52) * 0.15 = 0.442.
    const { r, g, b } = blendTowardCyan(0xff0000);
    const blendedHue = TARGET_HUE + (0 - TARGET_HUE) * HUE_BLEND;
    expect(blendedHue).toBeCloseTo(0.442, 4);
    const exp = expectedRgb(blendedHue);
    expect(r).toBeCloseTo(exp.r, 2);
    expect(g).toBeCloseTo(exp.g, 2);
    expect(b).toBeCloseTo(exp.b, 2);
  });

  it("leaves cyan nearly at the target hue", () => {
    // Cyan has h=0.5, so blendedHue = 0.52 + (0.5 - 0.52) * 0.15 = 0.517.
    const { r, g, b } = blendTowardCyan(0x00ffff);
    const blendedHue = TARGET_HUE + (0.5 - TARGET_HUE) * HUE_BLEND;
    expect(blendedHue).toBeCloseTo(0.517, 4);
    const exp = expectedRgb(blendedHue);
    expect(r).toBeCloseTo(exp.r, 3);
    expect(g).toBeCloseTo(exp.g, 3);
    expect(b).toBeCloseTo(exp.b, 3);
  });

  it("returns components in the 0..1 range", () => {
    const { r, g, b } = blendTowardCyan(0x123456);
    expect(r).toBeGreaterThanOrEqual(0);
    expect(r).toBeLessThanOrEqual(1);
    expect(g).toBeGreaterThanOrEqual(0);
    expect(g).toBeLessThanOrEqual(1);
    expect(b).toBeGreaterThanOrEqual(0);
    expect(b).toBeLessThanOrEqual(1);
  });
});

describe("makeMobiGlasMaterial + lifecycle", () => {
  it("registers the material and ticks its time uniform", () => {
    clearMobiGlasMaterials();
    const { material } = makeMobiGlasMaterial(0xff0000);
    expect(isMobiGlasMaterialRegistered(material)).toBe(true);
    updateMobiGlasTime(1.5);
    expect(material.uniforms.time.value).toBe(1.5);
    clearMobiGlasMaterials();
  });

  it("stops ticking once unregistered", () => {
    clearMobiGlasMaterials();
    const { material, unregister } = makeMobiGlasMaterial(0x00ff00);
    updateMobiGlasTime(2.0);
    expect(material.uniforms.time.value).toBe(2.0);
    unregister();
    expect(isMobiGlasMaterialRegistered(material)).toBe(false);
    updateMobiGlasTime(99.0);
    // Last tick before unregister was 2.0; the uniform must not advance.
    expect(material.uniforms.time.value).toBe(2.0);
    clearMobiGlasMaterials();
  });

  it("clearMobiGlasMaterials drops every registered material", () => {
    clearMobiGlasMaterials();
    const a = makeMobiGlasMaterial(0xff0000).material;
    const b = makeMobiGlasMaterial(0x00ff00).material;
    expect(isMobiGlasMaterialRegistered(a)).toBe(true);
    expect(isMobiGlasMaterialRegistered(b)).toBe(true);
    clearMobiGlasMaterials();
    expect(isMobiGlasMaterialRegistered(a)).toBe(false);
    expect(isMobiGlasMaterialRegistered(b)).toBe(false);
  });

  it("dispose after unregister does not throw", () => {
    clearMobiGlasMaterials();
    const { material, unregister } = makeMobiGlasMaterial(0x0000ff);
    unregister();
    expect(() => material.dispose()).not.toThrow();
  });

  it("seeds baseColor uniform from blendTowardCyan", () => {
    clearMobiGlasMaterials();
    const input = 0xff0000;
    const blended = blendTowardCyan(input);
    const { material } = makeMobiGlasMaterial(input);
    const uVec = material.uniforms.baseColor.value as THREE.Vector3;
    expect(uVec.x).toBeCloseTo(blended.r, 5);
    expect(uVec.y).toBeCloseTo(blended.g, 5);
    expect(uVec.z).toBeCloseTo(blended.b, 5);
    clearMobiGlasMaterials();
  });

  it("seeds time uniform at 0.0", () => {
    clearMobiGlasMaterials();
    const { material } = makeMobiGlasMaterial(0xffffff);
    expect(material.uniforms.time.value).toBe(0.0);
    clearMobiGlasMaterials();
  });
});
