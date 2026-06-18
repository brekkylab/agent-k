// Endpoint client for /automations. Each function calls request<T>() with the
// raw backend shape, then maps through transformers.ts to expose camelCase
// domain types to view code. snake_case never leaks past this module.

import { request } from './client';
import type {
  BackendAutomation,
  BackendAutomationList,
  BackendOccurrenceList,
  BackendRun,
  BackendRunEventList,
  BackendRunList,
  BackendTrigger,
  BackendTriggerList,
  BackendTriggerSpec,
  CreatedTriggerResponse,
} from './backend-types';
import {
  toAutomation,
  toCreatedTrigger,
  toOccurrence,
  toRun,
  toRunEvent,
  toTrigger,
} from './transformers';
import type {
  Automation,
  AutomationId,
  CreatedTrigger,
  Occurrence,
  Run,
  RunEvent,
  RunId,
  Trigger,
  TriggerId,
  TriggerSpec,
} from '@/domain/types';

// ── Automation ─────────────────────────────────────────────────────────────

export interface CreateAutomationInput {
  projectRef: string;
  name: string;
  description?: string | null;
  prompts: string[];
  agentType?: string | null;
  model?: string | null;
}

export interface UpdateAutomationInput {
  name?: string;
  description?: string | null;
  prompts?: string[];
  enabled?: boolean;
  agentType?: string;
  model?: string | null;
}

/** Omit `projectRef` to list across projects (admin only on the backend). */
export async function listAutomations(projectRef?: string): Promise<Automation[]> {
  const qs = projectRef ? `?project_ref=${encodeURIComponent(projectRef)}` : '';
  const res = await request<BackendAutomationList>(`/automations${qs}`);
  return res.items.map(toAutomation);
}

export async function createAutomation(input: CreateAutomationInput): Promise<Automation> {
  const raw = await request<BackendAutomation>('/automations', {
    method: 'POST',
    body: {
      project_ref: input.projectRef,
      name: input.name,
      description: input.description ?? null,
      prompts: input.prompts,
      agent_type: input.agentType ?? null,
      model: input.model ?? null,
    },
  });
  return toAutomation(raw);
}

export async function getAutomation(automationId: AutomationId): Promise<Automation> {
  const raw = await request<BackendAutomation>(`/automations/${automationId}`);
  return toAutomation(raw);
}

export async function updateAutomation(
  automationId: AutomationId,
  patch: UpdateAutomationInput,
): Promise<Automation> {
  const raw = await request<BackendAutomation>(`/automations/${automationId}`, {
    method: 'PATCH',
    body: {
      ...(patch.name !== undefined ? { name: patch.name } : {}),
      ...(patch.description !== undefined ? { description: patch.description } : {}),
      ...(patch.prompts !== undefined ? { prompts: patch.prompts } : {}),
      ...(patch.enabled !== undefined ? { enabled: patch.enabled } : {}),
      ...(patch.agentType !== undefined ? { agent_type: patch.agentType } : {}),
      // null is intentional (→ recommended); only `undefined` means "leave as-is".
      ...(patch.model !== undefined ? { model: patch.model } : {}),
    },
  });
  return toAutomation(raw);
}

export async function deleteAutomation(automationId: AutomationId): Promise<void> {
  await request(`/automations/${automationId}`, { method: 'DELETE' });
}

// ── Trigger ────────────────────────────────────────────────────────────────

export interface UpdateTriggerInput {
  spec?: TriggerSpec;
  enabled?: boolean;
}

export async function listTriggers(automationId: AutomationId): Promise<Trigger[]> {
  const res = await request<BackendTriggerList>(`/automations/${automationId}/triggers`);
  return res.items.map(toTrigger);
}

/**
 * Expand upcoming scheduled fires for all enabled cron triggers in a project,
 * within `[from, to)`. Computed server-side from cron expressions (nothing is
 * persisted). `from`/`to` are ISO instants.
 */
export async function listOccurrences(
  projectRef: string,
  from: string,
  to: string,
): Promise<{ items: Occurrence[]; truncated: boolean }> {
  const qs = new URLSearchParams({ project_ref: projectRef, from, to }).toString();
  const res = await request<BackendOccurrenceList>(`/automations/occurrences?${qs}`);
  return { items: res.items.map(toOccurrence), truncated: res.truncated };
}

/** Returns the created trigger plus the one-time `webhookToken` (webhook only). */
export async function createTrigger(
  automationId: AutomationId,
  spec: TriggerSpec,
): Promise<CreatedTrigger> {
  const raw = await request<CreatedTriggerResponse>(
    `/automations/${automationId}/triggers`,
    { method: 'POST', body: specToBody(spec) },
  );
  return toCreatedTrigger(raw);
}

export async function getTrigger(
  automationId: AutomationId,
  triggerId: TriggerId,
): Promise<Trigger> {
  const raw = await request<BackendTrigger>(
    `/automations/${automationId}/triggers/${triggerId}`,
  );
  return toTrigger(raw);
}

export async function updateTrigger(
  automationId: AutomationId,
  triggerId: TriggerId,
  patch: UpdateTriggerInput,
): Promise<Trigger> {
  const raw = await request<BackendTrigger>(
    `/automations/${automationId}/triggers/${triggerId}`,
    {
      method: 'PATCH',
      body: {
        ...(patch.spec !== undefined ? { spec: specToBody(patch.spec) } : {}),
        ...(patch.enabled !== undefined ? { enabled: patch.enabled } : {}),
      },
    },
  );
  return toTrigger(raw);
}

export async function deleteTrigger(
  automationId: AutomationId,
  triggerId: TriggerId,
): Promise<void> {
  await request(`/automations/${automationId}/triggers/${triggerId}`, {
    method: 'DELETE',
  });
}

// ── Run ────────────────────────────────────────────────────────────────────

export async function listRuns(
  automationId: AutomationId,
  options?: { limit?: number; offset?: number },
): Promise<Run[]> {
  const params = new URLSearchParams();
  if (options?.limit !== undefined) params.set('limit', String(options.limit));
  if (options?.offset !== undefined) params.set('offset', String(options.offset));
  const qs = params.toString();
  const res = await request<BackendRunList>(
    `/automations/${automationId}/runs${qs ? `?${qs}` : ''}`,
  );
  return res.items.map(toRun);
}

/**
 * Automation runs across a project whose scheduled time falls in `[from, to)`
 * — the calendar's realized (past) slots. `from`/`to` are ISO.
 */
export async function listRunsInWindow(
  projectRef: string,
  from: string,
  to: string,
): Promise<Run[]> {
  const qs = new URLSearchParams({ project_ref: projectRef, from, to }).toString();
  const res = await request<BackendRunList>(`/automations/runs?${qs}`);
  return res.items.map(toRun);
}

/** Manually fire a run; backend currently accepts no body. */
export async function createRun(automationId: AutomationId): Promise<Run> {
  const raw = await request<BackendRun>(`/automations/${automationId}/runs`, {
    method: 'POST',
    body: {},
  });
  return toRun(raw);
}

export async function getRun(
  automationId: AutomationId,
  runId: RunId,
): Promise<Run> {
  const raw = await request<BackendRun>(`/automations/${automationId}/runs/${runId}`);
  return toRun(raw);
}

export async function listRunEvents(
  automationId: AutomationId,
  runId: RunId,
): Promise<RunEvent[]> {
  const res = await request<BackendRunEventList>(
    `/automations/${automationId}/runs/${runId}/events`,
  );
  return res.items.map(toRunEvent);
}

/** Cancel a queued or running run. Returns the updated run (status:cancelled). */
export async function cancelRun(
  automationId: AutomationId,
  runId: RunId,
): Promise<Run> {
  const raw = await request<BackendRun>(
    `/automations/${automationId}/runs/${runId}/cancel`,
    { method: 'POST', body: {} },
  );
  return toRun(raw);
}

// ── helpers ────────────────────────────────────────────────────────────────

/** Map domain TriggerSpec to the backend body shape (drops null tz for cron). */
function specToBody(spec: TriggerSpec): BackendTriggerSpec {
  if (spec.kind === 'cron') {
    return spec.tz ? { kind: 'cron', expr: spec.expr, tz: spec.tz } : { kind: 'cron', expr: spec.expr };
  }
  return { kind: 'webhook' };
}
