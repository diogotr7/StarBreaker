import { describe, expect, it } from "vitest";
import { blendTotal, locationFromParts, partsFromLocation } from "./dna";

describe("quad-blend bilinear math", () => {
  it("each corner location maps to 100% on its slot", () => {
    expect(partsFromLocation(0, 0)).toEqual([blendTotal, 0, 0, 0]); // TL
    expect(partsFromLocation(1, 0)).toEqual([0, blendTotal, 0, 0]); // TR
    expect(partsFromLocation(1, 1)).toEqual([0, 0, blendTotal, 0]); // BR
    expect(partsFromLocation(0, 1)).toEqual([0, 0, 0, blendTotal]); // BL
  });

  it("center splits evenly", () => {
    const p = partsFromLocation(0.5, 0.5);
    p.forEach((v) => expect(Math.abs(v - blendTotal / 4)).toBeLessThanOrEqual(1));
  });

  it("location round-trips back through partsFromLocation", () => {
    for (const [l, t] of [[0.25, 0.75], [0.6, 0.1], [0.5, 0.5]]) {
      const parts = partsFromLocation(l, t);
      const loc = locationFromParts(parts);
      expect(Math.abs(loc.left - l)).toBeLessThan(0.001);
      expect(Math.abs(loc.top - t)).toBeLessThan(0.001);
    }
  });
});
