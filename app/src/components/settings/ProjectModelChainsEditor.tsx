// Per-project recommendation-chain editor (project settings). Each agent type
// has an ordered list of models; resolution walks it and uses the first model
// whose provider is configured (the last entry is the terminal default). Models
// can be listed regardless of provider availability — unavailable ones are just
// marked. Omitting an agent type (or resetting it) falls back to the built-in
// default chain.

import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getModelCatalog, modelLabel, tierLabel } from '@/api/models';
import { updateProject } from '@/api/projects';
import { AGENT_SURFACES } from '@/domain/agentSurfaces';
import { SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';
import { useToastStore } from '@/components/Toast';

// Agent types in display order, with their surface labels. agent_type == surface id.
const AGENTS = AGENT_SURFACES.map((s) => ({ type: s.id, label: s.label }));

type Chains = Record<string, string[]>;

const sameChain = (a: string[], b: string[]) => JSON.stringify(a) === JSON.stringify(b);

export function ProjectModelChainsEditor({
  projectSlug,
  overrides,
  editable,
}: {
  projectSlug: string;
  overrides: Chains;
  editable: boolean;
}) {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  // Default chains (no project_ref) + the full model catalog for labels/availability.
  const catalog = useQuery({ queryKey: ['models'], queryFn: () => getModelCatalog(), staleTime: 5 * 60_000 });

  const defaults = useMemo<Chains>(() => {
    const m: Chains = {};
    catalog.data?.agents.forEach((a) => { m[a.agentType] = a.chain; });
    return m;
  }, [catalog.data]);

  // draft[type] = the ordered list currently being edited (seeded from the
  // project override, else the default chain). Re-seeded when project/defaults load.
  const [draft, setDraft] = useState<Chains>({});
  useEffect(() => {
    if (!catalog.data) return;
    const seed: Chains = {};
    for (const { type } of AGENTS) seed[type] = overrides[type] ?? defaults[type] ?? [];
    setDraft(seed);
  }, [catalog.data, projectSlug]);

  const save = useMutation({
    mutationFn: () => {
      // Only persist agents that differ from their default chain; equal → omit
      // so the project tracks the moving default instead of pinning it.
      const next: Chains = {};
      for (const { type } of AGENTS) {
        const chain = draft[type] ?? [];
        if (chain.length > 0 && !sameChain(chain, defaults[type] ?? [])) next[type] = chain;
      }
      return updateProject(projectSlug, { recommendedChains: next });
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['project', projectSlug] }),
        queryClient.invalidateQueries({ queryKey: ['models', projectSlug] }),
      ]);
      showToast('추천 모델 체인을 저장했습니다');
    },
    onError: () => showToast('추천 모델 체인 저장에 실패했습니다'),
  });

  if (!catalog.data) {
    return (
      <div style={{ marginBottom: 24 }}>
        <SectionLabel>Model recommendations</SectionLabel>
        <p style={{ color: 'var(--cw-ink-4)', fontSize: 13 }}>모델 목록을 불러오는 중…</p>
      </div>
    );
  }

  const dirty = AGENTS.some(({ type }) => {
    const chain = draft[type] ?? [];
    const current = overrides[type] ?? defaults[type] ?? [];
    return !sameChain(chain, current);
  });

  const setChain = (type: string, next: string[]) => setDraft((d) => ({ ...d, [type]: next }));
  const move = (type: string, i: number, dir: -1 | 1) => {
    const list = [...(draft[type] ?? [])];
    const j = i + dir;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j]!, list[i]!];
    setChain(type, list);
  };

  return (
    <div style={{ marginBottom: 24 }}>
      <SectionLabel>Model recommendations</SectionLabel>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        {AGENTS.map(({ type, label }) => {
          const chain = draft[type] ?? [];
          const isDefault = sameChain(chain, defaults[type] ?? []);
          const remaining = catalog.data!.models.filter((m) => !chain.includes(m.id));
          return (
            <div
              key={type}
              style={{
                border: '1px solid var(--cw-line)',
                borderRadius: 10,
                background: 'var(--cw-paper-2)',
                padding: 14,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
                <b style={{ fontSize: 13, color: 'var(--cw-ink)' }}>
                  {label}
                  {isDefault && (
                    <span style={{ marginLeft: 8, fontSize: 11, color: 'var(--cw-ink-4)', fontWeight: 400 }}>(기본값)</span>
                  )}
                </b>
                {editable && !isDefault && (
                  <button
                    type="button"
                    className="cw-btn-secondary"
                    onClick={() => setChain(type, defaults[type] ?? [])}
                    style={{ fontSize: 12, padding: '3px 8px' }}
                  >
                    기본값으로
                  </button>
                )}
              </div>

              <ol style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 6 }}>
                {chain.map((id, i) => {
                  const model = catalog.data!.models.find((m) => m.id === id);
                  const unavailable = model ? !model.available : true;
                  return (
                    <li
                      key={id}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '6px 8px',
                        border: '1px solid var(--cw-line)',
                        borderRadius: 8,
                        background: 'var(--cw-paper)',
                        opacity: unavailable ? 0.55 : 1,
                      }}
                    >
                      <span style={{ width: 18, textAlign: 'center', color: 'var(--cw-ink-4)', fontSize: 12 }}>{i + 1}</span>
                      <span style={{ flex: 1, minWidth: 0, fontSize: 13, color: 'var(--cw-ink)' }}>
                        {modelLabel(catalog.data, id)}
                        {model && (
                          <span style={{ marginLeft: 6, fontSize: 11, color: 'var(--cw-ink-4)' }}>· {tierLabel(model.tier)}</span>
                        )}
                        {unavailable && (
                          <span style={{ marginLeft: 6, fontSize: 11, color: 'var(--cw-destructive)' }}>· Provider 없음</span>
                        )}
                      </span>
                      {editable && (
                        <span style={{ display: 'inline-flex', gap: 2 }}>
                          <IconBtn label="위로" disabled={i === 0} onClick={() => move(type, i, -1)}><span style={{ fontSize: 13, lineHeight: 1 }}>↑</span></IconBtn>
                          <IconBtn label="아래로" disabled={i === chain.length - 1} onClick={() => move(type, i, 1)}><span style={{ fontSize: 13, lineHeight: 1 }}>↓</span></IconBtn>
                          <IconBtn label="제거" disabled={chain.length <= 1} onClick={() => setChain(type, chain.filter((x) => x !== id))}><Icon name="x" size={13} /></IconBtn>
                        </span>
                      )}
                    </li>
                  );
                })}
              </ol>

              {editable && remaining.length > 0 && (
                <select
                  value=""
                  onChange={(e) => { if (e.target.value) setChain(type, [...chain, e.target.value]); }}
                  style={{
                    marginTop: 8,
                    fontSize: 12,
                    padding: '5px 8px',
                    border: '1px solid var(--cw-line)',
                    borderRadius: 8,
                    background: 'var(--cw-paper)',
                    color: 'var(--cw-ink-2)',
                  }}
                >
                  <option value="">+ 모델 추가…</option>
                  {remaining.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label} ({tierLabel(m.tier)}){m.available ? '' : ' — Provider 없음'}
                    </option>
                  ))}
                </select>
              )}
            </div>
          );
        })}
      </div>

      {editable && (
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 14 }}>
          <button
            type="button"
            className="cw-btn-primary"
            onClick={() => save.mutate()}
            disabled={!dirty || save.isPending}
          >
            {save.isPending ? '저장 중…' : '추천 모델 저장'}
          </button>
        </div>
      )}
    </div>
  );
}

function IconBtn({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 24,
        height: 24,
        border: '1px solid var(--cw-line)',
        borderRadius: 6,
        background: 'var(--cw-paper-2)',
        color: 'var(--cw-ink-3)',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.4 : 1,
        padding: 0,
      }}
    >
      {children}
    </button>
  );
}
