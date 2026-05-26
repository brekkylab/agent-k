import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { MouseEvent } from 'react';
import { createPortal } from 'react-dom';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { Icon } from '@/components/Icon';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { summarizeCron } from '@/components/SchedulePicker';
import { cancelRun as cancelRunApi, createRun, listAutomations, listRunEvents, listRuns, listTriggers } from '@/api/automations';
import { listMessages } from '@/api/messages';
import type { Automation, Message, Run, Trigger } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectId/automation/')({
  component: AutomationsPage,
});

type RunStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
type TriggerKind = 'cron' | 'webhook' | 'manual';

interface RunEventLike { id: number; ts: string; kind: string; detail?: string }

const PAGE_SIZE = 15;
const MAX_PAGES_ALL = 10;
const ALL_MODE_CAP = PAGE_SIZE * MAX_PAGES_ALL;
const SINGLE_MODE_CAP = 200;

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: 'Queued', running: 'Running', succeeded: 'Succeeded', failed: 'Failed', cancelled: 'Cancelled',
};

const TRIGGER_LABEL: Record<TriggerKind, string> = {
  cron: 'recurring', webhook: 'webhook', manual: 'manual',
};

function formatRunWhen(run: Run): string {
  const t = Date.parse(run.createdAt);
  if (Number.isNaN(t)) return run.createdAt.slice(0, 10);
  const diffSec = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (diffSec < 60) return '방금';
  const min = Math.round(diffSec / 60);
  if (min < 60) return `${min}분 전`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}시간 전`;
  const day = Math.round(hr / 24);
  if (day < 7) return day === 1 ? '어제' : `${day}일 전`;
  return run.createdAt.slice(0, 10);
}

function formatRunDuration(run: Run): string {
  if (run.status === 'queued') return '—';
  if (run.status === 'running') return '진행중';
  const start = Date.parse(run.createdAt);
  const end = Date.parse(run.updatedAt);
  if (Number.isNaN(start) || Number.isNaN(end) || end < start) return '—';
  const sec = Math.max(0, Math.round((end - start) / 1000));
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return s === 0 ? `${m}m` : `${m}m ${s}s`;
}

function formatScheduledAt(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso.slice(0, 16).replace('T', ' ');
  const diffSec = Math.round((t - Date.now()) / 1000);
  if (diffSec < 0) {
    const ago = -diffSec;
    if (ago < 60) return '방금';
    const min = Math.round(ago / 60);
    if (min < 60) return `${min}분 전`;
    const hr = Math.round(min / 60);
    if (hr < 24) return `${hr}시간 전`;
    const day = Math.round(hr / 24);
    return day === 1 ? '어제' : `${day}일 전`;
  }
  if (diffSec < 60) return '곧';
  const min = Math.round(diffSec / 60);
  if (min < 60) return `${min}분 후`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}시간 후`;
  const day = Math.round(hr / 24);
  if (day === 1) return '내일';
  if (day < 7) return `${day}일 후`;
  return iso.slice(0, 10);
}

function shortRunId(run: Run): string {
  return run.id.slice(0, 6);
}

function formatEventTime(iso: string): string {
  // ISO → HH:MM:SS; fall back to first 19 chars if Date.parse fails.
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso.slice(11, 19) || iso;
  const d = new Date(t);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
}

function formatEventPayload(payload: unknown): string | undefined {
  if (payload == null) return undefined;
  if (typeof payload === 'string') return payload;
  try {
    const s = JSON.stringify(payload);
    return s.length > 200 ? `${s.slice(0, 200)}…` : s;
  } catch {
    return undefined;
  }
}

function triggerSummaryText(t: Trigger): string {
  if (t.spec.kind === 'cron') {
    const cron = summarizeCron(t.spec.expr);
    return t.spec.tz ? `${cron}\n${t.spec.tz}` : `${cron}\nUTC`;
  }
  return `whk_${t.id.slice(0, 6)}`;
}

function AutomationsPage() {
  const { projectId } = Route.useParams();
  const navigate = useNavigate();
  const [selectedAutomationId, setSelectedAutomationId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<RunStatus | 'all'>('all');
  const [triggerFilter, setTriggerFilter] = useState<TriggerKind | 'all'>('all');
  const [page, setPage] = useState(0);

  const automationsQuery = useQuery({
    queryKey: ['automations', projectId],
    queryFn: () => listAutomations(projectId),
  });
  const automations: Automation[] = automationsQuery.data ?? [];
  const automationById = useMemo(
    () => Object.fromEntries(automations.map((a) => [a.id, a])),
    [automations],
  );
  const queryClient = useQueryClient();
  const runQueries = useQueries({
    queries: automations.map((a) => ({
      queryKey: ['runs', a.id],
      queryFn: () => listRuns(a.id),
      // Poll while any run in this automation is still in-flight.
      refetchInterval: (q: { state: { data?: Run[] } }) =>
        (q.state.data ?? []).some((r) => r.status === 'queued' || r.status === 'running')
          ? 4000 : false,
    })),
  });
  const triggerQueries = useQueries({
    queries: automations.map((a) => ({
      queryKey: ['triggers', a.id],
      queryFn: () => listTriggers(a.id),
    })),
  });
  const runsDataKey = runQueries.map((q) => q.dataUpdatedAt).join('|');
  const triggersDataKey = triggerQueries.map((q) => q.dataUpdatedAt).join('|');
  const allRuns: Run[] = useMemo(() => {
    const flat = runQueries.flatMap((q) => q.data ?? []);
    const sorted = [...flat].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    return sorted.slice(0, ALL_MODE_CAP);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runsDataKey]);

  const isSingleMode = selectedAutomationId !== null;
  const fanOutForSelected: Run[] = useMemo(() => {
    if (!isSingleMode) return [];
    const idx = automations.findIndex((a) => a.id === selectedAutomationId);
    return idx >= 0 ? runQueries[idx]?.data ?? [] : [];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isSingleMode, selectedAutomationId, runsDataKey, automations]);
  const fanOutCount = fanOutForSelected.length;
  const topUpQuery = useQuery({
    queryKey: ['runs', selectedAutomationId, 'topup', fanOutCount],
    queryFn: () => listRuns(selectedAutomationId!, {
      limit: SINGLE_MODE_CAP - fanOutCount,
      offset: fanOutCount,
    }),
    enabled: isSingleMode && fanOutCount > 0 && fanOutCount < SINGLE_MODE_CAP,
    refetchInterval: (q) =>
      (q.state.data ?? []).some((r) => r.status === 'queued' || r.status === 'running')
        ? 4000 : false,
  });
  const triggerById: Record<string, Trigger> = useMemo(
    () => Object.fromEntries(triggerQueries.flatMap((q) => q.data ?? []).map((t) => [t.id, t])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [triggersDataKey],
  );
  // allRuns is already sorted desc by createdAt; the first occurrence per
  // automationId is its latest run.
  const lastRunByAutomation: Record<string, Run> = useMemo(() => {
    const map: Record<string, Run> = {};
    for (const r of allRuns) {
      if (!map[r.automationId]) map[r.automationId] = r;
    }
    return map;
  }, [allRuns]);

  const [cancelledIds, setCancelledIds] = useState<Set<string>>(new Set());
  const cancelMutation = useMutation({
    mutationFn: ({ automationId, runId }: { automationId: string; runId: string }) =>
      cancelRunApi(automationId, runId),
    onSuccess: (_, vars) => {
      void queryClient.invalidateQueries({ queryKey: ['runs', vars.automationId] });
    },
  });
  const requestCancel = (run: Run) => {
    setCancelledIds((prev) => new Set(prev).add(run.id));
    cancelMutation.mutate({ automationId: run.automationId, runId: run.id });
  };

  const effectiveStatus = (run: Run): RunStatus =>
    cancelledIds.has(run.id) ? 'cancelled' : run.status;

  const runTriggerKind = (run: Run): TriggerKind => {
    if (!run.triggerId) return 'manual';
    const t = triggerById[run.triggerId];
    return (t?.kind as TriggerKind) ?? 'manual';
  };

  const isDisabled = (id: string) => {
    const a = automationById[id];
    return a ? !a.enabled : false;
  };

  const displayRuns: Run[] = isSingleMode
    ? [...fanOutForSelected, ...(topUpQuery.data ?? [])]
    : allRuns;
  const visibleRuns = useMemo(() => displayRuns.filter((run) => {
    if (!isSingleMode && selectedAutomationId && run.automationId !== selectedAutomationId) return false;
    if (statusFilter !== 'all' && effectiveStatus(run) !== statusFilter) return false;
    if (triggerFilter !== 'all' && runTriggerKind(run) !== triggerFilter) return false;
    return true;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [displayRuns, isSingleMode, selectedAutomationId, statusFilter, triggerFilter, cancelledIds, triggerById]);

  // Retry chains: the LATEST run in a chain (the head — nothing else points
  // to it via previousRunId) shows at depth 0; older attempts are listed
  // underneath at depth 1 (flat, no progressive nesting).
  const groupedRows = useMemo(() => {
    const visibleById = new Map(visibleRuns.map((r) => [r.id, r]));
    const pointedTo = new Set<string>();
    for (const r of visibleRuns) {
      if (r.previousRunId && visibleById.has(r.previousRunId)) pointedTo.add(r.previousRunId);
    }
    const out: { run: Run; depth: number }[] = [];
    for (const r of visibleRuns) {
      if (pointedTo.has(r.id)) continue;
      out.push({ run: r, depth: 0 });
      let cursor = r.previousRunId;
      while (cursor && visibleById.has(cursor)) {
        const prev = visibleById.get(cursor)!;
        out.push({ run: prev, depth: 1 });
        cursor = prev.previousRunId;
      }
    }
    return out;
  }, [visibleRuns]);

  useEffect(() => { setPage(0); }, [selectedAutomationId, statusFilter, triggerFilter]);

  const totalPages = Math.max(1, Math.ceil(groupedRows.length / PAGE_SIZE));
  const safePage = Math.min(page, totalPages - 1);
  const pagedRows = useMemo(
    () => groupedRows.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE),
    [groupedRows, safePage],
  );

  const triggersByAutomation: Record<string, Trigger[]> = useMemo(() => {
    const map: Record<string, Trigger[]> = {};
    for (const t of Object.values(triggerById)) {
      const arr = map[t.automationId] ?? [];
      arr.push(t);
      map[t.automationId] = arr;
    }
    return map;
  }, [triggerById]);

  const selectedAutomation = selectedAutomationId ? automationById[selectedAutomationId] : null;
  const selectedRun: Run | null = selectedRunId
    ? allRuns.find((r) => r.id === selectedRunId) ?? null
    : null;
  const messagesQuery = useQuery({
    queryKey: ['messages', selectedRun?.sessionId],
    queryFn: () => listMessages(selectedRun!.sessionId),
    enabled: Boolean(selectedRun),
  });
  const selectedRunLive = selectedRun
    ? selectedRun.status === 'queued' || selectedRun.status === 'running'
    : false;
  const eventsQuery = useQuery({
    queryKey: ['runEvents', selectedRun?.automationId, selectedRun?.id],
    queryFn: () => listRunEvents(selectedRun!.automationId, selectedRun!.id),
    enabled: Boolean(selectedRun),
    // Poll the audit log while the selected run is still in-flight.
    refetchInterval: selectedRunLive ? 4000 : false,
  });
  const previewMessages: Message[] = messagesQuery.data ?? [];
  const runEvents: RunEventLike[] = useMemo(
    () => (eventsQuery.data ?? []).map((e) => ({
      id: e.id,
      ts: formatEventTime(e.ts),
      kind: e.kind,
      detail: formatEventPayload(e.payload),
    })),
    [eventsQuery.data],
  );

  const toggleRun = (runId: string) => {
    setSelectedRunId((prev) => (prev === runId ? null : runId));
  };

  const openSettings = (automationId: string) => {
    navigate({
      to: '/projects/$projectId/automation/$automationId',
      params: { projectId, automationId },
    });
  };

  const manualRunMutation = useMutation({
    mutationFn: (automationId: string) => createRun(automationId),
    onSuccess: (_, automationId) => {
      void queryClient.invalidateQueries({ queryKey: ['runs', automationId] });
    },
  });
  const [pendingManualRun, setPendingManualRun] = useState<Automation | null>(null);
  const triggerManualRun = (automationId: string) => {
    if (manualRunMutation.isPending) return;
    const target = automationById[automationId];
    if (!target) return;
    setPendingManualRun(target);
  };
  const confirmManualRun = () => {
    if (!pendingManualRun) return;
    manualRunMutation.mutate(pendingManualRun.id);
    setPendingManualRun(null);
  };

  // Below this viewport, drop the right-side drawer column and expand the
  // selected row inline within the runs list instead.
  const isWide = useWideLayout('(min-width: 1440px)');

  const renderRunDetail = (run: Run) => {
    const status = effectiveStatus(run);
    const cancellable = status === 'queued' || status === 'running';
    const trigger = run.triggerId ? triggerById[run.triggerId] ?? null : null;
    return (
    <>
      <header className="cw-run-drawer-head">
        <div>
          <div className="cw-run-drawer-title">
            <StatusDot status={status} />
            <strong>{automationById[run.automationId]?.name}</strong>
            <span className="cw-run-attempt">#{shortRunId(run)}</span>
          </div>
          <div className="cw-run-drawer-meta">
            <TriggerBadge trigger={trigger} placement="below-start" />
            <span>{STATUS_LABEL[status]}</span>
            <span>·</span>
            <span>{formatRunWhen(run)}</span>
            <span>·</span>
            {status === 'queued' ? (
              <span>scheduled {formatScheduledAt(run.scheduledFor)}</span>
            ) : (
              <span>duration {formatRunDuration(run)}</span>
            )}
          </div>
        </div>
        <div className="cw-run-drawer-actions">
          {cancellable && (
            <button
              type="button"
              className="cw-btn-secondary cw-btn-destructive cw-run-cancel"
              onClick={() => requestCancel(run)}
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
            const isAi = msg.sender.kind === 'agent';
            return (
              <article
                key={msg.id}
                className={`cw-message ${isAi ? 'is-ai' : 'is-self'}`}
              >
                {isAi && <span className="cw-ai-chip">AI</span>}
                <div className="cw-message-body">
                  <div className="cw-message-meta">
                    <b>{isAi ? 'Agent' : 'Prompt'}</b>
                    <time>{formatRunWhen(run)}</time>
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
            <span className="cw-rail-count">{allRuns.length}</span>
          </button>

          {automations.map((automation) => {
            const runCount = allRuns.filter((r) => r.automationId === automation.id).length;
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
                <StatusDot status={lastRunByAutomation[automation.id]?.status ?? 'queued'} compact />
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
                <div className="cw-rail-extra">
                  {automation.description && (
                    <p className="cw-rail-desc">{automation.description}</p>
                  )}
                  <ul className="cw-rail-trigger-summary">
                    {(triggersByAutomation[automation.id] ?? []).length === 0 ? (
                      <li>
                        <span className="cw-trigger-badge cw-trigger-manual">manual only</span>
                        <span className="cw-rail-trigger-meta">트리거 없음</span>
                      </li>
                    ) : triggersByAutomation[automation.id].map((t) => (
                      <li key={t.id}>
                        <span className={`cw-trigger-badge cw-trigger-${t.kind}`}>{t.kind === 'cron' ? 'recurring' : t.kind}</span>
                        <span className="cw-rail-trigger-meta">{triggerSummaryText(t)}</span>
                      </li>
                    ))}
                  </ul>
                </div>
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
                <span>{selectedAutomation.description ?? ''}</span>
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
                { value: 'cron',    label: 'recurring' },
                { value: 'webhook', label: 'webhook' },
                { value: 'manual',  label: 'manual' },
              ]}
            />
            {(statusFilter !== 'all' || triggerFilter !== 'all') && (
              <button
                type="button"
                className="cw-filter-reset"
                onClick={() => { setStatusFilter('all'); setTriggerFilter('all'); }}
              >
                <Icon name="rotate-ccw" size={12} /> reset filter
              </button>
            )}
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
              {pagedRows.map(({ run, depth }) => {
                const automation = automationById[run.automationId];
                const active = selectedRunId === run.id;
                const isRetry = depth > 0;
                const chevronRot = isWide
                  ? (active ? 180 : 0)     // wide: < selected, > not
                  : (active ? -90 : 90);   // narrow: ^ selected, v not
                return (
                  <li key={run.id}>
                    <button
                      type="button"
                      className={`cw-run-row ${active ? 'is-active' : ''} ${isRetry ? 'is-retry' : ''}`}
                      style={isRetry ? { paddingLeft: 16 + depth * 24 } : undefined}
                      onClick={() => toggleRun(run.id)}
                    >
                      <StatusDot status={effectiveStatus(run)} />
                      <span className="cw-run-title">
                        <strong>{automation?.name ?? '(deleted)'}</strong>
                        <span className="cw-run-attempt">#{shortRunId(run)}</span>
                      </span>
                      <TriggerBadge trigger={run.triggerId ? triggerById[run.triggerId] ?? null : null} />
                      <span className="cw-run-duration">
                        {effectiveStatus(run) === 'queued'
                          ? formatScheduledAt(run.scheduledFor)
                          : formatRunDuration(run)}
                      </span>
                      <span className="cw-run-when">{formatRunWhen(run)}</span>
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

      {pendingManualRun && (
        <ConfirmDialog
          title="수동 실행 확인"
          body={`'${pendingManualRun.name}' automation을 지금 한 번 실행할까요?`}
          confirmLabel="실행"
          pending={manualRunMutation.isPending}
          onConfirm={confirmManualRun}
          onClose={() => setPendingManualRun(null)}
          confirmOnEnter
        />
      )}
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
  /** `null` ⇒ manual run (no backing trigger row). */
  trigger: Trigger | null;
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

  const kind: TriggerKind = trigger?.kind ?? 'manual';
  const label = TRIGGER_LABEL[kind];

  if (!trigger) {
    return <span className={`cw-trigger-badge cw-trigger-${kind}`}>{label}</span>;
  }

  return (
    <>
      <span
        ref={badgeRef}
        className={`cw-trigger-badge cw-trigger-${kind} cw-trigger-badge-host`}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        tabIndex={0}
      >
        {label}
      </span>
      {open && coords && createPortal(
        <span
          className={`cw-trigger-popover cw-trigger-popover-${placement} is-portal`}
          role="tooltip"
          style={{ top: coords.top, left: coords.left }}
        >
          <span className="cw-trigger-popover-row"><b>{label}</b></span>
          {trigger.spec.kind === 'cron' && (
            <>
              <span className="cw-trigger-popover-row"><span>when</span><code>{summarizeCron(trigger.spec.expr)}</code></span>
              {trigger.spec.tz && (
                <span className="cw-trigger-popover-row"><span>tz</span><code>{trigger.spec.tz}</code></span>
              )}
              {trigger.nextFireAt && (
                <span className="cw-trigger-popover-row"><span>next</span><code>{trigger.nextFireAt.slice(0, 16).replace('T', ' ')}</code></span>
              )}
            </>
          )}
          {trigger.spec.kind === 'webhook' && (
            <span className="cw-trigger-popover-row"><span>token</span><code>whk_{trigger.id.slice(0, 6)}</code></span>
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
