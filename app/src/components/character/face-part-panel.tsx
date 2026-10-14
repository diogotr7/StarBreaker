// Face-part panel: a labelled QuadBlendPad wired to the character store.
// Reads one face part's blends + the current gender, and writes changes back
// via setPart (which debounces a GLB regenerate).
import { QuadBlendPad } from "./quad-blend-pad";
import { useCharacterStore } from "../../stores/character-store";
import { FACE_PARTS, headIdMax } from "../../lib/dna";

export function FacePartPanel({ index }: { index: number }) {
  const part = useCharacterStore((s) => s.dna.face_parts[index]);
  const gender = useCharacterStore((s) => s.dna.gender);
  const setPart = useCharacterStore((s) => s.setPart);

  const values = part.map((b) => b.value) as [number, number, number, number];
  const headIds = part.map((b) => b.head_id) as [number, number, number, number];

  return (
    <div className="flex flex-col gap-1 p-2 border border-border rounded-md bg-bg-alt">
      <span className="text-xs font-medium text-text">{FACE_PARTS[index]}</span>
      <QuadBlendPad
        values={values}
        headIds={headIds}
        headIdMax={headIdMax[gender]}
        onValues={(parts) => parts.forEach((v, i) => setPart(index, i, v, headIds[i]))}
        onHeadId={(corner, id) => setPart(index, corner, values[corner], id)}
      />
    </div>
  );
}
