// Character Builder view: a 2D quad-blend DNA editor (13 face parts) wired to a
// live Three.js head preview. The toolbar flips gender, toggles head-only/full,
// loads/exports .chf, exports .glb, and randomizes/resets the DNA.
//
// File IO matches the app's existing convention (see datacore-browser /
// p4k-browser): the frontend picks a path via @tauri-apps/plugin-dialog, then a
// Rust command performs the read/write. No @tauri-apps/plugin-fs dependency.
import { useEffect } from "react";
import { CharacterPreview } from "../components/character/character-preview";
import { ExpressionPanel } from "../components/character/expression-panel";
import { FacePartPanel } from "../components/character/face-part-panel";
import { useCharacterStore } from "../stores/character-store";
import { FACE_PARTS, emptyDna, getRandDna } from "../lib/dna";
import { dnaExportChfTo, dnaExportGlbTo, dnaImportChf } from "../lib/commands";

export function CharacterBuilder() {
  const dna = useCharacterStore((s) => s.dna);
  const full = useCharacterStore((s) => s.full);
  const glb = useCharacterStore((s) => s.glb);
  const loading = useCharacterStore((s) => s.loading);
  const error = useCharacterStore((s) => s.error);
  const setDna = useCharacterStore((s) => s.setDna);
  const setGender = useCharacterStore((s) => s.setGender);
  const setFull = useCharacterStore((s) => s.setFull);
  const regenerate = useCharacterStore((s) => s.regenerate);
  const loadExpressions = useCharacterStore((s) => s.loadExpressions);

  // Build the initial preview once on mount.
  useEffect(() => {
    regenerate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // (Re)load idle expressions on mount and whenever gender changes.
  useEffect(() => {
    loadExpressions();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dna.gender]);

  const loadChf = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({
      title: "Load .chf",
      multiple: false,
      filters: [{ name: "Character file", extensions: ["chf"] }],
    });
    if (typeof path !== "string") return;
    try {
      const imported = await dnaImportChf(path);
      setDna(imported);
    } catch (err) {
      console.error("Load .chf failed:", err);
    }
  };

  const saveChf = async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      title: "Export .chf",
      defaultPath: "character.chf",
      filters: [{ name: "Character file", extensions: ["chf"] }],
    });
    if (!path) return;
    try {
      await dnaExportChfTo(dna, path);
    } catch (err) {
      console.error("Export .chf failed:", err);
    }
  };

  const saveGlb = async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      title: "Export .glb",
      defaultPath: "character.glb",
      filters: [{ name: "glTF binary", extensions: ["glb"] }],
    });
    if (!path) return;
    try {
      await dnaExportGlbTo(dna, full, path);
    } catch (err) {
      console.error("Export .glb failed:", err);
    }
  };

  const toolbarBtn =
    "px-3 py-1 rounded-md text-xs font-medium bg-surface text-text-sub " +
    "hover:bg-surface-hi hover:text-text transition-colors cursor-pointer shrink-0";

  return (
    <div className="flex-1 flex overflow-hidden">
      {/* ── Left: editor ── */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Toolbar */}
        <div
          className="flex items-center gap-2 px-3 border-b border-border bg-bg-alt shrink-0"
          style={{ height: "var(--toolbar-height)" }}
        >
          <button
            onClick={() => setGender(dna.gender === "Male" ? "Female" : "Male")}
            className={toolbarBtn}
            title="Toggle gender"
          >
            {dna.gender}
          </button>

          <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
            <input
              type="checkbox"
              checked={full}
              onChange={(e) => setFull(e.target.checked)}
              className="accent-accent w-3 h-3"
            />
            <span className="text-[11px] text-text-dim">Full head</span>
          </label>

          <div className="flex-1" />

          <button onClick={loadChf} className={toolbarBtn}>
            Load .chf
          </button>
          <button onClick={saveChf} className={toolbarBtn}>
            Export .chf
          </button>
          <button onClick={saveGlb} className={toolbarBtn}>
            Export .glb
          </button>
          <button
            onClick={() => setDna(getRandDna(dna.gender))}
            className={toolbarBtn}
          >
            Randomize
          </button>
          <button
            onClick={() => setDna(emptyDna(dna.gender))}
            className={toolbarBtn}
          >
            Reset
          </button>
        </div>

        <ExpressionPanel />

        {/* Face-part grid */}
        <div className="flex-1 overflow-y-auto p-3">
          {error && (
            <p className="mb-3 text-xs text-danger break-all">{error}</p>
          )}
          <div className="grid grid-cols-3 gap-2">
            {FACE_PARTS.map((_, i) => (
              <FacePartPanel key={i} index={i} />
            ))}
          </div>
        </div>
      </div>

      {/* ── Right: live preview ── */}
      <div className="w-1/2 max-w-[640px] shrink-0 border-l border-border bg-bg-deep relative">
        <CharacterPreview glb={glb} />
        {loading && (
          <div className="absolute top-2 right-2 text-[11px] text-text-dim bg-bg-alt/80 px-2 py-1 rounded">
            Updating…
          </div>
        )}
      </div>
    </div>
  );
}
