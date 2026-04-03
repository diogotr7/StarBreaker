import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { DiscoverResult } from "../lib/commands";
import { tauriStorage } from "../lib/tauri-storage";

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

  // Persisted: custom paths the user has opened before
  recentCustomPaths: string[];
  addRecentCustomPath: (path: string) => void;

  setLoading: (loading: boolean) => void;
  setProgress: (fraction: number, message: string) => void;
  setLoaded: (path: string, source: string, entryCount: number) => void;
  setError: (error: string) => void;
  clearError: () => void;
}

type PersistedAppState = Pick<AppState, "recentCustomPaths">;

export const useAppStore = create<AppState>()(
  persist<AppState, [], [], PersistedAppState>(
    (set) => ({
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

      recentCustomPaths: [],
      addRecentCustomPath: (path) =>
        set((s) => ({
          recentCustomPaths: [
            path,
            ...s.recentCustomPaths.filter((p) => p !== path),
          ].slice(0, 10),
        })),

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
      setError: (error) =>
        set({ error, loading: false, loadingProgress: 0, loadingMessage: "" }),
      clearError: () => set({ error: null }),
    }),
    {
      name: "app",
      storage: tauriStorage,
      partialize: (s) => ({
        recentCustomPaths: s.recentCustomPaths,
      }),
    },
  ),
);
