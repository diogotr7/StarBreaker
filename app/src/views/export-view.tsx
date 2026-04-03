import { useEffect, useState } from "react";
import { useExportStore } from "../stores/export-store";
import { ResizeHandle } from "../components/resize-handle";
import {
  scanCategories,
  startExport,
  cancelExport,
  onExportProgress,
  onExportDone,
  browseOutputDir,
  type ExportRequest,
} from "../lib/commands";

export function ExportView() {
  const [optionsWidth, setOptionsWidth] = useState(260);
  const categories = useExportStore((s) => s.categories);
  const categoriesLoading = useExportStore((s) => s.categoriesLoading);
  const activeCategory = useExportStore((s) => s.activeCategory);
  const setActiveCategory = useExportStore((s) => s.setActiveCategory);
  const setCategories = useExportStore((s) => s.setCategories);
  const setCategoriesLoading = useExportStore((s) => s.setCategoriesLoading);

  const selected = useExportStore((s) => s.selected);
  const toggleEntity = useExportStore((s) => s.toggleEntity);
  const selectAllFiltered = useExportStore((s) => s.selectAllFiltered);
  const clearFiltered = useExportStore((s) => s.clearFiltered);

  const search = useExportStore((s) => s.search);
  const setSearch = useExportStore((s) => s.setSearch);
  const hideNpcVariants = useExportStore((s) => s.hideNpcVariants);
  const setHideNpcVariants = useExportStore((s) => s.setHideNpcVariants);

  const lod = useExportStore((s) => s.lod);
  const mip = useExportStore((s) => s.mip);
  const includeTextures = useExportStore((s) => s.includeTextures);
  const includeInterior = useExportStore((s) => s.includeInterior);
  const includeNormals = useExportStore((s) => s.includeNormals);
  const includeLights = useExportStore((s) => s.includeLights);
  const includeTangents = useExportStore((s) => s.includeTangents);
  const includeMaterials = useExportStore((s) => s.includeMaterials);
  const experimentalTextures = useExportStore((s) => s.experimentalTextures);
  const threads = useExportStore((s) => s.threads);
  const outputDir = useExportStore((s) => s.outputDir);
  const setLod = useExportStore((s) => s.setLod);
  const setMip = useExportStore((s) => s.setMip);
  const setIncludeTextures = useExportStore((s) => s.setIncludeTextures);
  const setIncludeInterior = useExportStore((s) => s.setIncludeInterior);
  const setIncludeNormals = useExportStore((s) => s.setIncludeNormals);
  const setIncludeLights = useExportStore((s) => s.setIncludeLights);
  const setIncludeTangents = useExportStore((s) => s.setIncludeTangents);
  const setIncludeMaterials = useExportStore((s) => s.setIncludeMaterials);
  const setExperimentalTextures = useExportStore((s) => s.setExperimentalTextures);
  const setThreads = useExportStore((s) => s.setThreads);
  const setOutputDir = useExportStore((s) => s.setOutputDir);

  const exporting = useExportStore((s) => s.exporting);
  const progress = useExportStore((s) => s.progress);
  const progressTotal = useExportStore((s) => s.progressTotal);
  const progressLabel = useExportStore((s) => s.progressLabel);
  const exportErrors = useExportStore((s) => s.exportErrors);
  const result = useExportStore((s) => s.result);
  const setExporting = useExportStore((s) => s.setExporting);
  const setProgress = useExportStore((s) => s.setProgress);
  const addExportError = useExportStore((s) => s.addExportError);
  const setResult = useExportStore((s) => s.setResult);

  // Load categories on mount
  useEffect(() => {
    setCategoriesLoading(true);
    scanCategories()
      .then((cats) => setCategories(cats))
      .catch((err) => {
        console.error("Failed to scan categories:", err);
        setCategoriesLoading(false);
      });
  }, [setCategoriesLoading, setCategories]);

  // Subscribe to export events on mount
  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    onExportProgress((p) => {
      if (!cancelled) {
        setProgress(p.current, p.total, p.entity_name);
        if (p.error) {
          addExportError(p.error);
        }
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    });

    onExportDone((r) => {
      if (!cancelled) {
        setResult(r);
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    });

    return () => {
      cancelled = true;
      for (const fn of unlisteners) fn();
    };
  }, [setProgress, addExportError, setResult]);

  const category = categories[activeCategory];
  const filtered = category
    ? category.entities.filter(
        (e) => {
          if (hideNpcVariants && e.is_npc_or_internal) return false;
          if (search === "") return true;
          const q = search.toLowerCase();
          return e.name.toLowerCase().includes(q) ||
            (e.display_name?.toLowerCase().includes(q) ?? false);
        },
      )
    : [];

  const selectedInCategory = filtered.filter((e) => selected.has(e.id)).length;
  const totalSelected = selected.size;

  const canExport = totalSelected > 0 && outputDir !== null && !exporting;

  const progressFraction = progressTotal > 0 ? progress / progressTotal : 0;

  const handleExport = () => {
    const allEntities = categories.flatMap((c) => c.entities);
    const selectedEntities = allEntities.filter((e) => selected.has(e.id));
    const request: ExportRequest = {
      record_ids: selectedEntities.map((e) => e.id),
      names: selectedEntities.map((e) => e.name),
      output_dir: outputDir!,
      lod,
      mip,
      include_textures: includeTextures,
      include_interior: includeInterior,
      include_normals: includeNormals,
      include_lights: includeLights,
      include_tangents: includeTangents,
      include_materials: includeMaterials,
      experimental_textures: experimentalTextures,
      threads,
    };
    setExporting(true);
    startExport(request).catch((err) => {
      console.error("Export failed:", err);
      setExporting(false);
    });
  };

  const handleCancel = () => {
    cancelExport().catch((err) => console.error("Cancel failed:", err));
  };

  const handleBrowse = () => {
    browseOutputDir().then((dir) => {
      if (dir !== null) setOutputDir(dir);
    });
  };

  return (
    <div className="flex-1 flex overflow-hidden relative">
      {/* ── Export overlay ── */}
      {exporting && (
        <div className="absolute inset-0 z-10 bg-bg/80 backdrop-blur-sm flex items-center justify-center">
          <div className="w-[360px] bg-bg-alt border border-border rounded-lg p-6 flex flex-col gap-4 shadow-lg">
            <h3 className="text-sm font-semibold text-text">
              Exporting models...
            </h3>

            <div className="flex flex-col gap-1.5">
              <div className="w-full bg-surface rounded-full h-2 overflow-hidden">
                <div
                  className="bg-accent h-full rounded-full transition-all duration-300"
                  style={{ width: `${progressFraction * 100}%` }}
                />
              </div>
              <div className="flex items-center justify-between">
                <p className="text-[11px] text-text-dim truncate flex-1">
                  {progressLabel}
                </p>
                <span className="text-[11px] text-text-faint tabular-nums ml-2 shrink-0">
                  {progress}/{progressTotal}
                </span>
              </div>
            </div>

            {exportErrors.length > 0 && (
              <div className="max-h-24 overflow-y-auto rounded bg-danger/5 border border-danger/20 px-3 py-2">
                {exportErrors.map((err, i) => (
                  <p key={i} className="text-[11px] text-danger/80 leading-relaxed">
                    {err}
                  </p>
                ))}
              </div>
            )}

            <button
              onClick={handleCancel}
              className="w-full py-2 rounded-md text-xs font-medium bg-danger/15 text-danger
                         hover:bg-danger/25 transition-colors cursor-pointer"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* ── Left: Entity Selection ── */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Toolbar */}
        <div className="flex items-center gap-2 px-3 border-b border-border bg-bg-alt shrink-0" style={{ height: "var(--toolbar-height)" }}>
          <input
            type="text"
            placeholder="Search entities..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="flex-1 bg-surface rounded-md px-3 py-1.5 text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring"
          />
          <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
            <input
              type="checkbox"
              checked={hideNpcVariants}
              onChange={(e) => setHideNpcVariants(e.target.checked)}
              className="accent-accent w-3 h-3"
            />
            <span className="text-[11px] text-text-dim">Hide NPC</span>
          </label>
          <button
            onClick={() => selectAllFiltered(filtered.map((e) => e.id))}
            className="text-[11px] text-text-dim hover:text-text px-2 py-1 rounded-md hover:bg-surface/60
                       transition-colors cursor-pointer shrink-0"
          >
            All
          </button>
          <button
            onClick={() => clearFiltered(filtered.map((e) => e.id))}
            className="text-[11px] text-text-dim hover:text-text px-2 py-1 rounded-md hover:bg-surface/60
                       transition-colors cursor-pointer shrink-0"
          >
            None
          </button>
          <span className="text-[11px] text-text-faint tabular-nums shrink-0">
            {selectedInCategory}/{filtered.length}
          </span>
          <div className="flex gap-1 shrink-0">
            {categoriesLoading ? (
              <span className="text-xs text-text-dim px-3 py-1">
                Scanning...
              </span>
            ) : (
              categories.map((cat, i) => (
                <button
                  key={cat.name}
                  onClick={() => setActiveCategory(i)}
                  className={`
                    px-3 py-1 rounded-md text-xs font-medium transition-colors cursor-pointer
                    ${
                      i === activeCategory
                        ? "bg-primary/15 text-text"
                        : "bg-surface text-text-dim hover:bg-surface-hi hover:text-text"
                    }
                  `}
                >
                  {cat.name}
                  <span className="ml-1.5 opacity-60">
                    {cat.entities.length}
                  </span>
                </button>
              ))
            )}
          </div>
        </div>

        {/* Entity list */}
        <div className="flex-1 overflow-y-auto px-1">
          {filtered.map((entity) => {
            const isSelected = selected.has(entity.id);
            return (
              <label
                key={entity.id}
                className={`
                  flex items-center gap-2.5 px-3 py-[5px] rounded-md cursor-pointer text-xs
                  transition-colors select-none
                  ${isSelected ? "bg-primary/8 text-text" : "text-text-sub hover:bg-surface/40"}
                `}
              >
                <input
                  type="checkbox"
                  checked={isSelected}
                  onChange={() => toggleEntity(entity.id)}
                  className="accent-accent w-3.5 h-3.5 rounded shrink-0"
                />
                <span className="truncate">
                  {entity.display_name ?? entity.name}
                  {entity.display_name && (
                    <span className="ml-1.5 text-text-faint text-[10px]">
                      {entity.name}
                    </span>
                  )}
                </span>
              </label>
            );
          })}
        </div>
      </div>

      {/* ── Right: Options Panel ── */}
      <ResizeHandle width={optionsWidth} onResize={setOptionsWidth} side="left" min={200} max={400} />
      <div className="shrink-0 border-l border-border bg-bg-alt flex flex-col" style={{ width: optionsWidth }}>
        <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-5">
          <h2 className="text-xs font-semibold text-primary uppercase tracking-wider">
            Export Options
          </h2>

          {/* LOD */}
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-text-sub">LOD Level</span>
              <span className="text-xs text-text-faint tabular-nums">
                {lod}
              </span>
            </div>
            <input
              type="range"
              min={0}
              max={4}
              value={lod}
              onChange={(e) => setLod(Number(e.target.value))}
              className="w-full accent-accent h-1.5"
            />
            <div className="flex justify-between text-[10px] text-text-faint">
              <span>Highest</span>
              <span>Lowest</span>
            </div>
          </div>

          {/* Mip */}
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-text-sub">Texture Mip</span>
              <span className="text-xs text-text-faint tabular-nums">
                {mip}
              </span>
            </div>
            <input
              type="range"
              min={0}
              max={6}
              value={mip}
              onChange={(e) => setMip(Number(e.target.value))}
              className="w-full accent-accent h-1.5"
            />
            <div className="flex justify-between text-[10px] text-text-faint">
              <span>Full res</span>
              <span>Smallest</span>
            </div>
          </div>

          {/* Threads */}
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-text-sub">Threads</span>
              <span className="text-xs text-text-faint tabular-nums">
                {threads === 0 ? "Auto" : threads}
              </span>
            </div>
            <input
              type="range"
              min={0}
              max={navigator.hardwareConcurrency || 16}
              value={threads}
              onChange={(e) => setThreads(Number(e.target.value))}
              className="w-full accent-accent h-1.5"
            />
            <div className="flex justify-between text-[10px] text-text-faint">
              <span>Auto</span>
              <span>{navigator.hardwareConcurrency || 16}</span>
            </div>
          </div>

          {/* Toggles */}
          <div className="flex flex-col gap-2">
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeTextures}
                onChange={(e) => setIncludeTextures(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include textures
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeInterior}
                onChange={(e) => setIncludeInterior(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include interiors
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeNormals}
                onChange={(e) => setIncludeNormals(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include normals
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeLights}
                onChange={(e) => setIncludeLights(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include lights
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeTangents}
                onChange={(e) => setIncludeTangents(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include tangents
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeMaterials}
                onChange={(e) => setIncludeMaterials(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include materials
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group" title="Apply normal/roughness textures even when UV spaces may not match. Can cause specular noise on some materials.">
              <input
                type="checkbox"
                checked={experimentalTextures}
                onChange={(e) => setExperimentalTextures(e.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Experimental textures
              </span>
            </label>
          </div>

          {/* Output directory */}
          <div className="flex flex-col gap-1.5">
            <span className="text-xs text-text-sub">Output directory</span>
            <button
              onClick={handleBrowse}
              className="flex items-center gap-2 w-full bg-surface/50 border border-border rounded-md
                         px-3 py-2 text-xs text-left cursor-pointer hover:bg-surface/80 transition-colors"
            >
              <svg
                className="w-3.5 h-3.5 text-text-faint shrink-0"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={2}
              >
                <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
              </svg>
              <span
                className={outputDir ? "text-text truncate" : "text-text-faint"}
              >
                {outputDir ?? "Choose folder..."}
              </span>
            </button>
          </div>
        </div>

        {/* Bottom action area */}
        <div className="p-4 border-t border-border flex flex-col gap-3">
          {/* Result summary */}
          {!exporting && result && (
            <div className="flex flex-col gap-1.5">
              <p
                className={`text-[11px] ${result.errors > 0 ? "text-warning" : "text-success"}`}
              >
                {result.errors > 0
                  ? `Exported ${result.success} model${result.success !== 1 ? "s" : ""}, ${result.errors} failed`
                  : `Exported ${result.success} model${result.success !== 1 ? "s" : ""} successfully`}
              </p>
              {exportErrors.length > 0 && (
                <div className="max-h-20 overflow-y-auto rounded bg-danger/5 border border-danger/20 px-2 py-1.5">
                  {exportErrors.map((err, i) => (
                    <p key={i} className="text-[10px] text-danger/80 leading-relaxed">
                      {err}
                    </p>
                  ))}
                </div>
              )}
            </div>
          )}

          <button
            onClick={handleExport}
            disabled={!canExport}
            className={`
              w-full py-2 rounded-md text-xs font-medium transition-colors cursor-pointer
              ${
                canExport
                  ? "bg-accent text-on-accent hover:brightness-110"
                  : "bg-surface text-text-faint cursor-not-allowed"
              }
            `}
          >
            {totalSelected === 0
              ? "Select entities to export"
              : outputDir === null
                ? "Choose output directory"
                : `Export ${totalSelected} model${totalSelected !== 1 ? "s" : ""}`}
          </button>
        </div>
      </div>
    </div>
  );
}
