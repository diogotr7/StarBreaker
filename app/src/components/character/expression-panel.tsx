// Expression picker for the Character Builder: choose an idle emotion +
// variation, set intensity, and play the looping face-anim GLB in the preview.
// Reads/writes the expression state on the character store; the Play button
// triggers playExpression() which swaps in the animated GLB.
import { useCharacterStore } from "../../stores/character-store";

export function ExpressionPanel() {
  const emotions = useCharacterStore((s) => s.emotions);
  const emotion = useCharacterStore((s) => s.emotion);
  const variation = useCharacterStore((s) => s.variation);
  const intensity = useCharacterStore((s) => s.intensity);
  const playing = useCharacterStore((s) => s.playing);
  const loadingAnim = useCharacterStore((s) => s.loadingAnim);
  const setEmotion = useCharacterStore((s) => s.setEmotion);
  const setVariation = useCharacterStore((s) => s.setVariation);
  const setIntensity = useCharacterStore((s) => s.setIntensity);
  const playExpression = useCharacterStore((s) => s.playExpression);

  const current = emotions.find((e) => e.emotion === emotion);
  const sel =
    "bg-bg-alt border border-border rounded px-1 text-xs h-6 text-text-sub";

  return (
    <div className="flex items-center gap-2 px-3 py-1.5 border-t border-border bg-bg-alt shrink-0 flex-wrap">
      <span className="text-[11px] text-text-dim">Expression</span>
      <select
        className={sel}
        value={emotion ?? ""}
        onChange={(e) => setEmotion(e.target.value)}
      >
        {emotions.length === 0 && <option value="">(none)</option>}
        {emotions.map((e) => (
          <option key={e.emotion} value={e.emotion}>
            {e.emotion}
          </option>
        ))}
      </select>
      <select
        className={sel}
        value={variation}
        onChange={(e) => setVariation(Number(e.target.value))}
        disabled={!current}
      >
        {(current?.variations ?? []).map((v) => (
          <option key={v} value={v}>{`idle ${v}`}</option>
        ))}
      </select>
      <label className="flex items-center gap-1 text-[11px] text-text-dim">
        intensity
        <input
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={intensity}
          onChange={(e) => setIntensity(Number(e.target.value))}
          className="w-24 accent-accent"
        />
        <span className="w-7 tabular-nums">{intensity.toFixed(2)}</span>
      </label>
      <button
        onClick={() => playExpression()}
        disabled={!emotion || loadingAnim}
        className="px-3 py-1 rounded-md text-xs font-medium bg-surface text-text-sub hover:bg-surface-hi hover:text-text transition-colors cursor-pointer disabled:opacity-50"
      >
        {loadingAnim ? "Loading…" : "▶ Play"}
      </button>
      {playing && !loadingAnim && (
        <span className="text-[11px] text-text-dim">▶ playing</span>
      )}
    </div>
  );
}
