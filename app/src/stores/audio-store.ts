import { create } from "zustand";
import type {
  AudioEntityResult,
  AudioBankResult,
  AudioTriggerDetail,
  AudioSoundResult,
} from "../lib/commands";
import {
  audioInit,
  audioSearchEntities,
  audioSearchTriggers,
  audioListBanks,
  audioBankTriggers,
  audioBankMedia,
  audioEntityTriggers,
  audioResolveTrigger,
  audioDecodeWem,
} from "../lib/commands";

type SearchMode = "trigger" | "entity" | "bank";

interface AudioState {
  // Init
  isInitialized: boolean;
  isInitializing: boolean;
  triggerCount: number;
  bankCount: number;

  // Search
  searchQuery: string;
  searchMode: SearchMode;
  isSearching: boolean;
  searchSeq: number;

  // Column data
  entities: AudioEntityResult[];
  selectedEntity: string | null;
  banks: AudioBankResult[];
  selectedBank: string | null;
  triggers: AudioTriggerDetail[];
  selectedTrigger: string | null;
  sounds: AudioSoundResult[];

  // Playback
  currentSound: AudioSoundResult | null;
  blobUrl: string | null;
  isPlaying: boolean;
  progress: number;
  duration: number;

  // Error
  error: string | null;

  // Actions
  init: () => Promise<void>;
  setSearchMode: (mode: SearchMode) => void;
  search: (query: string) => Promise<void>;
  selectEntity: (name: string) => Promise<void>;
  selectBank: (name: string) => Promise<void>;
  selectTrigger: (triggerName: string) => Promise<void>;
  playSound: (sound: AudioSoundResult) => Promise<void>;
  stopSound: () => void;
  setProgress: (progress: number, duration: number) => void;
  setPlaybackEnded: () => void;
}

export const useAudioStore = create<AudioState>((set, get) => ({
  isInitialized: false,
  isInitializing: false,
  triggerCount: 0,
  bankCount: 0,

  searchQuery: "",
  searchMode: "bank",
  isSearching: false,
  searchSeq: 0,

  entities: [],
  selectedEntity: null,
  banks: [],
  selectedBank: null,
  triggers: [],
  selectedTrigger: null,
  sounds: [],

  currentSound: null,
  blobUrl: null,
  isPlaying: false,
  progress: 0,
  duration: 0,

  error: null,

  init: async () => {
    if (get().isInitialized || get().isInitializing) return;
    set({ isInitializing: true, error: null });
    try {
      const result = await audioInit();
      set({
        isInitialized: true,
        isInitializing: false,
        triggerCount: result.trigger_count,
        bankCount: result.bank_count,
      });
      // Default mode is bank — load all banks
      const banks = await audioListBanks();
      set({ banks });
    } catch (e) {
      set({ isInitializing: false, error: String(e) });
    }
  },

  setSearchMode: (mode) => {
    set({
      searchMode: mode,
      searchQuery: "",
      entities: [],
      selectedEntity: null,
      banks: [],
      selectedBank: null,
      triggers: [],
      selectedTrigger: null,
      sounds: [],
    });
    if (mode === "bank") {
      audioListBanks().then((banks) => set({ banks }));
    } else if (mode === "trigger") {
      audioSearchTriggers("").then((results) => {
        set({
          triggers: results.map((r) => ({
            trigger_name: r.trigger_name,
            bank_name: r.bank_name,
            duration_type: r.duration_type,
            sound_count: 0,
          })),
        });
      });
    }
  },

  search: async (query) => {
    const seq = get().searchSeq + 1;
    set({ searchQuery: query, isSearching: true, error: null, searchSeq: seq });
    try {
      const { searchMode } = get();
      if (searchMode === "entity") {
        if (!query.trim()) {
          set({ isSearching: false, entities: [], selectedEntity: null, triggers: [], selectedTrigger: null, sounds: [] });
          return;
        }
        const entities = await audioSearchEntities(query);
        if (get().searchSeq !== seq) return;
        set({ entities, isSearching: false, selectedEntity: null, triggers: [], selectedTrigger: null, sounds: [] });
      } else if (searchMode === "trigger") {
        const results = await audioSearchTriggers(query);
        if (get().searchSeq !== seq) return;
        set({
          triggers: results.map((r) => ({
            trigger_name: r.trigger_name,
            bank_name: r.bank_name,
            duration_type: r.duration_type,
            sound_count: 0,
          })),
          isSearching: false,
          entities: [],
          selectedEntity: null,
          selectedTrigger: null,
          sounds: [],
        });
      } else {
        // Bank mode — filter the loaded banks client-side
        const banks = await audioListBanks();
        if (get().searchSeq !== seq) return;
        const q = query.toLowerCase();
        set({
          banks: q ? banks.filter((b) => b.name.toLowerCase().includes(q)) : banks,
          isSearching: false,
          selectedBank: null,
          triggers: [],
          selectedTrigger: null,
          sounds: [],
        });
      }
    } catch (e) {
      if (get().searchSeq !== seq) return;
      set({ isSearching: false, error: String(e) });
    }
  },

  selectEntity: async (name) => {
    set({ selectedEntity: name, selectedTrigger: null, sounds: [], triggers: [], error: null });
    try {
      const triggers = await audioEntityTriggers(name);
      set({ triggers });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  selectBank: async (name) => {
    set({ selectedBank: name, selectedTrigger: null, sounds: [], triggers: [], error: null });
    try {
      const [triggers, media] = await Promise.all([
        audioBankTriggers(name),
        audioBankMedia(name),
      ]);
      set({ triggers, sounds: media });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  selectTrigger: async (triggerName) => {
    const prevSounds = get().sounds;
    set({ selectedTrigger: triggerName, sounds: [], error: null });
    try {
      const sounds = await audioResolveTrigger(triggerName);
      if (sounds.length > 0) {
        set({ sounds });
      } else if (get().searchMode === "bank" && get().selectedBank) {
        // Event resolution returned nothing (common for music banks where
        // events live in a different bank). Keep showing all bank media.
        set({ sounds: prevSounds });
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },

  playSound: async (sound) => {
    const prev = get().blobUrl;
    if (prev) URL.revokeObjectURL(prev);

    set({ currentSound: sound, blobUrl: null, isPlaying: false, progress: 0, duration: 0, error: null });
    try {
      const bytes = await audioDecodeWem(sound.media_id, sound.source_type, sound.bank_name);
      const blob = new Blob([new Uint8Array(bytes)], { type: "audio/ogg" });
      const url = URL.createObjectURL(blob);

      set({ blobUrl: url, isPlaying: true });
      window.dispatchEvent(new CustomEvent("audio-play", { detail: { url } }));
    } catch (e) {
      set({ isPlaying: false, currentSound: null, error: String(e) });
    }
  },

  stopSound: () => {
    const { blobUrl } = get();
    if (blobUrl) URL.revokeObjectURL(blobUrl);
    window.dispatchEvent(new CustomEvent("audio-stop"));
    set({ isPlaying: false, progress: 0, blobUrl: null });
  },

  setProgress: (progress, duration) => set({ progress, duration }),

  setPlaybackEnded: () => set({ isPlaying: false, progress: 0 }),
}));
