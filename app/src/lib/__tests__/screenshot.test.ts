// Pure-helper coverage for the screenshot module. Skips captureScreenshot
// itself - that path needs a live WebGL context, a DOM, and Tauri APIs,
// none of which are healthy under Vitest. The math + filename helpers are
// the load-bearing pieces that govern output framing and naming; if these
// are right, the live capture path only has Three.js wiring to get wrong.

import { describe, expect, it } from "vitest";
import {
  computeScreenshotDimensions,
  formatScreenshotFilename,
} from "../screenshot";

describe("computeScreenshotDimensions", () => {
  it("returns ~10000x5000 for aspect=2.0 and 50 megapixels", () => {
    const { width, height } = computeScreenshotDimensions(2.0, 50);
    // Within 1% of 10000x5000.
    expect(Math.abs(width - 10000)).toBeLessThan(100);
    expect(Math.abs(height - 5000)).toBeLessThan(50);
  });

  it("returns ~7071x7071 for aspect=1.0 and 50 megapixels", () => {
    const { width, height } = computeScreenshotDimensions(1.0, 50);
    expect(Math.abs(width - 7071)).toBeLessThan(71);
    expect(Math.abs(height - 7071)).toBeLessThan(71);
  });

  it("produces ~50M pixels for aspect=16/9 and 50 megapixels", () => {
    const { width, height } = computeScreenshotDimensions(16 / 9, 50);
    const total = width * height;
    // Within 1% of 50,000,000.
    expect(Math.abs(total - 50_000_000) / 50_000_000).toBeLessThan(0.01);
  });

  it("preserves the requested aspect ratio within rounding distance", () => {
    const { width, height } = computeScreenshotDimensions(2.0, 50);
    expect(Math.abs(width / height - 2.0)).toBeLessThan(0.001);
  });
});

describe("formatScreenshotFilename", () => {
  it("formats a known slug + date deterministically", () => {
    // April is month index 3 in JS Date; the formatter must zero-pad
    // both the date and the time fields.
    const d = new Date(2026, 3, 28, 14, 23, 45);
    expect(formatScreenshotFilename("mustang_alpha", d)).toBe(
      "screenshot_mustang_alpha_20260428_142345.png",
    );
  });

  it("falls back to 'viewer' when the slug is null", () => {
    const d = new Date(2026, 0, 1, 0, 0, 0);
    expect(formatScreenshotFilename(null, d)).toBe(
      "screenshot_viewer_20260101_000000.png",
    );
  });

  it("falls back to 'viewer' when the slug is undefined", () => {
    const d = new Date(2026, 0, 1, 0, 0, 0);
    expect(formatScreenshotFilename(undefined, d)).toBe(
      "screenshot_viewer_20260101_000000.png",
    );
  });

  it("falls back to 'viewer' when the slug is the empty string", () => {
    const d = new Date(2026, 0, 1, 0, 0, 0);
    expect(formatScreenshotFilename("", d)).toBe(
      "screenshot_viewer_20260101_000000.png",
    );
  });

  it("zero-pads single-digit hours, minutes, seconds", () => {
    const d = new Date(2026, 0, 1, 1, 2, 3);
    expect(formatScreenshotFilename("x", d)).toBe(
      "screenshot_x_20260101_010203.png",
    );
  });
});
