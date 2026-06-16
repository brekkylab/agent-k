import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import { SchedulePicker, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';
import { ComposerAgentPicker } from '@/components/chat/ComposerAgentPicker';
import { ComposerModelPicker } from '@/components/chat/ComposerModelPicker';
import { DEFAULT_AGENT_ID, type AgentId } from '@/domain/agentSurfaces';
import { getModelCatalog } from '@/api/models';
import { createAutomation, createTrigger, deleteAutomation } from '@/api/automations';
import { ApiError } from '@/api/client';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation/new')({
  loader: () => loadNs('automation'),
  component: NewAutomationPage,
});

type InitialTriggerKind = 'none' | 'cron' | 'webhook';

function NewAutomationPage() {
  const { projectSlug } = Route.useParams();
  const navigate = useNavigate();
  const { t } = useTranslation('automation');

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [prompts, setPrompts] = useState<string[]>(['']);
  const [agentId, setAgentId] = useState<AgentId>(DEFAULT_AGENT_ID);
  const [model, setModel] = useState<string | null>(null);
  const [triggerKind, setTriggerKind] = useState<InitialTriggerKind>('cron');
  const [schedule, setSchedule] = useState<SchedulePickerValue>({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });

  const catalog = useQuery({
    queryKey: ['models', projectSlug],
    queryFn: () => getModelCatalog(projectSlug),
    staleTime: 5 * 60_000,
  });

  const goBack = () => {
    navigate({ to: '/projects/$projectSlug/automation', params: { projectSlug } });
  };

  const canSubmit = name.trim().length > 0 && prompts.some((p) => p.trim().length > 0);

  const updatePrompt = (i: number, value: string) => {
    setPrompts((p) => p.map((line, idx) => (idx === i ? value : line)));
  };
  const addPrompt = () => setPrompts((p) => [...p, '']);
  const removePrompt = (i: number) => setPrompts((p) => p.filter((_, idx) => idx !== i));

  const [revealedToken, setRevealedToken] = useState<string | null>(null);
  const [createdAutomationId, setCreatedAutomationId] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  const discardMutation = useMutation({
    mutationFn: (id: string) => deleteAutomation(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['automations', projectSlug] });
    },
  });
  const handleDiscard = () => {
    const id = createdAutomationId;
    if (!id) return;
    setRevealedToken(null);
    setCreatedAutomationId(null);
    discardMutation.mutate(id);
  };

  const createMutation = useMutation({
    mutationFn: async () => {
      const automation = await createAutomation({
        projectRef: projectSlug,
        name: name.trim(),
        description: description.trim() ? description.trim() : null,
        prompts: prompts.filter((p) => p.trim().length > 0),
        agentType: agentId,
        model,
      });
      if (triggerKind === 'cron') {
        await createTrigger(automation.id, {
          kind: 'cron',
          expr: schedule.expr,
          tz: schedule.tz || null,
        });
        return { automationId: automation.id, webhookToken: null as string | null };
      }
      if (triggerKind === 'webhook') {
        const created = await createTrigger(automation.id, { kind: 'webhook' });
        return { automationId: automation.id, webhookToken: created.webhookToken };
      }
      return { automationId: automation.id, webhookToken: null as string | null };
    },
    onSuccess: (result) => {
      void queryClient.invalidateQueries({ queryKey: ['automations', projectSlug] });
      if (result.webhookToken) {
        setCreatedAutomationId(result.automationId);
        setRevealedToken(result.webhookToken);
      } else {
        goBack();
      }
    },
    onError: (err) => {
      setSubmitError(
        err instanceof ApiError ? err.message
          : err instanceof Error ? err.message
          : 'create failed',
      );
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit || createMutation.isPending) return;
    setSubmitError(null);
    createMutation.mutate();
  };

  return (
    <section className="cw-page cw-automation-settings cw-page-enter">
      <button className="cw-btn-secondary cw-back-link" type="button" onClick={goBack}>
        <Icon name="arrow-left" size={14} /> {t('back')}
      </button>

      <form onSubmit={handleSubmit}>
        <header className="cw-page-head">
          <div>
            <h1>{t('form.new_title')}</h1>
            <p>{t('form.new_subtitle')}</p>
          </div>
          <div>
            <button className="cw-btn-secondary" type="button" onClick={goBack}>{t('form.cancel')}</button>
            <button
              className="cw-btn-primary"
              type="submit"
              disabled={!canSubmit || createMutation.isPending}
            >
              <Icon name="check" size={14} /> {createMutation.isPending ? t('form.creating') : t('form.create')}
            </button>
          </div>
        </header>

        <div className="cw-settings-stack">
          <section className="cw-settings-card">
            <h2>{t('form.general')}</h2>
            <label className="cw-settings-field">
              <span>{t('form.name')}</span>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t('form.name_placeholder')}
                autoFocus
                required
              />
            </label>
            <label className="cw-settings-field">
              <span>{t('form.description')}</span>
              <textarea
                rows={3}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={t('form.description_placeholder')}
              />
            </label>
          </section>

          <section className="cw-settings-card">
            <header className="cw-settings-card-head">
              <h2>{t('form.prompts')}</h2>
              <button className="cw-btn-secondary" type="button" onClick={addPrompt}>
                <Icon name="plus" size={14} /> {t('form.add_prompt')}
              </button>
            </header>
            <p className="cw-settings-hint">{t('form.prompts_hint')}</p>
            <ol className="cw-prompt-list">
              {prompts.map((line, i) => (
                <li key={i}>
                  <span className="cw-prompt-index">{i + 1}</span>
                  <textarea
                    rows={2}
                    value={line}
                    onChange={(e) => updatePrompt(i, e.target.value)}
                    placeholder={t('form.prompt_placeholder')}
                  />
                  <button
                    className="cw-prompt-remove"
                    type="button"
                    aria-label={t('form.remove_prompt')}
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
            <h2>{t('form.agent_model')}</h2>
            <p className="cw-settings-hint">
              {t('form.agent_model_hint_new')}
            </p>
            <div
              className="cw-agent-pickwrap"
              data-agent={agentId}
              style={{ display: 'flex', flexDirection: 'column', gap: 12, alignItems: 'flex-start' }}
            >
              <ComposerAgentPicker value={agentId} onChange={setAgentId} standalone />
              <p className="cw-agent-pick-desc">{t(`agent.${agentId}.description`)}</p>
              <ComposerModelPicker
                catalog={catalog.data}
                agentType={agentId}
                value={model}
                onChange={setModel}
              />
            </div>
          </section>

          <section className="cw-settings-card">
            <h2>{t('form.first_trigger')}</h2>
            <p className="cw-settings-hint">{t('form.first_trigger_hint')}</p>

            <div className="cw-trigger-picker" role="radiogroup" aria-label={t('form.trigger_kind_aria')}>
              {(['cron', 'webhook', 'none'] as InitialTriggerKind[]).map((kind) => {
                const active = triggerKind === kind;
                const label = kind === 'cron' ? t('form.schedule')
                  : kind === 'webhook' ? t('form.webhook')
                  : t('form.none');
                const sub = kind === 'cron' ? t('form.schedule_sub')
                  : kind === 'webhook' ? t('form.webhook_sub')
                  : t('form.none_sub');
                return (
                  <label
                    key={kind}
                    className={`cw-trigger-option ${active ? 'is-active' : ''}`}
                  >
                    <input
                      type="radio"
                      name="triggerKind"
                      value={kind}
                      checked={active}
                      onChange={() => setTriggerKind(kind)}
                    />
                    <span className="cw-trigger-option-label">
                      <b>{label}</b>
                      <em>{sub}</em>
                    </span>
                  </label>
                );
              })}
            </div>

            {triggerKind === 'cron' && (
              <div className="cw-trigger-detail">
                <SchedulePicker value={schedule} onChange={setSchedule} />
              </div>
            )}

            {triggerKind === 'webhook' && (
              <div className="cw-trigger-detail">
                <p className="cw-settings-hint">
                  {t('form.webhook_token_hint')}
                </p>
              </div>
            )}

            {triggerKind === 'none' && (
              <div className="cw-trigger-detail">
                <p className="cw-settings-hint">
                  {t('form.none_hint')}
                </p>
              </div>
            )}
          </section>
        </div>
      </form>

      {submitError && (
        <p className="cw-settings-hint" role="alert" style={{ color: 'var(--cw-destructive)', marginTop: 12 }}>
          {t('form.create_error', { message: submitError })}
        </p>
      )}

      {revealedToken && (
        <WebhookTokenDialog
          token={revealedToken}
          curlSample={`curl -X POST \\\n  -H 'Authorization: Bearer ${revealedToken}' \\\n  https://api.example.com/webhooks/automations`}
          onClose={() => { setRevealedToken(null); setCreatedAutomationId(null); goBack(); }}
          onDiscard={createdAutomationId ? handleDiscard : undefined}
          discardLabel={t('form.webhook_discard')}
        />
      )}
    </section>
  );
}
