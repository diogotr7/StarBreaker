// Quad-blend pad: a square 2D pad with a draggable dot and four corner
// head-id inputs. Pure controlled component — props in, callbacks out, no
// store access. Uses native pointer events (no react-draggable dependency).
import { useRef } from "react";
import { locationFromParts, partsFromLocation } from "../../lib/dna";

interface Props {
  values: [number, number, number, number];
  headIds: [number, number, number, number];
  headIdMax: number;
  onValues: (parts: [number, number, number, number]) => void;
  onHeadId: (corner: number, id: number) => void;
}

export function QuadBlendPad({ values, headIds, headIdMax, onValues, onHeadId }: Props) {
  const padRef = useRef<HTMLDivElement>(null);
  const loc = locationFromParts(values);

  const onPointer = (e: React.PointerEvent) => {
    const el = padRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const left = Math.min(Math.max((e.clientX - r.left) / r.width, 0), 1);
    const top = Math.min(Math.max((e.clientY - r.top) / r.height, 0), 1);
    onValues(partsFromLocation(left, top));
  };

  const pct = (v: number) => `${Math.round((v / 65534) * 100)}%`;

  const renderHeadId = (corner: number) => (
    <input
      type="number"
      min={0}
      max={headIdMax}
      value={headIds[corner]}
      onChange={(e) =>
        onHeadId(corner, Math.max(0, Math.min(headIdMax, Number(e.target.value) || 0)))
      }
      className="w-8 bg-bg-deep border border-border rounded text-text text-[10px] text-center px-0.5 py-0.5 focus:outline-none focus:border-primary/50"
    />
  );

  return (
    <div className="flex flex-col gap-1 w-32">
      <div className="flex items-center justify-between gap-1 text-[10px] text-text-dim">
        {renderHeadId(0)}
        <span>{pct(values[0])}</span>
        <span>{pct(values[1])}</span>
        {renderHeadId(1)}
      </div>
      <div
        ref={padRef}
        onPointerDown={(e) => {
          (e.target as HTMLElement).setPointerCapture(e.pointerId);
          onPointer(e);
        }}
        onPointerMove={(e) => {
          if (e.buttons === 1) onPointer(e);
        }}
        className="relative aspect-square border border-border bg-bg-deep cursor-crosshair"
      >
        <div className="absolute left-1/2 top-0 bottom-0 border-l border-dashed border-border" />
        <div className="absolute top-1/2 left-0 right-0 border-t border-dashed border-border" />
        <div
          className="absolute w-2 h-2 rounded-full bg-primary -translate-x-1/2 -translate-y-1/2 pointer-events-none"
          style={{ left: `${loc.left * 100}%`, top: `${loc.top * 100}%` }}
        />
      </div>
      <div className="flex items-center justify-between gap-1 text-[10px] text-text-dim">
        {renderHeadId(3)}
        <span>{pct(values[3])}</span>
        <span>{pct(values[2])}</span>
        {renderHeadId(2)}
      </div>
    </div>
  );
}
