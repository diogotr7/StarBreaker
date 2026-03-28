import { create } from "zustand";
import type { DiscoverResult } from "../lib/commands";

export type AppMode = "p4k" | "datacore" | "export" | "audio";

interface AppState {
  mode: AppMode;
  setMode: (mode: AppMode) => void;

  hasData: boolean;
  loading: boolean;
  loadingProgress: number;
  loadingMessage: string;
  error: string | null;

  p4kPath: string | null;
  p4kSource: string | null;
  entryCount: number;

  discoveries: DiscoverResult[];
  setDiscoveries: (discoveries: DiscoverResult[]) => void;

  setLoading: (loading: boolean) => void;
  setProgress: (fraction: number, message: string) => void;
  setLoaded: (path: string, source: string, entryCount: number) => void;
  setError: (error: string) => void;
  clearError: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  mode: "p4k",
  setMode: (mode) => set({ mode }),

  hasData: false,
  loading: false,
  loadingProgress: 0,
  loadingMessage: "",
  error: null,

  p4kPath: null,
  p4kSource: null,
  entryCount: 0,

  discoveries: [],
  setDiscoveries: (discoveries) => set({ discoveries }),

  setLoading: (loading) => set({ loading }),
  setProgress: (fraction, message) =>
    set({ loadingProgress: fraction, loadingMessage: message }),
  setLoaded: (path, source, entryCount) =>
    set({
      hasData: true,
      loading: false,
      error: null,
      p4kPath: path,
      p4kSource: source,
      entryCount,
      loadingProgress: 1,
      loadingMessage: "Done",
    }),
  setError: (error) => set({ error, loading: false, loadingProgress: 0, loadingMessage: "" }),
  clearError: () => set({ error: null }),
}));
