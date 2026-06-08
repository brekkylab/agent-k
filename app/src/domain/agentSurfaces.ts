export type AgentId = 'coworker' | 'speedwagon' | 'deep-research' | 'buddy';

export type AgentSurfaceIcon = 'zap' | 'search' | 'analysis' | 'brainstorm';

// Suggested-prompt shape. The label/seedText copy is i18n'd in the
// `automation` namespace under `agent.<id>.prompts`, resolved via
// `t('agent.<id>.prompts', { returnObjects: true })`.
export interface SuggestedPrompt {
  label: string;
  seedText: string;
}

export interface AgentSurface {
  id: AgentId;
  // Brand name — not translated.
  label: string;
  icon: AgentSurfaceIcon;
}

export const AGENT_SURFACES: readonly AgentSurface[] = [
  { id: 'coworker', label: 'Coworker', icon: 'zap' },
  { id: 'speedwagon', label: 'Speedwagon', icon: 'search' },
  { id: 'deep-research', label: 'Deep Research', icon: 'analysis' },
  { id: 'buddy', label: 'Buddy', icon: 'brainstorm' },
] as const;

export const DEFAULT_AGENT_ID: AgentId = AGENT_SURFACES[0].id;

// `agent_type` (sent to / stored by the backend) is identical to the surface
// `AgentId` — no mapping needed. So `getAgentSurface` doubles as the lookup for
// a stored agent_type, and the selected AgentId is sent as-is.
export function getAgentSurface(id: string | undefined): AgentSurface {
  return AGENT_SURFACES.find((agent) => agent.id === id) ?? AGENT_SURFACES[0];
}
