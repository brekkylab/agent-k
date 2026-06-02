// Model catalog: the set of selectable LLMs (grouped by capability tier), each
// agent's recommended models, and which providers this server actually has
// keys for. Backed by GET /models. Used to render the composer model picker.

import { request } from './client';

export type ModelTier = 'light' | 'standard' | 'max';

export interface CatalogModel {
  id: string; // "provider/model-id"
  label: string;
  tier: ModelTier;
  /** True when this server has the provider's API key configured. */
  available: boolean;
}

export interface AgentRecommendation {
  agentType: string;
  /** Provider-priority chain; `chain[0]` is primary, every entry is highlighted. */
  chain: string[];
  /** What "recommended" resolves to right now given provider availability. */
  resolvedModel: string;
}

export interface ModelCatalog {
  models: CatalogModel[];
  agents: AgentRecommendation[];
}

interface BackendModelEntry {
  id: string;
  label: string;
  tier: ModelTier;
  available: boolean;
}

interface BackendAgentRecommendation {
  agent_type: string;
  chain: string[];
  resolved_model: string;
}

interface BackendModelCatalog {
  models: BackendModelEntry[];
  agents: BackendAgentRecommendation[];
}

export async function getModelCatalog(projectRef?: string): Promise<ModelCatalog> {
  const qs = projectRef ? `?project_ref=${encodeURIComponent(projectRef)}` : '';
  const raw = await request<BackendModelCatalog>(`/models${qs}`);
  return {
    models: raw.models,
    agents: raw.agents.map((a) => ({
      agentType: a.agent_type,
      chain: a.chain,
      resolvedModel: a.resolved_model,
    })),
  };
}

const TIER_LABELS: Record<ModelTier, string> = {
  light: '경량',
  standard: '표준',
  max: '고성능',
};

export function tierLabel(tier: ModelTier): string {
  return TIER_LABELS[tier];
}

export function modelLabel(catalog: ModelCatalog | undefined, id: string): string {
  return catalog?.models.find((m) => m.id === id)?.label ?? id;
}

/** The recommendation entry for an agent type, if present. */
export function recommendationFor(
  catalog: ModelCatalog | undefined,
  agentType: string,
): AgentRecommendation | undefined {
  return catalog?.agents.find((a) => a.agentType === agentType);
}
