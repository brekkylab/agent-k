import { create } from "zustand";
import type { ProviderName } from "./constants";
import type { ApiSpeedwagon, ApiSource } from "./types";
import { getSpeedwagons as apiGetSpeedwagons, getSources as apiGetSources } from "./api";

// ============================================================
// Zustand State Boundary
// ============================================================
// Backend (API)       | Zustand (UI state + read-only list cache)
// --------------------|------------------------------------------
// Provider Profiles   | —
// Agents              | —
// Sessions list       | —
// Session messages    | —
// Sources             | sources (캐시)
// Speedwagons         | speedwagons (캐시)
// Session speedwagon/source relationships | —
// —                   | activeSessionId
// —                   | selectedProvider / selectedModel (pending session용)
// —                   | pendingSpeedwagonIds (pending session용)
// ============================================================

// Promise dedup: 동시 호출 시 동일 Promise 재사용
let speedwagonsFetchPromise: Promise<void> | null = null;
let sourcesFetchPromise: Promise<void> | null = null;

interface AppState {
  // Active session (UI state)
  activeSessionId: string | null;
  setActiveSession: (id: string | null) => void;
  sessionListVersion: number;
  bumpSessionListVersion: () => void;

  // Speedwagons cache (API mirror)
  speedwagons: ApiSpeedwagon[];
  speedwagonsLoading: boolean;
  fetchSpeedwagons: () => Promise<void>;

  // Sources cache (API mirror)
  sources: ApiSource[];
  sourcesLoading: boolean;
  fetchSources: () => Promise<void>;

  // Pending session model selection (before session is created)
  selectedProvider: ProviderName | null;
  selectedModel: string | null;
  selectedProfileId: string | null;
  setSelectedModel: (provider: ProviderName, model: string, profileId: string) => void;

  // Pending session speedwagon selection (before session is created)
  pendingSpeedwagonIds: string[];
  setPendingSpeedwagonIds: (ids: string[]) => void;

  // Pending first message (set in NewChatWelcome, consumed by ApiRuntimeProvider)
  pendingMessage: string | null;
  setPendingMessage: (msg: string | null) => void;
}

export const useAppStore = create<AppState>((set) => ({
  // Active session
  activeSessionId: null,
  setActiveSession: (id) => set({ activeSessionId: id }),
  sessionListVersion: 0,
  bumpSessionListVersion: () => set((s) => ({ sessionListVersion: s.sessionListVersion + 1 })),

  // Speedwagons cache
  speedwagons: [],
  speedwagonsLoading: false,
  fetchSpeedwagons: () => {
    if (speedwagonsFetchPromise) return speedwagonsFetchPromise;
    set({ speedwagonsLoading: true });
    speedwagonsFetchPromise = apiGetSpeedwagons()
      .then((data) => set({ speedwagons: data }))
      .catch(() => {})
      .finally(() => {
        set({ speedwagonsLoading: false });
        speedwagonsFetchPromise = null;
      });
    return speedwagonsFetchPromise;
  },

  // Sources cache
  sources: [],
  sourcesLoading: false,
  fetchSources: () => {
    if (sourcesFetchPromise) return sourcesFetchPromise;
    set({ sourcesLoading: true });
    sourcesFetchPromise = apiGetSources()
      .then((data) => set({ sources: data }))
      .catch(() => {})
      .finally(() => {
        set({ sourcesLoading: false });
        sourcesFetchPromise = null;
      });
    return sourcesFetchPromise;
  },

  // Pending session model selection
  selectedProvider: null,
  selectedModel: null,
  selectedProfileId: null,
  setSelectedModel: (provider, model, profileId) =>
    set({ selectedProvider: provider, selectedModel: model, selectedProfileId: profileId }),

  // Pending session speedwagon selection
  pendingSpeedwagonIds: [],
  setPendingSpeedwagonIds: (ids) => set({ pendingSpeedwagonIds: ids }),

  // Pending first message
  pendingMessage: null,
  setPendingMessage: (msg) => set({ pendingMessage: msg }),
}));
