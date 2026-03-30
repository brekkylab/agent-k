import { create } from "zustand";
import type { ProviderName } from "./constants";

// ============================================================
// Zustand State Boundary
// ============================================================
// Backend (API)       | Zustand (UI state)
// --------------------|---------------------------
// Provider Profiles   | —
// Agents              | —
// Sessions list       | —
// Session messages    | —
// Sources             | —
// Speedwagons         | —
// Session speedwagon/source relationships | —
// —                   | activeSessionId
// —                   | selectedProvider / selectedModel (pending session용)
// —                   | pendingSpeedwagonIds (pending session용)
// ============================================================

interface AppState {
  // Active session (UI state)
  activeSessionId: string | null;
  setActiveSession: (id: string | null) => void;
  sessionListVersion: number;
  bumpSessionListVersion: () => void;
  speedwagonListVersion: number;
  bumpSpeedwagonListVersion: () => void;

  // Pending session model selection (before session is created)
  selectedProvider: ProviderName | null;
  selectedModel: string | null;
  selectedProfileId: string | null;
  setSelectedModel: (provider: ProviderName, model: string, profileId: string) => void;

  // Pending session speedwagon selection (before session is created)
  pendingSpeedwagonIds: string[];
  setPendingSpeedwagonIds: (ids: string[]) => void;
}

export const useAppStore = create<AppState>((set) => ({
  // Active session
  activeSessionId: null,
  setActiveSession: (id) => set({ activeSessionId: id }),
  sessionListVersion: 0,
  bumpSessionListVersion: () => set((s) => ({ sessionListVersion: s.sessionListVersion + 1 })),
  speedwagonListVersion: 0,
  bumpSpeedwagonListVersion: () => set((s) => ({ speedwagonListVersion: s.speedwagonListVersion + 1 })),

  // Pending session model selection
  selectedProvider: null,
  selectedModel: null,
  selectedProfileId: null,
  setSelectedModel: (provider, model, profileId) =>
    set({ selectedProvider: provider, selectedModel: model, selectedProfileId: profileId }),

  // Pending session speedwagon selection
  pendingSpeedwagonIds: [],
  setPendingSpeedwagonIds: (ids) => set({ pendingSpeedwagonIds: ids }),
}));
