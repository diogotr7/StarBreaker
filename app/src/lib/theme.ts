import type { SystemPalette } from "./commands";

/** Derive a full set of CSS variables from the OS system palette. */
export function applySystemTheme(palette: SystemPalette) {
  const root = document.documentElement;
  const dark = palette.scheme === "Dark";

  const bg = palette.background;
  const fg = palette.foreground;
  const accent = palette.accent;

  // Tint grays slightly toward the accent color for warmth.
  const tint = (base: string, amount: number) => blend(base, accent, 0.12 * Math.abs(amount / 30));
  const step = (amount: number) => tint(nudge(bg, amount), amount);

  root.style.setProperty("--color-bg", bg);
  root.style.setProperty("--color-bg-alt", step(dark ? 4 : -4));
  root.style.setProperty("--color-bg-deep", step(dark ? -6 : 6));
  root.style.setProperty("--color-surface", step(dark ? 12 : -8));
  root.style.setProperty("--color-surface-hi", step(dark ? 20 : -14));
  root.style.setProperty("--color-overlay", step(dark ? 30 : -22));

  root.style.setProperty("--color-text", fg);
  root.style.setProperty("--color-text-sub", blend(fg, bg, 0.3));
  root.style.setProperty("--color-text-dim", blend(fg, bg, 0.5));
  root.style.setProperty("--color-text-faint", blend(fg, bg, 0.68));

  // Compute contrast text color for use on primary/accent backgrounds.
  const onAccent = luminance(accent) > 0.4 ? "#000000" : "#FFFFFF";

  root.style.setProperty("--color-primary", accent);
  root.style.setProperty("--color-accent", accent);
  root.style.setProperty("--color-on-primary", onAccent);
  root.style.setProperty("--color-on-accent", onAccent);
  root.style.setProperty("--color-success", palette.success);
  root.style.setProperty("--color-warning", palette.warning);
  root.style.setProperty("--color-danger", palette.danger);

  root.style.setProperty("--color-border", step(dark ? 16 : -10));
  root.style.setProperty("--color-ring", accent);
}

// ── Helpers ──

/** Parse a hex color like "#1F1F1F" into [r, g, b]. */
function parseHex(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

/** Format [r, g, b] as "#RRGGBB". */
function toHex(r: number, g: number, b: number): string {
  const clamp = (n: number) => Math.max(0, Math.min(255, Math.round(n)));
  return `#${clamp(r).toString(16).padStart(2, "0")}${clamp(g).toString(16).padStart(2, "0")}${clamp(b).toString(16).padStart(2, "0")}`;
}

/** Nudge a hex color by adding `amount` to each channel. */
function nudge(hex: string, amount: number): string {
  const [r, g, b] = parseHex(hex);
  return toHex(r + amount, g + amount, b + amount);
}

/** Relative luminance of a hex color (0 = black, 1 = white). */
function luminance(hex: string): number {
  const [r, g, b] = parseHex(hex).map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Blend two hex colors: result = a * (1 - t) + b * t. */
function blend(a: string, b: string, t: number): string {
  const [ar, ag, ab] = parseHex(a);
  const [br, bg, bb] = parseHex(b);
  return toHex(
    ar + (br - ar) * t,
    ag + (bg - ag) * t,
    ab + (bb - ab) * t,
  );
}
