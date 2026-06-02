import { useEffect, useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import { SchedulePicker, summarizeCron, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useToastStore } from '@/components/Toast';
import { ComposerAgentPicker } from '@/components/chat/ComposerAgentPicker';
import { ComposerModelPicker } from '@/components/chat/ComposerModelPicker';
import { DEFAULT_AGENT_ID, type AgentId } from '@/domain/agentSurfaces';
import { getModelCatalog } from '@/api/models';
import {
  createTrigger as createTriggerApi,
  deleteAutomation as deleteAutomationApi,
  deleteTrigger as deleteTriggerApi,
  getAutomation,
  listTriggers,
  updateAutomation as updateAutomationApi,
  updateTrigger as updateTriggerApi,
} from '@/api/automations';
import { loadNs } from '@/i18n/loader';
import type { Trigger, TriggerSpec } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation/$automationId')({
  loader: () => loadNs('automation'),
  component: AutomationSettingsPage,
});

function AutomationSettingsPage() {
  const { projectSlug, automationId } = Route.useParams();
  const navigate = useNavigate();
  const { t } = useTranslation('automation');
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
  const [agentId, setAgentId] = useState<AgentId>(DEFAULT_AGENT_ID);
  const [model, setModel] = useState<string | null>(null);
  const [nameEditMode, setNameEditMode] = useState(false);
  const [descEditMode, setDescEditMode] = useState(false);
  const [promptsEditMode, setPromptsEditMode] = useState(false);

  const catalog = useQuery({
    queryKey: ['models', projectSlug],
    queryFn: () => getModelCatalog(projectSlug),
    staleTime: 5 * 60_000,
  });

  // Populate form state when the automation first loads (or when navigating
  // to a different one). Subsequent refetches don't re-fire — drafts in
  // progress stay intact.
  useEffect(() => {
    if (!automation) return;
    setName(automation.name);
    setDescription(automation.description ?? '');
    setPrompts(automation.prompts.length > 0 ? automation.prompts : ['']);
    setAgentId((automation.agentType as AgentId | null) ?? DEFAULT_AGENT_ID);
    setModel(automation.model);
  }, [automation?.id]);

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

  const nameMutation = useMutation({
    mutationFn: () => updateAutomationApi(automationId, { name: name.trim() }),
    onSuccess: (updated) => {
      invalidateAutomation();
      setName(updated.name);
      setNameEditMode(false);
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(t('toast.name_save_failed', { message: msg }));
    },
  });
  const descMutation = useMutation({
    mutationFn: () => updateAutomationApi(automationId, {
      description: description.trim() ? description.trim() : null,
    }),
    onSuccess: (updated) => {
      invalidateAutomation();
      setDescription(updated.description ?? '');
      setDescEditMode(false);
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(t('toast.desc_save_failed', { message: msg }));
    },
  });
  const promptsMutation = useMutation({
    mutationFn: () => updateAutomationApi(automationId, {
      prompts: prompts.filter((p) => p.trim().length > 0),
    }),
    onSuccess: (updated) => {
      invalidateAutomation();
      setPrompts(updated.prompts.length > 0 ? updated.prompts : ['']);
      setPromptsEditMode(false);
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(t('toast.prompts_save_failed', { message: msg }));
    },
  });

  const agentModelMutation = useMutation({
    mutationFn: () => updateAutomationApi(automationId, { agentType: agentId, model }),
    onSuccess: (updated) => {
      invalidateAutomation();
      setAgentId((updated.agentType as AgentId | null) ?? DEFAULT_AGENT_ID);
      setModel(updated.model);
      showToast(t('toast.agent_model_saved'));
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(t('toast.agent_model_save_failed', { message: msg }));
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
      showToast(t('toast.trigger_save_failed', { message: msg }));
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
          <Icon name="arrow-left" size={14} /> {t('back')}
        </button>
        <p>{t('detail.loading')}</p>
      </section>
    );
  }
  if (!automation) {
    return (
      <section className="cw-page cw-automation-settings cw-page-enter">
        <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
          <Icon name="arrow-left" size={14} /> {t('back')}
        </button>
        <h1>{t('detail.not_found_title')}</h1>
        <p>{t('detail.not_found_body')}</p>
      </section>
    );
  }

  const enabled = automation.enabled;
  const trimmedName = name.trim();
  const nameDirty = trimmedName !== automation.name;
  const nameSaveDisabled = !nameDirty || trimmedName.length === 0 || nameMutation.isPending;
  const descDirty = description.trim() !== (automation.description ?? '');
  const descSaveDisabled = !descDirty || descMutation.isPending;
  const cleanedPrompts = prompts.filter((p) => p.trim().length > 0);
  const promptsDirty =
    cleanedPrompts.length !== automation.prompts.length ||
    cleanedPrompts.some((p, i) => p !== automation.prompts[i]);
  const promptsSaveDisabled = !promptsDirty || cleanedPrompts.length === 0 || promptsMutation.isPending;
  const currentAgentId = (automation.agentType as AgentId | null) ?? DEFAULT_AGENT_ID;
  const agentModelDirty = agentId !== currentAgentId || model !== automation.model;

  return (
    <section className="cw-page cw-automation-settings cw-page-enter">
      <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
        <Icon name="arrow-left" size={14} /> {t('back')}
      </button>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          flexWrap: 'wrap',
          marginTop: 4,
          marginBottom: 24,
        }}
      >
        {nameEditMode ? (
          <>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  if (!nameSaveDisabled) nameMutation.mutate();
                } else if (e.key === 'Escape') {
                  e.preventDefault();
                  setName(automation.name);
                  setNameEditMode(false);
                }
              }}
              disabled={nameMutation.isPending}
              maxLength={100}
              aria-label={t('detail.name_aria')}
              style={{
                margin: 0,
                padding: '4px 10px',
                border: '1px solid var(--cw-line)',
                borderRadius: 8,
                background: 'var(--cw-paper)',
                color: 'var(--cw-ink)',
                fontSize: 'var(--cw-text-2xl)',
                lineHeight: 1.12,
                letterSpacing: '-0.025em',
                fontWeight: 650,
                fontFamily: 'inherit',
                minWidth: 280,
              }}
            />
            <button
              type="button"
              className="cw-btn-primary"
              onClick={() => nameMutation.mutate()}
              disabled={nameSaveDisabled}
            >
              {nameMutation.isPending ? t('detail.saving') : t('detail.save')}
            </button>
            <button
              type="button"
              className="cw-btn-secondary"
              onClick={() => {
                setName(automation.name);
                setNameEditMode(false);
              }}
              disabled={nameMutation.isPending}
            >
              {t('detail.cancel')}
            </button>
          </>
        ) : (
          <>
            <h1 style={{ margin: 0 }}>{automation.name}</h1>
            <button
              type="button"
              onClick={() => setNameEditMode(true)}
              aria-label={t('detail.name_edit_aria')}
              title={t('detail.edit_title')}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                width: 30,
                height: 30,
                border: '1px solid var(--cw-line)',
                borderRadius: 8,
                background: 'var(--cw-paper-2)',
                color: 'var(--cw-ink-3)',
                cursor: 'pointer',
                padding: 0,
              }}
            >
              <Icon name="writing" size={14} />
            </button>
          </>
        )}
      </div>

      <div className="cw-settings-stack">
        <section className="cw-settings-card">
          <h2>{t('detail.general')}</h2>
          <div className="cw-settings-toggle-row">
            <div>
              <b>{t('detail.status')}</b>
              <p className="cw-settings-hint">
                {enabled
                  ? t('detail.status_on_hint')
                  : t('detail.status_off_hint')}
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
              <span className="cw-toggle-label">{enabled ? t('detail.on') : t('detail.off')}</span>
            </label>
          </div>
        </section>

        <section className="cw-settings-card">
          <header className="cw-settings-card-head">
            <h2>{t('detail.description')}</h2>
            {!descEditMode && (
              <button
                type="button"
                onClick={() => setDescEditMode(true)}
                aria-label={t('detail.description_edit_aria')}
                title={t('detail.edit_title')}
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 22,
                  height: 22,
                  border: '1px solid var(--cw-line)',
                  borderRadius: 6,
                  background: 'var(--cw-paper-2)',
                  color: 'var(--cw-ink-3)',
                  cursor: 'pointer',
                  padding: 0,
                }}
              >
                <Icon name="writing" size={12} />
              </button>
            )}
          </header>
          {descEditMode ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <textarea
                autoFocus
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    e.preventDefault();
                    setDescription(automation.description ?? '');
                    setDescEditMode(false);
                  }
                }}
                placeholder={t('detail.description_placeholder')}
                disabled={descMutation.isPending}
                rows={3}
                style={{
                  resize: 'vertical',
                  minHeight: 80,
                  fontFamily: 'inherit',
                  fontSize: 14,
                  lineHeight: 1.6,
                  padding: '10px 12px',
                  border: '1px solid var(--cw-line)',
                  borderRadius: 8,
                  background: 'var(--cw-paper)',
                  color: 'var(--cw-ink)',
                }}
              />
              <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
                <button
                  type="button"
                  className="cw-btn-secondary"
                  onClick={() => {
                    setDescription(automation.description ?? '');
                    setDescEditMode(false);
                  }}
                  disabled={descMutation.isPending}
                >
                  {t('detail.cancel')}
                </button>
                <button
                  type="button"
                  className="cw-btn-primary"
                  onClick={() => descMutation.mutate()}
                  disabled={descSaveDisabled}
                >
                  {descMutation.isPending ? t('detail.saving') : t('detail.save')}
                </button>
              </div>
            </div>
          ) : (
            <p
              style={{
                margin: 0,
                color: automation.description ? 'var(--cw-ink-2)' : 'var(--cw-ink-4)',
                fontSize: 14,
                lineHeight: 1.6,
              }}
            >
              {automation.description || t('detail.description_empty')}
            </p>
          )}
        </section>

        <section className="cw-settings-card">
          <header className="cw-settings-card-head">
            <h2>{t('detail.prompts')}</h2>
            {!promptsEditMode && (
              <button
                type="button"
                onClick={() => setPromptsEditMode(true)}
                aria-label={t('detail.prompts_edit_aria')}
                title={t('detail.edit_title')}
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 22,
                  height: 22,
                  border: '1px solid var(--cw-line)',
                  borderRadius: 6,
                  background: 'var(--cw-paper-2)',
                  color: 'var(--cw-ink-3)',
                  cursor: 'pointer',
                  padding: 0,
                }}
              >
                <Icon name="writing" size={12} />
              </button>
            )}
          </header>
          <p className="cw-settings-hint">{t('detail.prompts_hint')}</p>
          {promptsEditMode ? (
            <>
              <ol className="cw-prompt-list">
                {prompts.map((line, i) => (
                  <li key={i}>
                    <span className="cw-prompt-index">{i + 1}</span>
                    <textarea
                      rows={2}
                      value={line}
                      onChange={(e) => updatePrompt(i, e.target.value)}
                      placeholder={t('detail.prompt_placeholder')}
                      disabled={promptsMutation.isPending}
                    />
                    <button
                      className="cw-prompt-remove"
                      type="button"
                      aria-label={t('detail.remove_prompt')}
                      onClick={() => removePrompt(i)}
                      disabled={prompts.length <= 1 || promptsMutation.isPending}
                    >
                      <Icon name="trash" size={14} />
                    </button>
                  </li>
                ))}
              </ol>
              <div style={{ display: 'flex', gap: 10, justifyContent: 'space-between', marginTop: 12 }}>
                <button
                  className="cw-btn-secondary"
                  type="button"
                  onClick={addPrompt}
                  disabled={promptsMutation.isPending}
                >
                  <Icon name="plus" size={14} /> {t('detail.add_prompt')}
                </button>
                <div style={{ display: 'flex', gap: 10 }}>
                  <button
                    type="button"
                    className="cw-btn-secondary"
                    onClick={() => {
                      setPrompts(automation.prompts.length > 0 ? automation.prompts : ['']);
                      setPromptsEditMode(false);
                    }}
                    disabled={promptsMutation.isPending}
                  >
                    {t('detail.cancel')}
                  </button>
                  <button
                    type="button"
                    className="cw-btn-primary"
                    onClick={() => promptsMutation.mutate()}
                    disabled={promptsSaveDisabled}
                  >
                    {promptsMutation.isPending ? t('detail.saving') : t('detail.save')}
                  </button>
                </div>
              </div>
            </>
          ) : (
            <ol className="cw-prompt-list">
              {automation.prompts.map((line, i) => (
                <li key={i}>
                  <span className="cw-prompt-index">{i + 1}</span>
                  <p style={{ margin: 0, color: 'var(--cw-ink-2)', fontSize: 14, lineHeight: 1.5, whiteSpace: 'pre-wrap' }}>
                    {line}
                  </p>
                </li>
              ))}
            </ol>
          )}
        </section>

        <section className="cw-settings-card">
          <h2>{t('detail.agent_model')}</h2>
          <p className="cw-settings-hint">
            {t('detail.agent_model_hint')}
          </p>
          <div
            className="cw-agent-pickwrap"
            data-agent={agentId}
            style={{ display: 'flex', flexDirection: 'column', gap: 12, alignItems: 'flex-start' }}
          >
            <ComposerAgentPicker value={agentId} onChange={setAgentId} standalone />
            <ComposerModelPicker
              catalog={catalog.data}
              agentType={agentId}
              value={model}
              onChange={setModel}
            />
          </div>
          <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 12 }}>
            {agentModelDirty && (
              <button
                type="button"
                className="cw-btn-secondary"
                onClick={() => {
                  setAgentId(currentAgentId);
                  setModel(automation.model);
                }}
                disabled={agentModelMutation.isPending}
              >
                {t('detail.revert')}
              </button>
            )}
            <button
              type="button"
              className="cw-btn-primary"
              onClick={() => agentModelMutation.mutate()}
              disabled={!agentModelDirty || agentModelMutation.isPending}
            >
              {agentModelMutation.isPending ? t('detail.saving') : t('detail.save')}
            </button>
          </div>
        </section>

        <section className="cw-settings-card">
          <header className="cw-settings-card-head">
            <h2>{t('detail.triggers')}</h2>
            {!showAddForm && (
              <button
                className="cw-btn-secondary"
                type="button"
                onClick={() => setShowAddForm(true)}
              >
                <Icon name="plus" size={14} /> {t('detail.add_trigger')}
              </button>
            )}
          </header>

          {showAddForm && (
            <div className="cw-trigger-draft">
              <div className="cw-trigger-picker" role="radiogroup" aria-label={t('detail.trigger_kind_aria')}>
                {(['cron', 'webhook'] as const).map((kind) => {
                  const active = draftKind === kind;
                  const label = kind === 'cron' ? t('detail.schedule') : t('detail.webhook');
                  const sub = kind === 'cron' ? t('detail.schedule_sub') : t('detail.webhook_sub');
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
                    {t('detail.webhook_token_hint')}
                  </p>
                </div>
              )}

              <div className="cw-trigger-draft-actions">
                <button className="cw-btn-secondary" type="button" onClick={resetDraft}>{t('detail.draft_cancel')}</button>
                <button
                  className="cw-btn-primary"
                  type="button"
                  onClick={submitDraft}
                  disabled={createTriggerMutation.isPending}
                >
                  <Icon name="check" size={14} /> {createTriggerMutation.isPending ? t('detail.adding') : t('detail.add')}
                </button>
              </div>
            </div>
          )}

          {triggers.length === 0 && !showAddForm ? (
            <p className="cw-settings-empty">{t('detail.no_triggers')}</p>
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
                          <span className="cw-trigger-badge cw-trigger-cron">schedule</span>
                          <label className="cw-trigger-enabled">
                            <span>{trigger.enabled ? t('detail.trigger_enabled') : t('detail.trigger_disabled')}</span>
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
                            aria-label={t('detail.save_aria')}
                            onClick={saveEdit}
                            title={t('detail.save_aria')}
                            disabled={updateTriggerMutation.isPending}
                          >
                            <Icon name="check" size={14} />
                          </button>
                          <button
                            className="cw-trigger-action"
                            type="button"
                            aria-label={t('detail.cancel_aria')}
                            onClick={cancelEdit}
                            title={t('detail.cancel_aria')}
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
                          {isCron ? 'schedule' : trigger.kind}
                        </span>
                        {trigger.spec.kind === 'cron' ? (
                          <>
                            <span className="cw-trigger-summary">{summarizeCron(trigger.spec.expr)}</span>
                            <span className="cw-trigger-meta">{trigger.spec.tz ?? 'UTC'}</span>
                          </>
                        ) : (
                          <>
                            <code className="cw-trigger-expr">whk_{trigger.id.slice(0, 6)}</code>
                            <span className="cw-trigger-meta">{t('detail.bearer_token')}</span>
                          </>
                        )}
                        <label className="cw-trigger-enabled">
                          <span>{trigger.enabled ? t('detail.trigger_enabled') : t('detail.trigger_disabled')}</span>
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
                          aria-label={t('detail.edit_trigger_aria')}
                          onClick={() => startEdit(trigger)}
                          disabled={!isCron}
                          title={!isCron ? t('detail.webhook_not_editable') : t('detail.edit')}
                        >
                          <Icon name="settings" size={14} />
                        </button>
                        <button
                          className={`cw-trigger-action${pendingTriggerDeleteId === trigger.id ? ' is-armed' : ''}`}
                          type="button"
                          aria-label={pendingTriggerDeleteId === trigger.id ? t('detail.delete_again') : t('detail.delete_trigger_aria')}
                          onClick={() => removeTrigger(trigger.id)}
                        >
                          {pendingTriggerDeleteId === trigger.id
                            ? t('detail.delete_again')
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
          <h2>{t('detail.danger_zone')}</h2>
          <p>{t('detail.danger_body')}</p>
          <button
            className="cw-btn-secondary cw-btn-destructive"
            type="button"
            onClick={() => setShowDeleteAutomationConfirm(true)}
            disabled={deleteAutomationMutation.isPending}
          >
            <Icon name="trash" size={14} /> {t('detail.delete_automation')}
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
          discardLabel={discardArmed ? t('detail.delete_again') : t('detail.discard_trigger')}
          discardArmed={discardArmed}
        />
      )}
      {showDeleteAutomationConfirm && (
        <ConfirmDialog
          title={t('detail.delete_confirm_title')}
          body={t('detail.delete_confirm_body', { name: automation.name })}
          confirmLabel={t('detail.delete_confirm_label')}
          destructive
          pending={deleteAutomationMutation.isPending}
          onConfirm={() => deleteAutomationMutation.mutate()}
          onClose={() => setShowDeleteAutomationConfirm(false)}
        />
      )}
    </section>
  );
}
