import { useEffect, useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Icon } from '@/components/Icon';
import { SchedulePicker, summarizeCron, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useToastStore } from '@/components/Toast';
import {
  createTrigger as createTriggerApi,
  deleteAutomation as deleteAutomationApi,
  deleteTrigger as deleteTriggerApi,
  getAutomation,
  listTriggers,
  updateAutomation as updateAutomationApi,
  updateTrigger as updateTriggerApi,
} from '@/api/automations';
import type { Trigger, TriggerSpec } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation/$automationId')({
  component: AutomationSettingsPage,
});

function AutomationSettingsPage() {
  const { projectSlug, automationId } = Route.useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const automationQuery = useQuery({
    queryKey: ['automation', automationId],
    queryFn: () => getAutomation(automationId),
  });
  const triggersQuery = useQuery({
    queryKey: ['triggers', automationId],
    queryFn: () => listTriggers(automationId),
  });

  const automation = automationQuery.data;
  const triggers: Trigger[] = triggersQuery.data ?? [];

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [prompts, setPrompts] = useState<string[]>(['']);
  const [syncedAt, setSyncedAt] = useState<string | null>(null);
  useEffect(() => {
    if (!automation) return;
    if (syncedAt === automation.updatedAt) return;
    setName(automation.name);
    setDescription(automation.description ?? '');
    setPrompts(automation.prompts.length > 0 ? automation.prompts : ['']);
    setSyncedAt(automation.updatedAt);
  }, [automation, syncedAt]);

  const goBack = () => {
    navigate({ to: '/projects/$projectSlug/automation', params: { projectSlug } });
  };

  // ── Mutations ───────────────────────────────────────────────────────────
  const invalidateAutomation = () => {
    void queryClient.invalidateQueries({ queryKey: ['automation', automationId] });
    void queryClient.invalidateQueries({ queryKey: ['automations', projectSlug] });
  };
  const invalidateTriggers = () => {
    void queryClient.invalidateQueries({ queryKey: ['triggers', automationId] });
  };

  const saveMutation = useMutation({
    mutationFn: () => updateAutomationApi(automationId, {
      name,
      description: description.trim() ? description : null,
      prompts: prompts.filter((p) => p.trim().length > 0),
    }),
    onSuccess: () => {
      invalidateAutomation();
      goBack();
    },
  });

  const [revealedToken, setRevealedToken] = useState<string | null>(null);
  const [revealedTriggerId, setRevealedTriggerId] = useState<string | null>(null);
  const [discardArmed, setDiscardArmed] = useState(false);
  const closeReveal = () => {
    setRevealedToken(null);
    setRevealedTriggerId(null);
    setDiscardArmed(false);
  };

  const createTriggerMutation = useMutation({
    mutationFn: (spec: TriggerSpec) => createTriggerApi(automationId, spec),
    onSuccess: (created) => {
      invalidateTriggers();
      if (created.webhookToken) {
        setRevealedTriggerId(created.trigger.id);
        setRevealedToken(created.webhookToken);
      }
      resetDraft();
    },
  });

  const toggleAutomationEnabledMutation = useMutation({
    mutationFn: (next: boolean) => updateAutomationApi(automationId, { enabled: next }),
    onSuccess: () => invalidateAutomation(),
  });

  const deleteAutomationMutation = useMutation({
    mutationFn: () => deleteAutomationApi(automationId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['automations', projectSlug] });
      goBack();
    },
  });

  const updateTriggerMutation = useMutation({
    mutationFn: (vars: { triggerId: string; patch: { spec?: TriggerSpec; enabled?: boolean } }) =>
      updateTriggerApi(automationId, vars.triggerId, vars.patch),
    onSuccess: () => invalidateTriggers(),
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(`트리거 수정 실패: ${msg}`);
    },
  });

  const deleteTriggerMutation = useMutation({
    mutationFn: (triggerId: string) => deleteTriggerApi(automationId, triggerId),
    onSuccess: () => invalidateTriggers(),
  });

  // ── Prompts ─────────────────────────────────────────────────────────────
  const updatePrompt = (i: number, value: string) => {
    setPrompts((p) => p.map((line, idx) => (idx === i ? value : line)));
  };
  const addPrompt = () => setPrompts((p) => [...p, '']);
  const removePrompt = (i: number) => setPrompts((p) => p.filter((_, idx) => idx !== i));

  // ── Trigger row actions ─────────────────────────────────────────────────
  const [pendingTriggerDeleteId, setPendingTriggerDeleteId] = useState<string | null>(null);
  const removeTrigger = (id: string) => {
    if (pendingTriggerDeleteId !== id) {
      setPendingTriggerDeleteId(id);
      return;
    }
    setPendingTriggerDeleteId(null);
    deleteTriggerMutation.mutate(id);
  };
  const toggleTriggerEnabled = (t: Trigger) =>
    updateTriggerMutation.mutate({ triggerId: t.id, patch: { enabled: !t.enabled } });

  // ── Add-trigger draft ───────────────────────────────────────────────────
  const [showAddForm, setShowAddForm] = useState(false);
  const [draftKind, setDraftKind] = useState<'cron' | 'webhook'>('cron');
  const [draftSchedule, setDraftSchedule] = useState<SchedulePickerValue>({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });
  const resetDraft = () => {
    setShowAddForm(false);
    setDraftKind('cron');
    setDraftSchedule({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });
  };
  const [showDeleteAutomationConfirm, setShowDeleteAutomationConfirm] = useState(false);
  const submitDraft = () => {
    if (draftKind === 'cron') {
      createTriggerMutation.mutate({
        kind: 'cron',
        expr: draftSchedule.expr || '0 * * * *',
        tz: draftSchedule.tz || null,
      });
    } else {
      createTriggerMutation.mutate({ kind: 'webhook' });
    }
  };

  // ── Inline cron edit ────────────────────────────────────────────────────
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editSchedule, setEditSchedule] = useState<SchedulePickerValue>({ expr: '', tz: '' });
  const startEdit = (t: Trigger) => {
    if (t.spec.kind !== 'cron') return;
    setEditingId(t.id);
    setEditSchedule({ expr: t.spec.expr, tz: t.spec.tz ?? '' });
  };
  const cancelEdit = () => setEditingId(null);
  const saveEdit = () => {
    const t = triggers.find((x) => x.id === editingId);
    if (!t || t.spec.kind !== 'cron') { setEditingId(null); return; }
    updateTriggerMutation.mutate({
      triggerId: t.id,
      patch: { spec: { kind: 'cron', expr: editSchedule.expr, tz: editSchedule.tz || null } },
    });
    setEditingId(null);
  };

  // ── Loading / not-found ────────────────────────────────────────────────
  if (automationQuery.isLoading) {
    return (
      <section className="cw-page cw-automation-settings cw-page-enter">
        <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
          <Icon name="arrow-left" size={14} /> Automations
        </button>
        <p>로딩 중…</p>
      </section>
    );
  }
  if (!automation) {
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

  const enabled = automation.enabled;

  return (
    <section className="cw-page cw-automation-settings cw-page-enter">
      <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
        <Icon name="arrow-left" size={14} /> Automations
      </button>

      <header className="cw-page-head">
        <div>
          <h1>{automation.name}</h1>
          <p>이 automation의 이름·프롬프트·트리거를 관리합니다.</p>
        </div>
        <div>
          <button className="cw-btn-secondary" type="button" onClick={goBack}>Cancel</button>
          <button
            className="cw-btn-primary"
            type="button"
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending}
          >
            {saveMutation.isPending ? 'Saving…' : 'Save changes'}
          </button>
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
                onChange={(e) => toggleAutomationEnabledMutation.mutate(e.target.checked)}
                disabled={toggleAutomationEnabledMutation.isPending}
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
          <p className="cw-settings-hint">트리거 추가/수정/삭제는 즉시 반영됩니다. Save changes는 automation 본문만 저장합니다.</p>

          {showAddForm && (
            <div className="cw-trigger-draft">
              <div className="cw-trigger-picker" role="radiogroup" aria-label="Trigger kind">
                {(['cron', 'webhook'] as const).map((kind) => {
                  const active = draftKind === kind;
                  const label = kind === 'cron' ? 'Recurring' : 'Webhook';
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
                <button
                  className="cw-btn-primary"
                  type="button"
                  onClick={submitDraft}
                  disabled={createTriggerMutation.isPending}
                >
                  <Icon name="check" size={14} /> {createTriggerMutation.isPending ? 'Adding…' : 'Add'}
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
                const isCron = trigger.spec.kind === 'cron';
                return (
                  <li key={trigger.id} className={`cw-trigger-row ${isEditing ? 'is-editing' : ''}`}>
                    {isEditing && isCron ? (
                      <>
                        <header className="cw-trigger-edit-head">
                          <span className="cw-trigger-badge cw-trigger-cron">recurring</span>
                          <label className="cw-trigger-enabled">
                            <span>{trigger.enabled ? 'enabled' : 'disabled'}</span>
                            <span className="cw-switch">
                              <input
                                type="checkbox"
                                checked={trigger.enabled}
                                onChange={() => toggleTriggerEnabled(trigger)}
                              />
                              <span className="cw-switch-track" aria-hidden />
                            </span>
                          </label>
                          <button
                            className="cw-trigger-action"
                            type="button"
                            aria-label="Save"
                            onClick={saveEdit}
                            title="Save"
                            disabled={updateTriggerMutation.isPending}
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
                          {isCron ? 'recurring' : trigger.kind}
                        </span>
                        {trigger.spec.kind === 'cron' ? (
                          <>
                            <span className="cw-trigger-summary">{summarizeCron(trigger.spec.expr)}</span>
                            <span className="cw-trigger-meta">{trigger.spec.tz ?? 'UTC'}</span>
                          </>
                        ) : (
                          <>
                            <code className="cw-trigger-expr">whk_{trigger.id.slice(0, 6)}</code>
                            <span className="cw-trigger-meta">bearer token</span>
                          </>
                        )}
                        <label className="cw-trigger-enabled">
                          <span>{trigger.enabled ? 'enabled' : 'disabled'}</span>
                          <span className="cw-switch">
                            <input
                              type="checkbox"
                              checked={trigger.enabled}
                              onChange={() => toggleTriggerEnabled(trigger)}
                            />
                            <span className="cw-switch-track" aria-hidden />
                          </span>
                        </label>
                        <button
                          className="cw-trigger-action"
                          type="button"
                          aria-label="Edit trigger"
                          onClick={() => startEdit(trigger)}
                          disabled={!isCron}
                          title={!isCron ? '웹훅은 편집 불가' : 'Edit'}
                        >
                          <Icon name="settings" size={14} />
                        </button>
                        <button
                          className={`cw-trigger-action${pendingTriggerDeleteId === trigger.id ? ' is-armed' : ''}`}
                          type="button"
                          aria-label={pendingTriggerDeleteId === trigger.id ? '한 번 더 눌러 삭제' : 'Delete trigger'}
                          onClick={() => removeTrigger(trigger.id)}
                        >
                          {pendingTriggerDeleteId === trigger.id
                            ? '한 번 더 눌러 삭제'
                            : <Icon name="trash" size={14} />}
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
          <button
            className="cw-btn-secondary cw-btn-destructive"
            type="button"
            onClick={() => setShowDeleteAutomationConfirm(true)}
            disabled={deleteAutomationMutation.isPending}
          >
            <Icon name="trash" size={14} /> Delete automation
          </button>
        </section>
      </div>

      {revealedToken && (
        <WebhookTokenDialog
          token={revealedToken}
          curlSample={`curl -X POST \\\n  -H 'Authorization: Bearer ${revealedToken}' \\\n  https://api.example.com/webhooks/automations`}
          onClose={closeReveal}
          onDiscard={revealedTriggerId ? () => {
            if (!discardArmed) {
              setDiscardArmed(true);
              return;
            }
            const tid = revealedTriggerId;
            closeReveal();
            deleteTriggerMutation.mutate(tid);
          } : undefined}
          discardLabel={discardArmed ? '한 번 더 눌러 삭제' : '트리거 삭제'}
          discardArmed={discardArmed}
        />
      )}
      {showDeleteAutomationConfirm && (
        <ConfirmDialog
          title="Automation 삭제"
          body={`'${automation.name}' automation을 정말 삭제할까요? 실행 이력과 webhook 토큰이 모두 사라집니다.`}
          confirmLabel="삭제"
          destructive
          pending={deleteAutomationMutation.isPending}
          onConfirm={() => deleteAutomationMutation.mutate()}
          onClose={() => setShowDeleteAutomationConfirm(false)}
        />
      )}
    </section>
  );
}
