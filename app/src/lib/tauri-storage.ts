import { LazyStore } from "@tauri-apps/plugin-store";
import type { PersistStorage, StorageValue } from "zustand/middleware";

const store = new LazyStore("settings.json");

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const tauriStorage: PersistStorage<any> = {
  getItem: async (key) => {
    return (await store.get<StorageValue<unknown>>(key)) ?? null;
  },
  setItem: async (key, value) => {
    await store.set(key, value);
  },
  removeItem: async (key) => {
    await store.delete(key);
  },
};
