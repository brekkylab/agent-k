import { describe, expect, it } from 'vitest';
import { AGENT_SURFACES, DEFAULT_AGENT_ID, getAgentSurface } from './agentSurfaces';

describe('agent surface preview registry', () => {
  it('keeps the default agent as the first configured surface', () => {
    expect(DEFAULT_AGENT_ID).toBe('coworker');
    expect(AGENT_SURFACES[0].id).toBe(DEFAULT_AGENT_ID);
  });

  it('falls back to the default surface for an unknown transient id', () => {
    expect(getAgentSurface('not-real')).toBe(AGENT_SURFACES[0]);
  });

  it('keeps each preview surface complete enough for home and session UI', () => {
    for (const surface of AGENT_SURFACES) {
      expect(surface.label).toBeTruthy();
      expect(surface.icon).toBeTruthy();
    }
  });
});
