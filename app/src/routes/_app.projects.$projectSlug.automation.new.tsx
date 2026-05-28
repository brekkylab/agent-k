import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Icon } from '@/components/Icon';
import { SchedulePicker, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';
import { createAutomation, createTrigger, deleteAutomation } from '@/api/automations';
import { ApiError } from '@/api/client';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation/new')({
  component: NewAutomationPage,
});

type InitialTriggerKind = 'none' | 'cron' | 'webhook';

function NewAutomationPage() {
  const { projectSlug } = Route.useParams();
  const navigate = useNavigate();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [prompts, setPrompts] = useState<string[]>(['']);
  const [triggerKind, setTriggerKind] = useState<InitialTriggerKind>('cron');
  const [schedule, setSchedule] = useState<SchedulePickerValue>({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });

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
        <Icon name="arrow-left" size={14} /> Automations
      </button>

      <form onSubmit={handleSubmit}>
        <header className="cw-page-head">
          <div>
            <h1>새 Automation</h1>
            <p>이름과 프롬프트를 정의하고, 필요하면 첫 트리거를 함께 설정합니다.</p>
          </div>
          <div>
            <button className="cw-btn-secondary" type="button" onClick={goBack}>Cancel</button>
            <button
              className="cw-btn-primary"
              type="submit"
              disabled={!canSubmit || createMutation.isPending}
            >
              <Icon name="check" size={14} /> {createMutation.isPending ? 'Creating…' : 'Create'}
            </button>
          </div>
        </header>

        <div className="cw-settings-stack">
          <section className="cw-settings-card">
            <h2>일반</h2>
            <label className="cw-settings-field">
              <span>이름</span>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="예: Daily summary"
                autoFocus
                required
              />
            </label>
            <label className="cw-settings-field">
              <span>설명</span>
              <textarea
                rows={3}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="이 automation이 무엇을 하는지 (선택)"
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
            <h2>첫 트리거</h2>
            <p className="cw-settings-hint">나중에 추가하거나 더 붙일 수도 있습니다.</p>

            <div className="cw-trigger-picker" role="radiogroup" aria-label="Initial trigger">
              {(['cron', 'webhook', 'none'] as InitialTriggerKind[]).map((kind) => {
                const active = triggerKind === kind;
                const label = kind === 'cron' ? 'Recurring'
                  : kind === 'webhook' ? 'Webhook'
                  : 'None (수동 전용)';
                const sub = kind === 'cron' ? '일정에 따라 반복 실행'
                  : kind === 'webhook' ? '외부 시스템에서 호출'
                  : '트리거 없이 수동 실행만';
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
                  생성 직후 한 번만 토큰이 노출됩니다. 안전한 곳에 보관하세요.
                </p>
              </div>
            )}

            {triggerKind === 'none' && (
              <div className="cw-trigger-detail">
                <p className="cw-settings-hint">
                  트리거 없이 생성합니다. 목록에서 수동 실행 버튼으로 호출할 수 있습니다.
                </p>
              </div>
            )}
          </section>
        </div>
      </form>

      {submitError && (
        <p className="cw-settings-hint" role="alert" style={{ color: 'var(--cw-destructive)', marginTop: 12 }}>
          생성 실패: {submitError}
        </p>
      )}

      {revealedToken && (
        <WebhookTokenDialog
          token={revealedToken}
          curlSample={`curl -X POST \\\n  -H 'Authorization: Bearer ${revealedToken}' \\\n  https://api.example.com/webhooks/automations`}
          onClose={() => { setRevealedToken(null); setCreatedAutomationId(null); goBack(); }}
          onDiscard={createdAutomationId ? handleDiscard : undefined}
          discardLabel="취소"
        />
      )}
    </section>
  );
}
