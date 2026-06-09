import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { AgentId } from '@/domain/agentSurfaces';

// Per-project, per-agent model preference, persisted to localStorage so a
// user's pick for each agent surface survives reloads — kept separate per
// project. A missing entry (or null) means "recommended" for that agent,
// resolved by the backend at agent-build time. This is the lightweight,
// client-only stand-in for a server-side per-user default.
type AgentModels = Partial<Record<AgentId, string | null>>;

interface ModelPrefsState {
  // project key (route slug) → agent → model id
  byProject: Record<string, AgentModels>;
  setModel: (projectKey: string, agentId: AgentId, model: string | null) => void;
}

export const useModelPrefsStore = create<ModelPrefsState>()(
  persist(
    (set) => ({
      byProject: {},
      setModel: (projectKey, agentId, model) =>
        set((s) => ({
          byProject: {
            ...s.byProject,
            [projectKey]: { ...s.byProject[projectKey], [agentId]: model },
          },
        })),
    }),
    { name: 'cowork-model-prefs' },
  ),
);
