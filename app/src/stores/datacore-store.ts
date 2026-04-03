import { create } from "zustand";
import type { RecordDto, SearchResultDto } from "../lib/commands";

interface NavHistory {
  stack: string[];
  position: number; // -1 means empty
}

interface DataCoreState {
  // Search
  searchQuery: string;
  searchResults: SearchResultDto[];
  searching: boolean;
  setSearchQuery: (query: string) => void;
  setSearchResults: (results: SearchResultDto[]) => void;
  setSearching: (searching: boolean) => void;

  // Selected record
  selectedRecord: RecordDto | null;
  loadingRecord: boolean;
  setSelectedRecord: (record: RecordDto | null) => void;
  setLoadingRecord: (loading: boolean) => void;

  // Navigation history
  nav: NavHistory;
  navigateTo: (recordId: string) => void;
  goBack: () => string | null;
  goForward: () => string | null;
  canGoBack: () => boolean;
  canGoForward: () => boolean;

  // Export
  saving: boolean;
  setSaving: (saving: boolean) => void;
}

export const useDataCoreStore = create<DataCoreState>((set, get) => ({
  searchQuery: "",
  searchResults: [],
  searching: false,
  setSearchQuery: (query) => set({ searchQuery: query }),
  setSearchResults: (results) => set({ searchResults: results, searching: false }),
  setSearching: (searching) => set({ searching }),

  selectedRecord: null,
  loadingRecord: false,
  setSelectedRecord: (record) => set({ selectedRecord: record, loadingRecord: false }),
  setLoadingRecord: (loading) => set({ loadingRecord: loading }),

  nav: { stack: [], position: -1 },
  navigateTo: (recordId) =>
    set((state) => {
      const nav = { ...state.nav };
      // Truncate forward history
      if (nav.position >= 0) {
        nav.stack = nav.stack.slice(0, nav.position + 1);
      } else {
        nav.stack = [];
      }
      nav.stack.push(recordId);
      nav.position = nav.stack.length - 1;
      return { nav };
    }),
  goBack: () => {
    const { nav } = get();
    if (nav.position > 0) {
      const newPos = nav.position - 1;
      set({ nav: { ...nav, position: newPos } });
      return nav.stack[newPos];
    }
    return null;
  },
  goForward: () => {
    const { nav } = get();
    if (nav.position + 1 < nav.stack.length) {
      const newPos = nav.position + 1;
      set({ nav: { ...nav, position: newPos } });
      return nav.stack[newPos];
    }
    return null;
  },
  canGoBack: () => {
    const { nav } = get();
    return nav.position > 0;
  },
  canGoForward: () => {
    const { nav } = get();
    return nav.position >= 0 && nav.position + 1 < nav.stack.length;
  },

  saving: false,
  setSaving: (saving) => set({ saving }),
}));
