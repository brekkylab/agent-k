import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Icon } from '@/components/Icon';
import { SchedulePicker, summarizeCron, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';

export const Route = createFileRoute('/_app/projects/$projectId/automation/$automationId')({
  component: AutomationSettingsPage,
});

type TriggerEntry =
  | { id: string; kind: 'cron';    expr: string; tz: string; enabled: boolean }
  | { id: string; kind: 'webhook'; tokenPreview: string;     enabled: boolean };

interface MockAutomationDetail {
  id: string;
  name: string;
  description: string;
  prompts: string[];
  triggers: TriggerEntry[];
  disabled?: boolean;
}

const MOCK_DETAILS: Record<string, MockAutomationDetail> = {
  a1: {
    id: 'a1',
    name: 'Daily summary',
    description: 'Recap yesterday\'s sessions and surface today\'s priorities.',
    prompts: [
      '어제 진행된 세션을 1-2줄 요약으로 정리해줘.',
      '오늘 우선순위 항목 3개를 뽑아줘.',
    ],
    triggers: [
      { id: 't1', kind: 'cron', expr: '0 10 * * *', tz: 'Asia/Seoul', enabled: true },
    ],
  },
  a2: {
    id: 'a2',
    name: 'Weekly digest',
    description: 'Friday rollup to email.',
    prompts: ['이번 주 핵심 결정 사항을 정리해줘.'],
    triggers: [
      { id: 't2', kind: 'cron', expr: '0 9 * * 5', tz: 'Asia/Seoul', enabled: true },
    ],
  },
  a3: {
    id: 'a3',
    name: 'On-call ping',
    description: 'External webhook hand-off.',
    prompts: ['인시던트 내용을 받아 1줄 요약과 담당자를 추정해줘.'],
    triggers: [
      { id: 't3', kind: 'webhook', tokenPreview: 'whk_••••e8f2', enabled: true },
    ],
    disabled: true,
  },
  a4: {
    id: 'a4',
    name: 'Backfill ledger',
    description: 'Manual one-shot reruns for missed ledger entries.',
    prompts: ['주어진 기간의 누락 ledger 항목을 다시 처리해줘.'],
    triggers: [],
  },
};

function AutomationSettingsPage() {
  const { projectId, automationId } = Route.useParams();
  const navigate = useNavigate();
  const detail = MOCK_DETAILS[automationId];

  const [name, setName] = useState(detail?.name ?? '');
  const [description, setDescription] = useState(detail?.description ?? '');
  const [enabled, setEnabled] = useState(!detail?.disabled);
  const [prompts, setPrompts] = useState<string[]>(detail?.prompts ?? ['']);
  const [triggers, setTriggers] = useState<TriggerEntry[]>(detail?.triggers ?? []);

  const goBack = () => {
    navigate({ to: '/projects/$projectId/automation', params: { projectId } });
  };

  if (!detail) {
    return (
      <section className="cw-page cw-automation-settings cw-page-enter">
        <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
          <Icon name="arrow-left" size={14} /> Automations
        </button>
        <h1>Automation을 찾을 수 없습니다</h1>
        <p>이 automation은 삭제되었거나 권한이 없습니다.</p>
      </section>
    );
  }

  const updatePrompt = (i: number, value: string) => {
    setPrompts((p) => p.map((line, idx) => (idx === i ? value : line)));
  };
  const addPrompt = () => setPrompts((p) => [...p, '']);
  const removePrompt = (i: number) => setPrompts((p) => p.filter((_, idx) => idx !== i));

  const removeTrigger = (id: string) => setTriggers((prev) => prev.filter((t) => t.id !== id));
  const toggleEnabled = (id: string) =>
    setTriggers((prev) => prev.map((t) => (t.id === id ? { ...t, enabled: !t.enabled } : t)));

  // Add-trigger form state — collapsed by default, opens on "Add trigger".
  const [showAddForm, setShowAddForm] = useState(false);
  const [draftKind, setDraftKind] = useState<'cron' | 'webhook'>('cron');
  const [draftSchedule, setDraftSchedule] = useState<SchedulePickerValue>({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });
  const resetDraft = () => {
    setShowAddForm(false);
    setDraftKind('cron');
    setDraftSchedule({ expr: '0 9 * * 1', tz: 'Asia/Seoul' });
  };
  const [revealedToken, setRevealedToken] = useState<string | null>(null);
  const submitDraft = () => {
    const id = `t-${Date.now().toString(36)}`;
    if (draftKind === 'cron') {
      const next: TriggerEntry = {
        id, kind: 'cron',
        expr: draftSchedule.expr || '0 * * * *',
        tz: draftSchedule.tz || 'Asia/Seoul',
        enabled: true,
      };
      setTriggers((prev) => [...prev, next]);
      resetDraft();
    } else {
      const fullToken = generateWebhookToken();
      const next: TriggerEntry = {
        id, kind: 'webhook',
        tokenPreview: `whk_••••${fullToken.slice(-4)}`,
        enabled: true,
      };
      setTriggers((prev) => [...prev, next]);
      resetDraft();
      setRevealedToken(fullToken);
    }
  };

  // Inline edit state for an existing trigger row.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editSchedule, setEditSchedule] = useState<SchedulePickerValue>({ expr: '', tz: '' });
  const startEdit = (t: TriggerEntry) => {
    setEditingId(t.id);
    if (t.kind === 'cron') setEditSchedule({ expr: t.expr, tz: t.tz });
  };
  const cancelEdit = () => setEditingId(null);
  const saveEdit = () => {
    setTriggers((prev) =>
      prev.map((t) => {
        if (t.id !== editingId) return t;
        if (t.kind === 'cron') return { ...t, expr: editSchedule.expr || t.expr, tz: editSchedule.tz || t.tz };
        return t;
      }),
    );
    setEditingId(null);
  };

  return (
    <section className="cw-page cw-automation-settings cw-page-enter">
      <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
        <Icon name="arrow-left" size={14} /> Automations
      </button>

      <header className="cw-page-head">
        <div>
          <h1>{detail.name}</h1>
          <p>이 automation의 이름·프롬프트·트리거를 관리합니다.</p>
        </div>
        <div>
          <button className="cw-btn-primary" type="button">Save changes</button>
        </div>
      </header>

      <div className="cw-settings-stack">
        <section className="cw-settings-card">
          <h2>일반</h2>
          <div className="cw-settings-toggle-row">
            <div>
              <b>상태</b>
              <p className="cw-settings-hint">
                {enabled
                  ? '활성화 상태 — 트리거가 발화되면 실행됩니다.'
                  : '비활성화 상태 — 트리거가 발화돼도 실행되지 않고, 수동 실행 버튼도 막힙니다.'}
              </p>
            </div>
            <label className="cw-toggle-switch">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
              />
              <span className="cw-toggle-slider" />
              <span className="cw-toggle-label">{enabled ? 'On' : 'Off'}</span>
            </label>
          </div>
          <label className="cw-settings-field">
            <span>이름</span>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Automation 이름"
            />
          </label>
          <label className="cw-settings-field">
            <span>설명</span>
            <textarea
              rows={3}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="이 automation이 무엇을 하는지"
            />
          </label>
        </section>

        <section className="cw-settings-card">
          <header className="cw-settings-card-head">
            <h2>Prompts</h2>
            <button className="cw-btn-secondary" type="button" onClick={addPrompt}>
              <Icon name="plus" size={14} /> Add prompt
            </button>
          </header>
          <p className="cw-settings-hint">실행 시 순서대로 평가됩니다. 첫 프롬프트가 후속 단계의 컨텍스트가 됩니다.</p>
          <ol className="cw-prompt-list">
            {prompts.map((line, i) => (
              <li key={i}>
                <span className="cw-prompt-index">{i + 1}</span>
                <textarea
                  rows={2}
                  value={line}
                  onChange={(e) => updatePrompt(i, e.target.value)}
                  placeholder="프롬프트를 입력하세요"
                />
                <button
                  className="cw-prompt-remove"
                  type="button"
                  aria-label="Remove prompt"
                  onClick={() => removePrompt(i)}
                  disabled={prompts.length <= 1}
                >
                  <Icon name="trash" size={14} />
                </button>
              </li>
            ))}
          </ol>
        </section>

        <section className="cw-settings-card">
          <header className="cw-settings-card-head">
            <h2>Triggers</h2>
            {!showAddForm && (
              <button
                className="cw-btn-secondary"
                type="button"
                onClick={() => setShowAddForm(true)}
              >
                <Icon name="plus" size={14} /> Add trigger
              </button>
            )}
          </header>

          {showAddForm && (
            <div className="cw-trigger-draft">
              <div className="cw-trigger-picker" role="radiogroup" aria-label="Trigger kind">
                {(['cron', 'webhook'] as const).map((kind) => {
                  const active = draftKind === kind;
                  const label = kind === 'cron' ? 'Schedule' : 'Webhook';
                  const sub = kind === 'cron' ? '일정에 따라 반복 실행' : '외부 시스템에서 호출';
                  return (
                    <label key={kind} className={`cw-trigger-option ${active ? 'is-active' : ''}`}>
                      <input
                        type="radio"
                        name="draftKind"
                        value={kind}
                        checked={active}
                        onChange={() => setDraftKind(kind)}
                      />
                      <span className="cw-trigger-option-label">
                        <b>{label}</b>
                        <em>{sub}</em>
                      </span>
                    </label>
                  );
                })}
              </div>

              {draftKind === 'cron' && (
                <div className="cw-trigger-detail">
                  <SchedulePicker value={draftSchedule} onChange={setDraftSchedule} />
                </div>
              )}

              {draftKind === 'webhook' && (
                <div className="cw-trigger-detail">
                  <p className="cw-settings-hint">
                    생성 직후 한 번만 토큰이 노출됩니다. 안전한 곳에 보관하세요.
                  </p>
                </div>
              )}

              <div className="cw-trigger-draft-actions">
                <button className="cw-btn-secondary" type="button" onClick={resetDraft}>Cancel</button>
                <button className="cw-btn-primary" type="button" onClick={submitDraft}>
                  <Icon name="check" size={14} /> Add
                </button>
              </div>
            </div>
          )}

          {triggers.length === 0 && !showAddForm ? (
            <p className="cw-settings-empty">트리거가 없습니다. 수동 실행만 가능합니다.</p>
          ) : triggers.length > 0 ? (
            <ul className="cw-trigger-list">
              {triggers.map((trigger) => {
                const isEditing = editingId === trigger.id;
                return (
                  <li key={trigger.id} className={`cw-trigger-row ${isEditing ? 'is-editing' : ''}`}>
                    {isEditing && trigger.kind === 'cron' ? (
                      <>
                        <header className="cw-trigger-edit-head">
                          <span className="cw-trigger-badge cw-trigger-cron">schedule</span>
                          <label className="cw-trigger-enabled">
                            <input
                              type="checkbox"
                              checked={trigger.enabled}
                              onChange={() => toggleEnabled(trigger.id)}
                            />
                            <span>{trigger.enabled ? 'enabled' : 'disabled'}</span>
                          </label>
                          <button
                            className="cw-trigger-action"
                            type="button"
                            aria-label="Save"
                            onClick={saveEdit}
                            title="Save"
                          >
                            <Icon name="check" size={14} />
                          </button>
                          <button
                            className="cw-trigger-action"
                            type="button"
                            aria-label="Cancel"
                            onClick={cancelEdit}
                            title="Cancel"
                          >
                            <Icon name="x" size={14} />
                          </button>
                        </header>
                        <div className="cw-trigger-edit-body">
                          <SchedulePicker value={editSchedule} onChange={setEditSchedule} />
                        </div>
                      </>
                    ) : (
                      <>
                        <span className={`cw-trigger-badge cw-trigger-${trigger.kind}`}>
                          {trigger.kind === 'cron' ? 'schedule' : trigger.kind}
                        </span>
                        {trigger.kind === 'cron' ? (
                          <>
                            <span className="cw-trigger-summary">{summarizeCron(trigger.expr)}</span>
                            <span className="cw-trigger-meta">{trigger.tz}</span>
                          </>
                        ) : (
                          <>
                            <code className="cw-trigger-expr">{trigger.tokenPreview}</code>
                            <span className="cw-trigger-meta">bearer token</span>
                          </>
                        )}
                        <label className="cw-trigger-enabled">
                          <input
                            type="checkbox"
                            checked={trigger.enabled}
                            onChange={() => toggleEnabled(trigger.id)}
                          />
                          <span>{trigger.enabled ? 'enabled' : 'disabled'}</span>
                        </label>
                        <button
                          className="cw-trigger-action"
                          type="button"
                          aria-label="Edit trigger"
                          onClick={() => startEdit(trigger)}
                          disabled={trigger.kind === 'webhook'}
                          title={trigger.kind === 'webhook' ? '웹훅은 편집 불가' : 'Edit'}
                        >
                          <Icon name="settings" size={14} />
                        </button>
                        <button
                          className="cw-trigger-action"
                          type="button"
                          aria-label="Delete trigger"
                          onClick={() => removeTrigger(trigger.id)}
                        >
                          <Icon name="trash" size={14} />
                        </button>
                      </>
                    )}
                  </li>
                );
              })}
            </ul>
          ) : null}
        </section>

        <section className="cw-settings-card cw-settings-danger">
          <h2>Danger zone</h2>
          <p>이 automation을 삭제하면 실행 이력과 webhook 토큰이 모두 사라집니다.</p>
          <button className="cw-btn-secondary cw-btn-destructive" type="button">
            <Icon name="trash" size={14} /> Delete automation
          </button>
        </section>
      </div>

      {revealedToken && (
        <WebhookTokenDialog
          token={revealedToken}
          curlSample={`curl -X POST \\\n  -H 'Authorization: Bearer ${revealedToken}' \\\n  https://api.example.com/webhooks/automations`}
          onClose={() => setRevealedToken(null)}
        />
      )}
    </section>
  );
}

function generateWebhookToken(): string {
  const bytes = new Uint8Array(24);
  if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  }
  const hex = Array.from(bytes).map((b) => b.toString(16).padStart(2, '0')).join('');
  return `whk_${hex}`;
}
