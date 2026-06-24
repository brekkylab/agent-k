// The current user's own per-project agent grant. "Inherit ceiling" sends null;
// otherwise the explicitly checked subset, including picks outside the ceiling —
// those are preserved (saved) but inert at runtime, since the backend intersects
// the grant with the ceiling. The checklist keeps them selectable and flags them
// (strikethrough + "inactive") rather than disabling them.
//
// The only gate is "is this the current user?" — the backend allows the member
// themselves (owner or not) to edit their own grant, and 403s for anyone else.

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { setMemberAgentCapabilities } from '@/api/projects';
import { AGENT_CAPABILITIES } from '@/domain/agentCapabilities';
import { SectionLabel } from '@/components/uiPrimitives';
import { useToastStore } from '@/components/Toast';
import { ApiError } from '@/api/client';
import { AgentCapabilityChecklist } from './AgentCapabilityChecklist';
import { AgentModeToggle } from './AgentModeToggle';

const sameSet = (a: string[] | null, b: string[] | null): boolean => {
  if (a === null || b === null) return a === b;
  if (a.length !== b.length) return false;
  const sb = new Set(b);
  return a.every((x) => sb.has(x));
};

export function MemberAgentGrantEditor({
  projectSlug,
  userId,
  ceiling,
  capabilities,
}: {
  projectSlug: string;
  /** The current user's id — backend authorizes self-edits only. */
  userId: string;
  /** The project ceiling; `null` = no limit (all). */
  ceiling: string[] | null;
  /** The member's current persisted grant; `null` = inherit the ceiling. */
  capabilities: string[] | null;
}) {
  const { t } = useTranslation('project');
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const allowed = useMemo(() => (ceiling === null ? null : new Set(ceiling)), [ceiling]);
  const [inherit, setInherit] = useState(capabilities === null);
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(capabilities ?? ceiling ?? AGENT_CAPABILITIES),
  );

  // Re-seed when the persisted grant/ceiling (or identity) changes.
  useEffect(() => {
    setInherit(capabilities === null);
    setSelected(new Set(capabilities ?? ceiling ?? AGENT_CAPABILITIES));
  }, [capabilities, ceiling, userId, projectSlug]);

  // Persist the full selection, including capabilities currently outside the
  // ceiling — the choice is kept so it takes effect if the ceiling later allows
  // it. The backend intersects with the ceiling at run time, so out-of-ceiling
  // picks are inert until then (the checklist flags them as such).
  const draftWire = useMemo<string[] | null>(() => {
    if (inherit) return null;
    return AGENT_CAPABILITIES.filter((c) => selected.has(c));
  }, [inherit, selected]);

  const dirty = !sameSet(draftWire, capabilities);

  const save = useMutation({
    mutationFn: () => setMemberAgentCapabilities(projectSlug, userId, draftWire),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['members', projectSlug] });
      showToast(t('agent_grant.saved'));
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'save failed';
      showToast(t('agent_grant.save_failed', { message: msg }));
    },
  });

  const toggle = (cap: string, next: boolean) => {
    setSelected((prev) => {
      const copy = new Set(prev);
      if (next) copy.add(cap); else copy.delete(cap);
      return copy;
    });
  };

  return (
    <div style={{ marginTop: 24 }}>
      <SectionLabel>{t('agent_grant.section_label')}</SectionLabel>
      <p style={{ margin: '4px 0 12px', color: 'var(--cw-ink-3)', fontSize: 12, lineHeight: 1.55 }}>
        {t('agent_grant.help')}
      </p>

      <AgentModeToggle
        label={t('agent_grant.inherit')}
        hint={t('agent_grant.inherit_hint')}
        checked={inherit}
        onChange={setInherit}
      />

      {!inherit && (
        <AgentCapabilityChecklist
          ns="project"
          selected={selected}
          onToggle={toggle}
          allowed={allowed}
        />
      )}

      <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 14 }}>
        <button
          type="button"
          className="cw-btn-primary"
          onClick={() => save.mutate()}
          disabled={!dirty || save.isPending}
        >
          {save.isPending ? t('agent_grant.saving') : t('agent_grant.save')}
        </button>
      </div>
    </div>
  );
}
