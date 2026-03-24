import { create } from "zustand";
import type { Document, Knowledge, SessionDocument } from "./types";
import { DUMMY_DOCUMENTS, DUMMY_KNOWLEDGES } from "./dummy-data";

// ============================================================
// Zustand State Boundary (Phase 5 완료)
// ============================================================
// Backend (API)       | Zustand (UI state)
// --------------------|---------------------------
// Provider Profiles   | —
// Agents              | —
// Sessions list       | —
// Session messages    | —
// —                   | activeSessionId
// —                   | documents[] (mockup)
// —                   | knowledges[] (mockup)
// —                   | sessionLocalData (Knowledge/Document associations per session)
// ============================================================

interface SessionLocalData {
  knowledgeIds: string[];
  sessionDocuments: SessionDocument[];
}

interface AppState {
  // Documents (mockup)
  documents: Document[];
  addDocument: (doc: Document) => void;
  removeDocument: (id: string) => void;

  // Knowledge (mockup)
  knowledges: Knowledge[];
  addKnowledge: (kn: Knowledge) => void;
  updateKnowledge: (id: string, updates: Partial<Knowledge>) => void;
  removeKnowledge: (id: string) => void;
  toggleDocumentInKnowledge: (knowledgeId: string, documentId: string) => void;

  // Active session (UI state)
  activeSessionId: string | null;
  setActiveSession: (id: string | null) => void;
  sessionListVersion: number;
  bumpSessionListVersion: () => void;

  // Per-session local data (Knowledge/Document associations — not in Backend)
  sessionLocalData: Record<string, SessionLocalData>;
  getSessionLocalData: (sessionId: string) => SessionLocalData;
  updateSessionKnowledge: (sessionId: string, knowledgeIds: string[]) => void;
  addSessionDocument: (sessionId: string, doc: SessionDocument) => void;
  removeSessionDocument: (sessionId: string, index: number) => void;
  removeSessionLocalData: (sessionId: string) => void;
}

const DEFAULT_SESSION_LOCAL: SessionLocalData = {
  knowledgeIds: [],
  sessionDocuments: [],
};

export const useAppStore = create<AppState>((set, get) => ({
  // Documents (mockup)
  documents: DUMMY_DOCUMENTS,
  addDocument: (doc) =>
    set((s) => ({ documents: [...s.documents, doc] })),
  removeDocument: (id) =>
    set((s) => ({
      documents: s.documents.filter((d) => d.id !== id),
      knowledges: s.knowledges.map((k) => ({
        ...k,
        documentIds: k.documentIds.filter((did) => did !== id),
      })),
    })),

  // Knowledge (mockup)
  knowledges: DUMMY_KNOWLEDGES,
  addKnowledge: (kn) =>
    set((s) => ({ knowledges: [...s.knowledges, kn] })),
  updateKnowledge: (id, updates) =>
    set((s) => ({
      knowledges: s.knowledges.map((k) =>
        k.id === id ? { ...k, ...updates } : k
      ),
    })),
  removeKnowledge: (id) =>
    set((s) => ({
      knowledges: s.knowledges.filter((k) => k.id !== id),
    })),
  toggleDocumentInKnowledge: (knowledgeId, documentId) =>
    set((s) => ({
      knowledges: s.knowledges.map((k) => {
        if (k.id !== knowledgeId) return k;
        const has = k.documentIds.includes(documentId);
        return {
          ...k,
          documentIds: has
            ? k.documentIds.filter((d) => d !== documentId)
            : [...k.documentIds, documentId],
        };
      }),
    })),

  // Active session
  activeSessionId: null,
  setActiveSession: (id) => set({ activeSessionId: id }),
  sessionListVersion: 0,
  bumpSessionListVersion: () => set((s) => ({ sessionListVersion: s.sessionListVersion + 1 })),

  // Per-session local data
  sessionLocalData: {},
  getSessionLocalData: (sessionId) => {
    return get().sessionLocalData[sessionId] ?? DEFAULT_SESSION_LOCAL;
  },
  updateSessionKnowledge: (sessionId, knowledgeIds) =>
    set((s) => ({
      sessionLocalData: {
        ...s.sessionLocalData,
        [sessionId]: {
          ...(s.sessionLocalData[sessionId] ?? DEFAULT_SESSION_LOCAL),
          knowledgeIds,
        },
      },
    })),
  addSessionDocument: (sessionId, doc) =>
    set((s) => {
      const current = s.sessionLocalData[sessionId] ?? DEFAULT_SESSION_LOCAL;
      return {
        sessionLocalData: {
          ...s.sessionLocalData,
          [sessionId]: {
            ...current,
            sessionDocuments: [...current.sessionDocuments, doc],
          },
        },
      };
    }),
  removeSessionDocument: (sessionId, index) =>
    set((s) => {
      const current = s.sessionLocalData[sessionId] ?? DEFAULT_SESSION_LOCAL;
      return {
        sessionLocalData: {
          ...s.sessionLocalData,
          [sessionId]: {
            ...current,
            sessionDocuments: current.sessionDocuments.filter((_, i) => i !== index),
          },
        },
      };
    }),
  removeSessionLocalData: (sessionId) =>
    set((s) => {
      const { [sessionId]: _, ...rest } = s.sessionLocalData;
      return { sessionLocalData: rest };
    }),
}));
