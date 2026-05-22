import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { MouseEvent } from 'react';
import { createPortal } from 'react-dom';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Icon } from '@/components/Icon';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';

export const Route = createFileRoute('/_app/projects/$projectId/automation/')({
  component: AutomationsPage,
});

type RunStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
type TriggerKind = 'cron' | 'webhook' | 'manual';
type EventKind =
  | 'triggered' | 'queued' | 'started'
  | 'succeeded' | 'failed' | 'retry_scheduled'
  | 'lease_lost' | 'step_started' | 'step_finished';

interface CronTriggerInfo { kind: 'cron'; expr: string; tz: string; nextLabel?: string }
interface WebhookTriggerInfo { kind: 'webhook'; tokenPreview: string; lastFiredLabel?: string }
interface ManualTriggerInfo { kind: 'manual'; actorLabel: string }
type TriggerInfo = CronTriggerInfo | WebhookTriggerInfo | ManualTriggerInfo;

interface MockAutomation {
  id: string;
  name: string;
  description: string;
  triggers: TriggerInfo[];
  lastRunStatus: RunStatus;
  disabled?: boolean;
}

interface MockRun {
  id: string;
  automationId: string;
  status: RunStatus;
  trigger: TriggerInfo;
  scheduledFor: string;
  durationLabel: string;
  whenLabel: string;
  attempt: number;
}

interface MockPreviewMessage {
  id: number;
  role: 'user' | 'assistant' | 'system';
  body: string;
}

interface MockEvent {
  id: number;
  ts: string;
  kind: EventKind;
  detail?: string;
}

const AUTOMATIONS: MockAutomation[] = [
  {
    id: 'a1', name: 'Daily summary',
    description: 'Recap yesterday\'s sessions',
    triggers: [{ kind: 'cron', expr: '0 10 * * *', tz: 'Asia/Seoul', nextLabel: '10:00 (14분 후)' }],
    lastRunStatus: 'running',
  },
  {
    id: 'a2', name: 'Weekly digest',
    description: 'Friday rollup to email',
    triggers: [{ kind: 'cron', expr: '0 9 * * 5', tz: 'Asia/Seoul', nextLabel: '금 09:00' }],
    lastRunStatus: 'succeeded',
  },
  {
    id: 'a3', name: 'On-call ping',
    description: 'External webhook hand-off',
    triggers: [{ kind: 'webhook', tokenPreview: 'whk_••••e8f2', lastFiredLabel: '어제 08:11' }],
    lastRunStatus: 'failed',
    disabled: true,
  },
  {
    id: 'a4', name: 'Backfill ledger',
    description: 'Manual one-shot reruns',
    triggers: [{ kind: 'manual', actorLabel: 'olive' }],
    lastRunStatus: 'succeeded',
  },
];

const CRON_DAILY:  CronTriggerInfo    = { kind: 'cron',    expr: '0 10 * * *', tz: 'Asia/Seoul', nextLabel: '10:00 (14분 후)' };
const CRON_WEEKLY: CronTriggerInfo    = { kind: 'cron',    expr: '0 9 * * 5',  tz: 'Asia/Seoul', nextLabel: '금 09:00' };
const WEBHOOK:     WebhookTriggerInfo = { kind: 'webhook', tokenPreview: 'whk_••••e8f2', lastFiredLabel: '어제 08:11' };
const MANUAL:      ManualTriggerInfo  = { kind: 'manual',  actorLabel: 'olive' };

const RUNS: MockRun[] = [
  { id: 'r1', automationId: 'a1', status: 'running',   trigger: CRON_DAILY,  scheduledFor: '10:00', durationLabel: '2m',  whenLabel: '3분 전',  attempt: 1 },
  { id: 'r2', automationId: 'a2', status: 'succeeded', trigger: CRON_WEEKLY, scheduledFor: '09:00', durationLabel: '45s', whenLabel: '2시간 전', attempt: 1 },
  { id: 'r3', automationId: 'a3', status: 'failed',    trigger: WEBHOOK,     scheduledFor: '08:11', durationLabel: '1m',  whenLabel: '어제',    attempt: 2 },
  { id: 'r4', automationId: 'a1', status: 'queued',    trigger: CRON_DAILY,  scheduledFor: '11:00', durationLabel: '—',   whenLabel: 'queued',  attempt: 1 },
  { id: 'r5', automationId: 'a4', status: 'succeeded', trigger: MANUAL,      scheduledFor: '어제',  durationLabel: '12s', whenLabel: '어제',    attempt: 1 },
  { id: 'r6',  automationId: 'a2', status: 'succeeded', trigger: CRON_WEEKLY, scheduledFor: '지난주',  durationLabel: '48s', whenLabel: '7일 전',   attempt: 1 },
  { id: 'r7',  automationId: 'a1', status: 'succeeded', trigger: CRON_DAILY,  scheduledFor: '10:00',   durationLabel: '1m 10s', whenLabel: '어제',     attempt: 2 },
  { id: 'r8',  automationId: 'a2', status: 'failed',    trigger: CRON_WEEKLY, scheduledFor: '09:00',   durationLabel: '20s',    whenLabel: '지난주',   attempt: 4 },
  { id: 'r9',  automationId: 'a3', status: 'succeeded', trigger: WEBHOOK,     scheduledFor: '12:30',   durationLabel: '8s',     whenLabel: '어제',     attempt: 5 },
  { id: 'r10', automationId: 'a4', status: 'succeeded', trigger: MANUAL,      scheduledFor: '어제',    durationLabel: '22s',    whenLabel: '어제',     attempt: 2 },
  { id: 'r11', automationId: 'a1', status: 'succeeded', trigger: CRON_DAILY,  scheduledFor: '10:00',   durationLabel: '1m 5s',  whenLabel: '그저께',   attempt: 3 },
  { id: 'r12', automationId: 'a2', status: 'succeeded', trigger: CRON_WEEKLY, scheduledFor: '09:00',   durationLabel: '50s',    whenLabel: '2주 전',   attempt: 5 },
  { id: 'r13', automationId: 'a3', status: 'failed',    trigger: WEBHOOK,     scheduledFor: '07:55',   durationLabel: '30s',    whenLabel: '3일 전',   attempt: 4 },
  { id: 'r14', automationId: 'a4', status: 'succeeded', trigger: MANUAL,      scheduledFor: '지난주',  durationLabel: '15s',    whenLabel: '지난주',   attempt: 3 },
  { id: 'r15', automationId: 'a1', status: 'queued',    trigger: CRON_DAILY,  scheduledFor: '11:00',   durationLabel: '—',      whenLabel: 'scheduled',attempt: 5 },
  { id: 'r16', automationId: 'a2', status: 'succeeded', trigger: CRON_WEEKLY, scheduledFor: '09:00',   durationLabel: '44s',    whenLabel: '3주 전',   attempt: 6 },
];

const PREVIEW_BY_RUN: Record<string, MockPreviewMessage[]> = {
  r1: [
    { id: 1, role: 'system',    body: 'Trigger: cron `0 10 * * *` · Asia/Seoul' },
    { id: 2, role: 'user',      body: '어제(2026-05-20) 진행된 세션을 요약해줘.' },
    { id: 3, role: 'assistant', body: '어제 활동 요약을 시작합니다…\n\n총 **5건의 세션**을 확인했습니다.\n- Klient kickoff (Olive, 14:02)\n- GTM brainstorm (Milo, 15:30)\n- Q2 plan review (Owen, 16:45)\n\n나머지 항목은 두 번째 프롬프트에서 정리합니다.' },
  ],
  r2: [
    { id: 1, role: 'system',    body: 'Trigger: cron `0 9 * * 5` · Asia/Seoul' },
    { id: 2, role: 'user',      body: '이번 주 핵심 결정 사항을 정리해줘.' },
    { id: 3, role: 'assistant', body: '이번 주 핵심 결정\n\n1. **GTM 전환** — 1차 타깃은 mid-market로 확정\n2. **Q2 OKR** — 매출 목표 +18%\n3. **온콜 로테이션** — 페이저 정책 개정안 합의' },
  ],
  r3: [
    { id: 1, role: 'system',    body: 'Trigger: webhook bearer=***' },
    { id: 2, role: 'user',      body: '인시던트 INC-4471에 대한 상황 요약을 작성해.' },
    { id: 3, role: 'assistant', body: '실패: 업스트림에서 **502 Bad Gateway**가 발생했습니다.\n시도 1 — 재시도 예산 초과, 시도 2 — 동일한 502.\n사람이 한 번 확인이 필요합니다.' },
  ],
  r4: [
    { id: 1, role: 'system', body: '이 run은 아직 큐에 있습니다 — 결과 없음.' },
  ],
  r5: [
    { id: 1, role: 'system',    body: 'Trigger: manual run by olive' },
    { id: 2, role: 'user',      body: '5/15~5/19 누락된 ledger 항목을 다시 처리해.' },
    { id: 3, role: 'assistant', body: '**10건**의 누락 항목을 재처리했습니다.\n성공 9건, 충돌 1건 (`txn-2026-05-17-031`).' },
  ],
  r6: [
    { id: 1, role: 'system',    body: '이전 주차 요약 — 참고용' },
    { id: 2, role: 'assistant', body: '주요 변경 없음. 매출 추이 안정적, 신규 risk 없음.' },
  ],
};

const EVENTS_BY_RUN: Record<string, MockEvent[]> = {
  r1: [
    { id: 1, ts: '10:00:00', kind: 'triggered',    detail: 'trigger=cron 0 10 * * *' },
    { id: 2, ts: '10:00:00', kind: 'queued' },
    { id: 3, ts: '10:00:02', kind: 'started',      detail: 'worker=worker-0' },
    { id: 4, ts: '10:00:02', kind: 'step_started', detail: 'prompt[0] · "어제 세션 요약"' },
    { id: 5, ts: '10:01:30', kind: 'step_finished',detail: 'tokens=1,182 (in 420 / out 762)' },
    { id: 6, ts: '10:01:30', kind: 'step_started', detail: 'prompt[1] · "오늘 우선순위 추출"' },
  ],
  r2: [
    { id: 1, ts: '09:00:00', kind: 'triggered' },
    { id: 2, ts: '09:00:00', kind: 'queued' },
    { id: 3, ts: '09:00:01', kind: 'started' },
    { id: 4, ts: '09:00:42', kind: 'succeeded', detail: 'tokens=812' },
  ],
  r3: [
    { id: 1, ts: '08:11:03', kind: 'triggered',       detail: 'webhook bearer=***' },
    { id: 2, ts: '08:11:03', kind: 'queued' },
    { id: 3, ts: '08:11:05', kind: 'started' },
    { id: 4, ts: '08:11:58', kind: 'failed',          detail: 'tool_call exceeded retry budget' },
    { id: 5, ts: '08:12:00', kind: 'retry_scheduled', detail: 'attempt=2 in 30s' },
    { id: 6, ts: '08:12:30', kind: 'started' },
    { id: 7, ts: '08:13:12', kind: 'failed',          detail: 'upstream 502' },
  ],
  r4: [{ id: 1, ts: '—', kind: 'queued', detail: 'scheduled for 11:00' }],
  r5: [
    { id: 1, ts: '14:22', kind: 'triggered', detail: 'manual run by olive' },
    { id: 2, ts: '14:22', kind: 'started' },
    { id: 3, ts: '14:22', kind: 'succeeded' },
  ],
  r6: [
    { id: 1, ts: '09:00', kind: 'triggered' },
    { id: 2, ts: '09:00', kind: 'started' },
    { id: 3, ts: '09:00', kind: 'succeeded' },
  ],
};

const PAGE_SIZE = 15;

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: 'Queued', running: 'Running', succeeded: 'Succeeded', failed: 'Failed', cancelled: 'Cancelled',
};

const TRIGGER_LABEL: Record<TriggerKind, string> = {
  cron: 'schedule', webhook: 'webhook', manual: 'manual',
};

function AutomationsPage() {
  const { projectId } = Route.useParams();
  const navigate = useNavigate();
  const [selectedAutomationId, setSelectedAutomationId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<RunStatus | 'all'>('all');
  const [triggerFilter, setTriggerFilter] = useState<TriggerKind | 'all'>('all');
  const [page, setPage] = useState(0);

  const automationById = useMemo(
    () => Object.fromEntries(AUTOMATIONS.map((a) => [a.id, a])),
    [],
  );

  const visibleRuns = useMemo(() => RUNS.filter((run) => {
    if (selectedAutomationId && run.automationId !== selectedAutomationId) return false;
    if (statusFilter !== 'all' && effectiveStatus(run) !== statusFilter) return false;
    if (triggerFilter !== 'all' && run.trigger.kind !== triggerFilter) return false;
    return true;
  }), [selectedAutomationId, statusFilter, triggerFilter]);

  const totalPages = Math.max(1, Math.ceil(visibleRuns.length / PAGE_SIZE));
  // Reset to first page when filters or selection narrow the list past the current page.
  useEffect(() => { setPage(0); }, [selectedAutomationId, statusFilter, triggerFilter]);
  const safePage = Math.min(page, totalPages - 1);
  const pagedRuns = useMemo(
    () => visibleRuns.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE),
    [visibleRuns, safePage],
  );

  const selectedAutomation = selectedAutomationId ? automationById[selectedAutomationId] : null;
  const selectedRun = selectedRunId ? RUNS.find((r) => r.id === selectedRunId) ?? null : null;
  const previewMessages = (selectedRunId ? PREVIEW_BY_RUN[selectedRunId] ?? [] : [])
    .filter((m) => m.role !== 'system');
  const runEvents = selectedRunId ? EVENTS_BY_RUN[selectedRunId] ?? [] : [];

  const toggleRun = (runId: string) => {
    setSelectedRunId((prev) => (prev === runId ? null : runId));
  };

  const openSettings = (automationId: string) => {
    navigate({
      to: '/projects/$projectId/automation/$automationId',
      params: { projectId, automationId },
    });
  };

  const triggerManualRun = (automationId: string) => {
    // Mock action — would POST /automations/:id/runs in the real impl.
    // eslint-disable-next-line no-console
    console.info('[mock] manual run requested', automationId);
  };

  // Cancellation is purely client-side mock — backend would expose a
  // `DELETE /automations/:id/runs/:run_id` (or similar) endpoint.
  const [cancelledIds, setCancelledIds] = useState<Set<string>>(new Set());

  // Automation-level disable, initialized from the mock data. In production
  // this maps to PATCH /automations/{id} with { enabled: false }.
  const [disabledIds] = useState<Set<string>>(
    () => new Set(AUTOMATIONS.filter((a) => a.disabled).map((a) => a.id)),
  );
  const isDisabled = (id: string) => disabledIds.has(id);
  const effectiveStatus = (run: MockRun): RunStatus =>
    cancelledIds.has(run.id) ? 'cancelled' : run.status;
  const cancelRun = (runId: string) => {
    setCancelledIds((prev) => {
      const next = new Set(prev);
      next.add(runId);
      return next;
    });
    // eslint-disable-next-line no-console
    console.info('[mock] cancel run', runId);
  };

  // Below this viewport, drop the right-side drawer column and expand the
  // selected row inline within the runs list instead.
  const isWide = useWideLayout('(min-width: 1440px)');

  const renderRunDetail = (run: MockRun) => {
    const status = effectiveStatus(run);
    const cancellable = status === 'queued' || status === 'running';
    return (
    <>
      <header className="cw-run-drawer-head">
        <div>
          <div className="cw-run-drawer-title">
            <StatusDot status={status} />
            <strong>{automationById[run.automationId]?.name}</strong>
            <span className="cw-run-attempt">#{run.attempt}</span>
          </div>
          <div className="cw-run-drawer-meta">
            <TriggerBadge trigger={run.trigger} placement="below-start" />
            <span>{STATUS_LABEL[status]}</span>
            <span>·</span>
            <span>{run.whenLabel}</span>
            <span>·</span>
            <span>duration {run.durationLabel}</span>
          </div>
        </div>
        <div className="cw-run-drawer-actions">
          {cancellable && (
            <button
              type="button"
              className="cw-btn-secondary cw-btn-destructive cw-run-cancel"
              onClick={() => cancelRun(run.id)}
              title="실행 취소"
            >
              <Icon name="x" size={14} /> Cancel
            </button>
          )}
          <button
            type="button"
            className="cw-run-drawer-close"
            onClick={() => setSelectedRunId(null)}
            aria-label="Close run detail"
          >
            <Icon name="x" size={16} />
          </button>
        </div>
      </header>

      <div className="cw-run-preview cw-messages">
        {previewMessages.length === 0 ? (
          <p className="cw-run-preview-empty">이 run에는 표시할 세션 내용이 없습니다.</p>
        ) : (
          previewMessages.map((msg) => {
            const isAi = msg.role === 'assistant';
            return (
              <article
                key={msg.id}
                className={`cw-message ${isAi ? 'is-ai' : 'is-self'}`}
              >
                {isAi && <span className="cw-ai-chip">AI</span>}
                <div className="cw-message-body">
                  <div className="cw-message-meta">
                    <b>{isAi ? 'Agent' : 'Prompt'}</b>
                    <time>{run.whenLabel}</time>
                  </div>
                  <div className={isAi ? 'cw-ai-prose' : 'cw-message-bubble'}>
                    {isAi
                      ? <MarkdownRenderer text={msg.body} />
                      : msg.body.split('\n').map((line, i) => (
                          <p key={`${msg.id}-${i}`}>{line || ' '}</p>
                        ))}
                  </div>
                </div>
              </article>
            );
          })
        )}
      </div>

      <details className="cw-event-logs">
        <summary>
          <Icon name="chevron-right" size={14} />
          Event logs
          <span className="cw-event-logs-count">{runEvents.length}</span>
        </summary>
        {runEvents.length === 0 ? (
          <p className="cw-run-preview-empty">기록된 이벤트가 없습니다.</p>
        ) : (
          <ol className="cw-event-feed">
            {runEvents.map((event) => (
              <li key={event.id} className={`cw-event-row cw-event-${event.kind}`}>
                <time>{event.ts}</time>
                <span className="cw-event-kind">{event.kind}</span>
                {event.detail && <span className="cw-event-detail">{event.detail}</span>}
              </li>
            ))}
          </ol>
        )}
      </details>
    </>
    );
  };

  return (
    <section className="cw-page cw-automation-page cw-page-enter">
      <header className="cw-page-head">
        <div>
          <h1>Automations</h1>
          <p>Agent에게 시킬 일을 미리 정의해두고, 일정·웹훅·수동 등 다양한 방법으로 실행합니다.</p>
        </div>
        <div>
          <button
            className="cw-btn-primary"
            type="button"
            onClick={() => navigate({ to: '/projects/$projectId/automation/new', params: { projectId } })}
          >
            <Icon name="plus" size={14} /> New automation
          </button>
        </div>
      </header>

      <div className={`cw-automation-grid ${isWide ? '' : 'cw-automation-grid--narrow'}`}>
        <aside className="cw-automation-rail">
          <header className="cw-rail-header">
            <h2 className="cw-rail-title">Automations</h2>
            <button
              type="button"
              className="cw-rail-create"
              aria-label="New automation"
              title="New automation"
              onClick={() => navigate({ to: '/projects/$projectId/automation/new', params: { projectId } })}
            >
              <Icon name="plus" size={14} /> New
            </button>
          </header>

          <div className="cw-rail-list">
          <button
            type="button"
            className={`cw-rail-row ${selectedAutomationId === null ? 'is-active' : ''}`}
            onClick={() => setSelectedAutomationId(null)}
          >
            <span className="cw-rail-name">All automations</span>
            <span className="cw-rail-count">{RUNS.length}</span>
          </button>

          {AUTOMATIONS.map((automation) => {
            const runCount = RUNS.filter((r) => r.automationId === automation.id).length;
            const active = selectedAutomationId === automation.id;
            return (
              <div
                key={automation.id}
                className={`cw-rail-row cw-rail-row-automation ${active ? 'is-active' : ''} ${isDisabled(automation.id) ? 'is-disabled' : ''}`}
                onClick={() => { setSelectedAutomationId(automation.id); setSelectedRunId(null); }}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    setSelectedAutomationId(automation.id);
                    setSelectedRunId(null);
                  }
                }}
              >
                <StatusDot status={automation.lastRunStatus} compact />
                <span className="cw-rail-name">{automation.name}</span>
                {isDisabled(automation.id) && (
                  <span className="cw-rail-off" title="비활성화됨">Off</span>
                )}
                <span className="cw-rail-actions">
                  {!isDisabled(automation.id) && (
                    <RailAction
                      label="수동 실행"
                      icon="rocket"
                      onClick={() => triggerManualRun(automation.id)}
                    />
                  )}
                  <RailAction
                    label="설정"
                    icon="settings"
                    onClick={() => openSettings(automation.id)}
                  />
                </span>
                <span className="cw-rail-count">{runCount}</span>
              </div>
            );
          })}
          </div>
        </aside>

        <main className="cw-runs-pane">
          <div className="cw-runs-filterbar">
            {selectedAutomation && (
              <div className="cw-runs-context">
                <strong>{selectedAutomation.name}</strong>
                <span>{selectedAutomation.description}</span>
              </div>
            )}
            <FilterSelect<RunStatus | 'all'>
              label="Status"
              value={statusFilter}
              onChange={setStatusFilter}
              options={[
                { value: 'all',       label: 'All status' },
                { value: 'queued',    label: 'Queued' },
                { value: 'running',   label: 'Running' },
                { value: 'succeeded', label: 'Succeeded' },
                { value: 'failed',    label: 'Failed' },
                { value: 'cancelled', label: 'Cancelled' },
              ]}
            />
            <FilterSelect<TriggerKind | 'all'>
              label="Trigger"
              value={triggerFilter}
              onChange={setTriggerFilter}
              options={[
                { value: 'all',     label: 'All triggers' },
                { value: 'cron',    label: 'schedule' },
                { value: 'webhook', label: 'webhook' },
                { value: 'manual',  label: 'manual' },
              ]}
            />
            <span className="cw-runs-count">{visibleRuns.length} runs</span>
          </div>

          {visibleRuns.length === 0 ? (
            <div className="cw-runs-empty">
              <Icon name="calendar" size={20} />
              <b>표시할 run이 없습니다</b>
              <p>필터를 풀거나 새로 실행해보세요.</p>
            </div>
          ) : (
            <ul className="cw-runs-list">
              {pagedRuns.map((run) => {
                const automation = automationById[run.automationId];
                const active = selectedRunId === run.id;
                const chevronRot = isWide
                  ? (active ? 180 : 0)     // wide: < selected, > not
                  : (active ? -90 : 90);   // narrow: ^ selected, v not
                return (
                  <li key={run.id}>
                    <button
                      type="button"
                      className={`cw-run-row ${active ? 'is-active' : ''}`}
                      onClick={() => toggleRun(run.id)}
                    >
                      <StatusDot status={effectiveStatus(run)} />
                      <span className="cw-run-title">
                        <strong>{automation?.name ?? '(deleted)'}</strong>
                        <span className="cw-run-attempt">#{run.attempt}</span>
                      </span>
                      <TriggerBadge trigger={run.trigger} />
                      <span className="cw-run-duration">{run.durationLabel}</span>
                      <span className="cw-run-when">{run.whenLabel}</span>
                      <Icon
                        name="chevron-right"
                        size={14}
                        style={{ transform: `rotate(${chevronRot}deg)`, transition: 'transform 140ms' }}
                      />
                    </button>
                    {!isWide && active && selectedRun && (
                      <div className="cw-run-inline-detail">
                        {renderRunDetail(selectedRun)}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}

          {visibleRuns.length > 0 && totalPages > 1 && (
            <footer className="cw-runs-pagination">
              <button
                type="button"
                className="cw-page-btn"
                disabled={safePage === 0}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
              >
                <Icon name="arrow-left" size={12} /> Prev
              </button>
              <span className="cw-page-indicator">
                {safePage * PAGE_SIZE + 1}–{Math.min((safePage + 1) * PAGE_SIZE, visibleRuns.length)} / {visibleRuns.length}
              </span>
              <button
                type="button"
                className="cw-page-btn"
                disabled={safePage >= totalPages - 1}
                onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              >
                Next <Icon name="chevron-right" size={12} />
              </button>
            </footer>
          )}
        </main>

        {isWide && (
          <aside className="cw-run-drawer">
            {selectedRun ? renderRunDetail(selectedRun) : (
              <div className="cw-run-drawer-empty">
                <Icon name="message-square" size={22} />
                <b>Run을 선택하세요</b>
                <p>Runs 목록에서 항목을 클릭하면 상세 정보가 여기에 표시됩니다.</p>
              </div>
            )}
          </aside>
        )}
      </div>
    </section>
  );
}

function StatusDot({ status, compact = false }: { status: RunStatus; compact?: boolean }) {
  const title = STATUS_LABEL[status];
  return <span className={`cw-status-dot cw-status-${status} ${compact ? 'is-compact' : ''}`} title={title} aria-label={title} />;
}

type TriggerPlacement = 'below' | 'below-start';

function TriggerBadge({
  trigger,
  placement = 'below',
}: {
  trigger: TriggerInfo;
  placement?: TriggerPlacement;
}) {
  const badgeRef = useRef<HTMLSpanElement | null>(null);
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(null);

  const recompute = useCallback(() => {
    const el = badgeRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const top = r.bottom + 6;
    const left = placement === 'below-start' ? r.left : r.left + r.width / 2;
    setCoords({ top, left });
  }, [placement]);

  useLayoutEffect(() => {
    if (open) recompute();
  }, [open, recompute]);

  useEffect(() => {
    if (!open) return;
    const onScroll = () => recompute();
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', onScroll);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', onScroll);
    };
  }, [open, recompute]);

  return (
    <>
      <span
        ref={badgeRef}
        className={`cw-trigger-badge cw-trigger-${trigger.kind} cw-trigger-badge-host`}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        tabIndex={0}
      >
        {TRIGGER_LABEL[trigger.kind]}
      </span>
      {open && coords && createPortal(
        <span
          className={`cw-trigger-popover cw-trigger-popover-${placement} is-portal`}
          role="tooltip"
          style={{ top: coords.top, left: coords.left }}
        >
          <span className="cw-trigger-popover-row"><b>{TRIGGER_LABEL[trigger.kind]}</b></span>
          {trigger.kind === 'cron' && (
            <>
              <span className="cw-trigger-popover-row"><span>expr</span><code>{trigger.expr}</code></span>
              <span className="cw-trigger-popover-row"><span>tz</span><code>{trigger.tz}</code></span>
              {trigger.nextLabel && <span className="cw-trigger-popover-row"><span>next</span><code>{trigger.nextLabel}</code></span>}
            </>
          )}
          {trigger.kind === 'webhook' && (
            <>
              <span className="cw-trigger-popover-row"><span>token</span><code>{trigger.tokenPreview}</code></span>
              {trigger.lastFiredLabel && <span className="cw-trigger-popover-row"><span>last</span><code>{trigger.lastFiredLabel}</code></span>}
            </>
          )}
          {trigger.kind === 'manual' && (
            <span className="cw-trigger-popover-row"><span>by</span><code>{trigger.actorLabel}</code></span>
          )}
        </span>,
        document.body,
      )}
    </>
  );
}

function RailAction({
  label, icon, onClick, disabled = false,
}: {
  label: string;
  icon: 'rocket' | 'settings';
  onClick: () => void;
  disabled?: boolean;
}) {
  const handle = (e: MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation();
    if (disabled) return;
    onClick();
  };
  return (
    <button
      type="button"
      className="cw-rail-action"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={handle}
    >
      <Icon name={icon} size={13} />
    </button>
  );
}

interface FilterOption<T extends string> { value: T; label: string }
function FilterSelect<T extends string>({
  label, value, onChange, options,
}: {
  label: string;
  value: T;
  onChange: (v: T) => void;
  options: FilterOption<T>[];
}) {
  return (
    <label className="cw-runs-filter">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((opt) => <option key={opt.value} value={opt.value}>{opt.label}</option>)}
      </select>
    </label>
  );
}

function useWideLayout(query: string): boolean {
  const [matches, setMatches] = useState(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return true;
    return window.matchMedia(query).matches;
  });
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(query);
    const handler = (e: MediaQueryListEvent) => setMatches(e.matches);
    mq.addEventListener('change', handler);
    setMatches(mq.matches);
    return () => mq.removeEventListener('change', handler);
  }, [query]);
  return matches;
}
