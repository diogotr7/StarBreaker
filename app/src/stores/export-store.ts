import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { CategoryDto, ExportDone } from "../lib/commands";
import { tauriStorage } from "../lib/tauri-storage";

interface ExportState {
  // Categories
  categories: CategoryDto[];
  categoriesLoading: boolean;
  activeCategory: number;
  setActiveCategory: (index: number) => void;
  setCategories: (categories: CategoryDto[]) => void;
  setCategoriesLoading: (loading: boolean) => void;

  // Selection (set of entity IDs across all categories)
  selected: Set<string>;
  toggleEntity: (id: string) => void;
  selectAllFiltered: (ids: string[]) => void;
  clearFiltered: (ids: string[]) => void;

  // Search & filters
  search: string;
  setSearch: (query: string) => void;
  hideNpcVariants: boolean;
  setHideNpcVariants: (v: boolean) => void;

  // Export options
  lod: number;
  mip: number;
  materialMode: string;
  format: string;
  includeAttachments: boolean;
  includeInterior: boolean;
  includeLights: boolean;
  threads: number;
  outputDir: string | null;
  setLod: (v: number) => void;
  setMip: (v: number) => void;
  setMaterialMode: (v: string) => void;
  setFormat: (v: string) => void;
  setIncludeAttachments: (v: boolean) => void;
  setIncludeInterior: (v: boolean) => void;
  setIncludeLights: (v: boolean) => void;
  setThreads: (v: number) => void;
  setOutputDir: (dir: string | null) => void;

  // Export progress
  exporting: boolean;
  progress: number;
  progressTotal: number;
  progressLabel: string;
  exportErrors: string[];
  result: ExportDone | null;
  setExporting: (v: boolean) => void;
  setProgress: (current: number, total: number, label: string) => void;
  addExportError: (msg: string) => void;
  setResult: (result: ExportDone | null) => void;
  deselectIds: (ids: string[]) => void;
}

type PersistedExportState = Pick<
  ExportState,
  | "lod"
  | "mip"
  | "materialMode"
  | "includeAttachments"
  | "includeInterior"
  | "includeLights"
  | "threads"
  | "outputDir"
  | "hideNpcVariants"
>;

export const useExportStore = create<ExportState>()(
  persist<ExportState, [], [], PersistedExportState>(
    (set) => ({
  categories: [],
  categoriesLoading: false,
  activeCategory: 0,
  setActiveCategory: (index) => set({ activeCategory: index }),
  setCategories: (categories) => set({ categories, categoriesLoading: false }),
  setCategoriesLoading: (loading) => set({ categoriesLoading: loading }),

  selected: new Set(),
  toggleEntity: (id) =>
    set((s) => {
      const next = new Set(s.selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { selected: next };
    }),
  selectAllFiltered: (ids) =>
    set((s) => {
      const next = new Set(s.selected);
      for (const id of ids) next.add(id);
      return { selected: next };
    }),
  clearFiltered: (ids) =>
    set((s) => {
      const next = new Set(s.selected);
      for (const id of ids) next.delete(id);
      return { selected: next };
    }),

  search: "",
  setSearch: (query) => set({ search: query }),
  hideNpcVariants: true,
  setHideNpcVariants: (v) => set({ hideNpcVariants: v }),

  lod: 1,
  mip: 2,
  materialMode: "textures",
  format: "glb",
  includeAttachments: true,
  includeInterior: true,
  includeLights: true,
  threads: 0,
  outputDir: null,
  setLod: (v) => set({ lod: v }),
  setMip: (v) => set({ mip: v }),
  setMaterialMode: (v) => set({ materialMode: v }),
  setFormat: (v) => set({ format: v }),
  setIncludeAttachments: (v) => set({ includeAttachments: v }),
  setIncludeInterior: (v) => set({ includeInterior: v }),
  setIncludeLights: (v) => set({ includeLights: v }),
  setThreads: (v) => set({ threads: v }),
  setOutputDir: (dir) => set({ outputDir: dir }),

  exporting: false,
  progress: 0,
  progressTotal: 0,
  progressLabel: "",
  exportErrors: [],
  result: null,
  setExporting: (v) =>
    set({ exporting: v, ...(v ? { result: null, exportErrors: [] } : {}) }),
  setProgress: (current, total, label) =>
    set({ progress: current, progressTotal: total, progressLabel: label }),
  addExportError: (msg) =>
    set((s) => ({ exportErrors: [...s.exportErrors, msg] })),
  setResult: (result) => set({ result, exporting: false }),
  deselectIds: (ids) =>
    set((s) => {
      const next = new Set(s.selected);
      for (const id of ids) next.delete(id);
      return { selected: next };
    }),
    }),
    {
      name: "export",
      storage: tauriStorage,
      partialize: (s) => ({
        lod: s.lod,
        mip: s.mip,
        materialMode: s.materialMode,
        includeAttachments: s.includeAttachments,
        includeInterior: s.includeInterior,
        includeLights: s.includeLights,
        threads: s.threads,
        outputDir: s.outputDir,
        hideNpcVariants: s.hideNpcVariants,
      }),
    },
  ),
);
