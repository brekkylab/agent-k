// Project agent-capability ceiling editor (project settings, owner-only). The
// ceiling caps what *any* member's agent may do in this project. `null` means
// "no limit" — every capability is allowed. A "No limit" toggle switches between
// the unbounded state (sends null) and an explicit checked subset.

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { setProjectAgentCeiling } from '@/api/projects';
import { AGENT_CAPABILITIES } from '@/domain/agentCapabilities';
import { SectionLabel } from '@/components/uiPrimitives';
import { useToastStore } from '@/components/Toast';
import { AgentCapabilityChecklist } from './AgentCapabilityChecklist';
import { AgentModeToggle } from './AgentModeToggle';

const sameSet = (a: string[] | null, b: string[] | null): boolean => {
  if (a === null || b === null) return a === b;
  if (a.length !== b.length) return false;
  const sb = new Set(b);
  return a.every((x) => sb.has(x));
};

// Normalize a draft (noLimit + set) to the wire value the backend expects.
const toWire = (noLimit: boolean, set: ReadonlySet<string>): string[] | null =>
  noLimit ? null : AGENT_CAPABILITIES.filter((c) => set.has(c));

export function ProjectAgentCeilingEditor({
  projectSlug,
  ceiling,
  editable,
}: {
  projectSlug: string;
  /** Current persisted ceiling; `null` = no limit. */
  ceiling: string[] | null;
  editable: boolean;
}) {
  const { t } = useTranslation('project');
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const [noLimit, setNoLimit] = useState(ceiling === null);
  const [selected, setSelected] = useState<Set<string>>(new Set(ceiling ?? AGENT_CAPABILITIES));

  // Re-seed when the persisted value (or project) changes.
  useEffect(() => {
    setNoLimit(ceiling === null);
    setSelected(new Set(ceiling ?? AGENT_CAPABILITIES));
  }, [projectSlug, ceiling]);

  const draftWire = useMemo(() => toWire(noLimit, selected), [noLimit, selected]);
  const dirty = !sameSet(draftWire, ceiling);

  const save = useMutation({
    mutationFn: () => setProjectAgentCeiling(projectSlug, draftWire),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['project', projectSlug] });
      showToast(t('agent_ceiling.saved'));
    },
    onError: () => showToast(t('agent_ceiling.save_failed')),
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
      <SectionLabel>{t('agent_ceiling.section_label')}</SectionLabel>
      <p style={{ margin: '4px 0 12px', color: 'var(--cw-ink-3)', fontSize: 12, lineHeight: 1.55 }}>
        {t('agent_ceiling.help')}
      </p>

      <AgentModeToggle
        label={t('agent_ceiling.no_limit')}
        hint={t('agent_ceiling.no_limit_hint')}
        checked={noLimit}
        disabled={!editable}
        onChange={setNoLimit}
      />

      {!noLimit && (
        <AgentCapabilityChecklist
          ns="project"
          selected={selected}
          onToggle={toggle}
          disabled={!editable}
        />
      )}

      {editable && (
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 14 }}>
          <button
            type="button"
            className="cw-btn-primary"
            onClick={() => save.mutate()}
            disabled={!dirty || save.isPending}
          >
            {save.isPending ? t('agent_ceiling.saving') : t('agent_ceiling.save')}
          </button>
        </div>
      )}
    </div>
  );
}
