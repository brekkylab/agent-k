import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Icon } from '@/components/Icon';
import { SchedulePicker, type SchedulePickerValue } from '@/components/SchedulePicker';
import { WebhookTokenDialog } from '@/components/WebhookTokenDialog';

export const Route = createFileRoute('/_app/projects/$projectId/automation/new')({
  component: NewAutomationPage,
});

type InitialTriggerKind = 'none' | 'cron' | 'webhook';

function NewAutomationPage() {
  const { projectId } = Route.useParams();
  const navigate = useNavigate();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [prompts, setPrompts] = useState<string[]>(['']);
  const [triggerKind, setTriggerKind] = useState<InitialTriggerKind>('cron');
  const [schedule, setSchedule] = useState<SchedulePickerValue>({ expr: '0 9 * * 1,2,3,4,5', tz: 'Asia/Seoul' });

  const goBack = () => {
    navigate({ to: '/projects/$projectId/automation', params: { projectId } });
  };

  const canSubmit = name.trim().length > 0 && prompts.some((p) => p.trim().length > 0);

  const updatePrompt = (i: number, value: string) => {
    setPrompts((p) => p.map((line, idx) => (idx === i ? value : line)));
  };
  const addPrompt = () => setPrompts((p) => [...p, '']);
  const removePrompt = (i: number) => setPrompts((p) => p.filter((_, idx) => idx !== i));

  const [revealedToken, setRevealedToken] = useState<string | null>(null);
  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    // Mock submit — real impl posts to /automations then optionally /triggers.
    // eslint-disable-next-line no-console
    console.info('[mock] create automation', { name, description, prompts, triggerKind, schedule });
    if (triggerKind === 'webhook') {
      setRevealedToken(generateWebhookToken());
      return;
    }
    goBack();
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
            <button className="cw-btn-primary" type="submit" disabled={!canSubmit}>
              <Icon name="check" size={14} /> Create
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
                const label = kind === 'cron' ? 'Schedule'
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

      {revealedToken && (
        <WebhookTokenDialog
          token={revealedToken}
          curlSample={`curl -X POST \\\n  -H 'Authorization: Bearer ${revealedToken}' \\\n  https://api.example.com/webhooks/automations`}
          onClose={() => { setRevealedToken(null); goBack(); }}
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
