import { create } from "zustand";
import type { DnaInput, Gender } from "../lib/dna";
import { emptyDna } from "../lib/dna";
import {
  dnaPreviewGlb,
  dnaListExpressions,
  dnaExpressionAnimGlb,
  type ExpressionSet,
} from "../lib/commands";

interface CharacterState {
  dna: DnaInput;
  full: boolean;
  glb: ArrayBuffer | null;
  loading: boolean;
  error: string | null;
  setDna: (dna: DnaInput) => void;
  setGender: (gender: Gender) => void;
  setFull: (full: boolean) => void;
  setPart: (partIndex: number, blendIndex: number, value: number, headId: number) => void;
  regenerate: () => void;
  emotions: ExpressionSet[];
  emotion: string | null;
  variation: number;
  intensity: number;
  playing: boolean;
  loadingAnim: boolean;
  setEmotion: (emotion: string) => void;
  setVariation: (variation: number) => void;
  setIntensity: (intensity: number) => void;
  loadExpressions: () => void;
  playExpression: () => void;
}

// Coalesce rapid edits (pad drags, head-id spinners) into one regenerate.
// The preview GLB is now small + morph-target-free, so the round-trip is fast
// enough for a snappy debounce.
const PREVIEW_DEBOUNCE_MS = 90;

let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let previewGen = 0;

export const useCharacterStore = create<CharacterState>((set, get) => ({
  dna: emptyDna("Female"),
  full: true,
  glb: null,
  loading: false,
  error: null,
  emotions: [],
  emotion: null,
  variation: 1,
  // Faithful clip weight: the CAF controller decode now yields true-magnitude
  // controls, so 1.0 plays the expression at its authored strength (the slider
  // lets the user exaggerate/damp from there).
  intensity: 1.0,
  playing: false,
  loadingAnim: false,

  setDna: (dna) => {
    set({ dna });
    get().regenerate();
  },
  setGender: (gender) => {
    set({ dna: { ...get().dna, gender } });
    get().regenerate();
  },
  setFull: (full) => {
    set({ full });
    get().regenerate();
  },

  setPart: (partIndex, blendIndex, value, headId) => {
    const dna = get().dna;
    const face_parts = dna.face_parts.map((p, i) =>
      i === partIndex
        ? (p.map((b, j) => (j === blendIndex ? { value, head_id: headId } : b)) as typeof p)
        : p,
    );
    set({ dna: { ...dna, face_parts } });
    get().regenerate();
  },

  regenerate: () => {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      const gen = ++previewGen;
      const { dna, full } = get();
      // A new static regenerate supersedes any in-flight Play (previewGen is
      // shared), so clear loadingAnim too — otherwise the Play button would
      // stay stuck on "Loading…" because the play's guarded handler is skipped.
      set({ loading: true, error: null, loadingAnim: false });
      dnaPreviewGlb(dna, full)
        .then((glb) => {
          if (gen === previewGen) set({ glb, loading: false, playing: false });
        })
        .catch((e) => {
          if (gen === previewGen) set({ error: String(e), loading: false });
        });
    }, PREVIEW_DEBOUNCE_MS);
  },

  setEmotion: (emotion) => {
    const e = get().emotions.find((x) => x.emotion === emotion);
    set({ emotion, variation: e?.variations[0] ?? get().variation });
  },
  setVariation: (variation) => set({ variation }),
  setIntensity: (intensity) => set({ intensity }),

  loadExpressions: () => {
    const gender = get().dna.gender;
    dnaListExpressions(gender)
      .then((emotions) => {
        const first = emotions[0];
        set({
          emotions,
          emotion: first?.emotion ?? null,
          variation: first?.variations[0] ?? 1,
        });
      })
      .catch((e) => set({ error: String(e) }));
  },

  playExpression: () => {
    const { dna, emotion, variation, intensity } = get();
    if (!emotion) return;
    const gen = ++previewGen;
    set({ loadingAnim: true, error: null });
    dnaExpressionAnimGlb(dna, emotion, variation, intensity)
      .then((glb) => {
        if (gen === previewGen) set({ glb, playing: true, loadingAnim: false });
      })
      .catch((e) => {
        if (gen === previewGen) set({ error: String(e), loadingAnim: false });
      });
  },
}));
