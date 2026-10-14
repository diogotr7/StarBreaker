// DNA model + 2D quad-blend math for the Character Builder.
// Bilinear pad math ported verbatim from starchar/src/components/DnaQuadBlend.tsx
// (engine-correct, framework-agnostic).

export type Gender = "Male" | "Female";

export interface Blend {
  value: number; // 0..=65534
  head_id: number;
}

export type FacePartBlends = [Blend, Blend, Blend, Blend];

export interface DnaInput {
  gender: Gender;
  /** Length 13, indexed by FACE_PARTS order. */
  face_parts: FacePartBlends[];
}

/** Face parts in FacePart-enum order (matches the Rust DTO index mapping). */
export const FACE_PARTS = [
  "Left Eyebrow", "Right Eyebrow", "Left Eye", "Right Eye", "Nose",
  "Left Ear", "Right Ear", "Left Cheek", "Right Cheek", "Mouth",
  "Jaw", "Crown", "Neck",
] as const;

export const blendTotal = 65534;
export const headIdMax: Record<Gender, number> = { Male: 59, Female: 47 };

/** Blue-dot position (0..1, 0..1) from 4 corner weights (TL, TR, BR, BL). */
export function locationFromParts(
  parts: [number, number, number, number],
): { left: number; top: number } {
  const p = parts.map((x) => x / blendTotal) as [number, number, number, number];
  const corners = [
    { left: 0, top: 0 }, { left: 1, top: 0 },
    { left: 1, top: 1 }, { left: 0, top: 1 },
  ];
  let left = 0, top = 0;
  for (let i = 0; i < 4; i++) {
    left += corners[i].left * p[i];
    top += corners[i].top * p[i];
  }
  return { left, top };
}

/** 4 corner weights (summing to blendTotal) from a blue-dot position. */
export function partsFromLocation(
  left: number,
  top: number,
): [number, number, number, number] {
  left = Math.max(0, Math.min(1, left));
  top = Math.max(0, Math.min(1, top));
  const w1 = (1 - left) * (1 - top); // TL
  const w2 = left * (1 - top);       // TR
  const w3 = left * top;             // BR
  const w4 = (1 - left) * top;       // BL
  const sum = w1 + w2 + w3 + w4;
  return [
    Math.round((w1 / sum) * blendTotal),
    Math.round((w2 / sum) * blendTotal),
    Math.round((w3 / sum) * blendTotal),
    Math.round((w4 / sum) * blendTotal),
  ];
}

/**
 * Default DNA: corner head_ids 0,1,2,3 with full weight on the first
 * (head_id 0 = the .skin bind pose). The 1/2/3 corners are pre-seeded so the
 * quad-blend pad has distinct archetypes to drag toward out of the box.
 */
export function emptyDna(gender: Gender): DnaInput {
  return {
    gender,
    face_parts: FACE_PARTS.map(() => [
      { value: blendTotal, head_id: 0 },
      { value: 0, head_id: 1 },
      { value: 0, head_id: 2 },
      { value: 0, head_id: 3 },
    ] as FacePartBlends),
  };
}

/** Randomized DNA. Mirrors L/R pairs so faces stay sensible. */
export function getRandDna(gender: Gender): DnaInput {
  const maxId = headIdMax[gender];
  const randPart = (): FacePartBlends => {
    let total = blendTotal;
    const out: Blend[] = [];
    for (let i = 0; i < 4; i++) {
      const value = Math.floor(Math.random() * total);
      out.push({ value: i === 3 ? total : value, head_id: Math.floor(Math.random() * maxId) + 1 });
      total -= value;
    }
    return out as FacePartBlends;
  };
  const parts = FACE_PARTS.map(randPart);
  // Mirror right->left (indices per FACE_PARTS order).
  parts[0] = parts[1]; // Left Eyebrow = Right Eyebrow
  parts[2] = parts[3]; // Left Eye = Right Eye
  parts[5] = parts[6]; // Left Ear = Right Ear
  parts[7] = parts[8]; // Left Cheek = Right Cheek
  return { gender, face_parts: parts };
}
