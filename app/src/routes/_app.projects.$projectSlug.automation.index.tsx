import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Icon } from '@/components/Icon';
import { IconButton } from '@/components/uiPrimitives';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { summarizeCron } from '@/components/SchedulePicker';
import { cancelRun as cancelRunApi, createRun, listAutomations, listRunEvents, listRuns, listTriggers } from '@/api/automations';
import { listMessages } from '@/api/messages';
import { getModelCatalog, modelLabel } from '@/api/models';
import { getAgentSurface } from '@/domain/agentSurfaces';
import { formatMessageTime } from '@/lib/formatMessageTime';
import { useDuplicateSession } from '@/lib/useDuplicateSession';
import { shortSessionId } from '@/lib/sessionId';
import { loadNs } from '@/i18n/loader';
import type { Automation, Message, Run, Trigger } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation/')({
  loader: () => loadNs('automation'),
  component: AutomationsPage,
});

type RunStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
type TriggerKind = 'cron' | 'webhook' | 'manual';

interface RunEventLike { id: number; ts: string; kind: string; detail?: string }

const PAGE_SIZE = 15;
const MAX_PAGES_ALL = 10;
const ALL_MODE_CAP = PAGE_SIZE * MAX_PAGES_ALL;
const SINGLE_MODE_CAP = 200;

const STATUS_KEY: Record<RunStatus, string> = {
  queued: 'status.queued', running: 'status.running', succeeded: 'status.succeeded', failed: 'status.failed', cancelled: 'status.cancelled',
};

const TRIGGER_KIND_KEY: Record<TriggerKind, string> = {
  cron: 'trigger_kind.schedule', webhook: 'trigger_kind.webhook', manual: 'trigger_kind.manual',
};

function formatRunWhen(run: Run, t: TFunction<'automation'>): string {
  const parsed = Date.parse(run.createdAt);
  if (Number.isNaN(parsed)) return run.createdAt.slice(0, 10);
  const diffSec = Math.max(0, Math.round((Date.now() - parsed) / 1000));
  if (diffSec < 60) return t('relative.just_now');
  const min = Math.round(diffSec / 60);
  if (min < 60) return t('relative.minutes_ago', { count: min });
  const hr = Math.round(min / 60);
  if (hr < 24) return t('relative.hours_ago', { count: hr });
  const day = Math.round(hr / 24);
  if (day < 7) return day === 1 ? t('relative.yesterday') : t('relative.days_ago', { count: day });
  return run.createdAt.slice(0, 10);
}

function formatRunDuration(run: Run, t: TFunction<'automation'>): string {
  if (run.status === 'queued') return '—';
  if (run.status === 'running') return t('relative.running');
  const start = Date.parse(run.createdAt);
  const end = Date.parse(run.updatedAt);
  if (Number.isNaN(start) || Number.isNaN(end) || end < start) return '—';
  const sec = Math.max(0, Math.round((end - start) / 1000));
  if (sec < 60) return t('relative.dur_s', { count: sec });
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return s === 0 ? t('relative.dur_m', { count: m }) : t('relative.dur_ms', { m, s });
}

function formatScheduledAt(iso: string, t: TFunction<'automation'>): string {
  const parsed = Date.parse(iso);
  if (Number.isNaN(parsed)) return iso.slice(0, 16).replace('T', ' ');
  const diffSec = Math.round((parsed - Date.now()) / 1000);
  if (diffSec < 0) {
    const ago = -diffSec;
    if (ago < 60) return t('relative.just_now');
    const min = Math.round(ago / 60);
    if (min < 60) return t('relative.minutes_ago', { count: min });
    const hr = Math.round(min / 60);
    if (hr < 24) return t('relative.hours_ago', { count: hr });
    const day = Math.round(hr / 24);
    return day === 1 ? t('relative.yesterday') : t('relative.days_ago', { count: day });
  }
  if (diffSec < 60) return t('relative.soon');
  const min = Math.round(diffSec / 60);
  if (min < 60) return t('relative.in_minutes', { count: min });
  const hr = Math.round(min / 60);
  if (hr < 24) return t('relative.in_hours', { count: hr });
  const day = Math.round(hr / 24);
  if (day === 1) return t('relative.tomorrow');
  if (day < 7) return t('relative.in_days', { count: day });
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

function triggerSummaryText(trig: Trigger, t: TFunction<'automation'>): string {
  if (trig.spec.kind === 'cron') {
    const cron = summarizeCron(trig.spec.expr, t);
    return trig.spec.tz ? `${cron}\n${trig.spec.tz}` : `${cron}\nUTC`;
  }
  return `whk_${trig.id.slice(0, 6)}`;
}

function AutomationsPage() {
  const { projectSlug } = Route.useParams();
  const navigate = useNavigate();
  const { t } = useTranslation('automation');
  const [selectedAutomationId, setSelectedAutomationId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<RunStatus | 'all'>('all');
  const [triggerFilter, setTriggerFilter] = useState<TriggerKind | 'all'>('all');
  const [page, setPage] = useState(0);

  const catalogQuery = useQuery({
    queryKey: ['models', projectSlug],
    queryFn: () => getModelCatalog(projectSlug),
    staleTime: 5 * 60_000,
  });

  const automationsQuery = useQuery({
    queryKey: ['automations', projectSlug],
    queryFn: () => listAutomations(projectSlug),
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
    if (statusFilter !== 'all' && effectiveStatus(run) !== statusFilter) return false;
    if (triggerFilter !== 'all' && runTriggerKind(run) !== triggerFilter) return false;
    return true;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [displayRuns, statusFilter, triggerFilter, cancelledIds, triggerById]);

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
    ? displayRuns.find((r) => r.id === selectedRunId) ?? null
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
      to: '/projects/$projectSlug/automation/$automationId',
      params: { projectSlug, automationId },
    });
  };

  const manualRunMutation = useMutation({
    mutationFn: (automationId: string) => createRun(automationId),
    onSuccess: (_, automationId) => {
      void queryClient.invalidateQueries({ queryKey: ['runs', automationId] });
    },
  });
  const duplicateMutation = useDuplicateSession(projectSlug);
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
    const forkable = !cancellable;
    const trigger = run.triggerId ? triggerById[run.triggerId] ?? null : null;
    const agentSurface = run.agentType ? getAgentSurface(run.agentType) : null;
    const runModelLabel = run.model ? modelLabel(catalogQuery.data, run.model) : t('run_detail.recommended_model');
    return (
    <>
      <header className="cw-run-drawer-head">
        <div className="cw-run-drawer-title">
          <StatusDot status={status} />
          <strong>{automationById[run.automationId]?.name}</strong>
          <span className="cw-run-attempt">#{shortRunId(run)}</span>
        </div>
        <div className="cw-run-drawer-actions">
          <IconButton
            icon="sticky-notes"
            label={t('run_detail.duplicate_label')}
            title={
              !forkable ? t('run_detail.duplicate_disabled')
              : duplicateMutation.isPending ? t('run_detail.duplicating')
              : t('run_detail.duplicate_title')
            }
            expandedText={duplicateMutation.isPending ? t('run_detail.duplicating') : t('run_detail.duplicate_expanded')}
            confirmText={t('run_detail.duplicate_confirm')}
            disabled={!forkable || duplicateMutation.isPending}
            onClick={() => duplicateMutation.mutate(run.sessionId, {
              onSuccess: (newSession) => {
                navigate({
                  to: '/projects/$projectSlug/sessions/$sessionPrefix',
                  params: { projectSlug, sessionPrefix: shortSessionId(newSession.id) },
                });
              },
            })}
          />
          {cancellable && (
            <button
              type="button"
              className="cw-btn-secondary cw-btn-destructive cw-run-cancel"
              onClick={() => requestCancel(run)}
              title={t('run_detail.cancel_title')}
            >
              <Icon name="x" size={14} /> {t('run_detail.cancel')}
            </button>
          )}
          <button
            type="button"
            className="cw-run-drawer-close"
            onClick={() => setSelectedRunId(null)}
            aria-label={t('run_detail.close_aria')}
          >
            <Icon name="x" size={16} />
          </button>
        </div>
        <div className="cw-run-drawer-meta">
          <TriggerBadge trigger={trigger} placement="below-start" />
          <span>{formatRunWhen(run, t)}</span>
          <span>·</span>
          <span>
            {t(STATUS_KEY[status])}
            {status === 'queued'
              ? `(${t('run_detail.scheduled', { when: formatScheduledAt(run.scheduledFor, t) })})`
              : `(${t('run_detail.duration', { value: formatRunDuration(run, t) })})`}
          </span>
          {/* force the session info (agent + model) onto its own line */}
          <span className="cw-run-meta-break" aria-hidden />
          {agentSurface && (
            <>
              <span
                className="cw-session-agent-chip"
                data-agent={agentSurface.id}
                title={t('run_detail.agent_chip_title')}
              >
                <Icon name={agentSurface.icon} size={11} />
                <span>{agentSurface.label}</span>
              </span>
              <span>·</span>
            </>
          )}
          <span title={t('run_detail.model_title')}>{runModelLabel}</span>
        </div>
      </header>

      <div className="cw-run-preview cw-messages">
        {previewMessages.length === 0 ? (
          <p className="cw-run-preview-empty">{t('run_detail.preview_empty')}</p>
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
                    <b>{isAi ? t('run_detail.msg_agent') : t('run_detail.msg_prompt')}</b>
                    <time>{formatMessageTime(msg.createdAt)}</time>
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
          {t('run_detail.event_logs')}
          <span className="cw-event-logs-count">{runEvents.length}</span>
        </summary>
        {runEvents.length === 0 ? (
          <p className="cw-run-preview-empty">{t('run_detail.events_empty')}</p>
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
          <h1>{t('list.title')}</h1>
          <p>{t('list.subtitle')}</p>
        </div>
      </header>

      <div className={`cw-automation-grid ${isWide ? '' : 'cw-automation-grid--narrow'}`}>
        <aside className="cw-automation-rail">
          <header className="cw-rail-header">
            <h2 className="cw-rail-title">{t('list.rail_title')}</h2>
            <button
              type="button"
              className="cw-rail-create"
              aria-label={t('list.rail_new_aria')}
              title={t('list.rail_new_aria')}
              onClick={() => navigate({ to: '/projects/$projectSlug/automation/new', params: { projectSlug } })}
            >
              <Icon name="plus" size={14} /> {t('list.rail_new')}
            </button>
          </header>

          <div className="cw-rail-list">
          <button
            type="button"
            className={`cw-rail-row ${selectedAutomationId === null ? 'is-active' : ''}`}
            onClick={() => setSelectedAutomationId(null)}
          >
            <span className="cw-rail-name">{t('list.all_automations')}</span>
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
                  <span className="cw-rail-off" title={t('list.off_title')}>{t('list.off')}</span>
                )}
                <span className="cw-rail-actions">
                  {!isDisabled(automation.id) && (
                    <RailAction
                      label={t('list.manual_run')}
                      icon="rocket"
                      onClick={() => triggerManualRun(automation.id)}
                    />
                  )}
                  <RailAction
                    label={t('list.settings')}
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
                        <span className="cw-trigger-badge cw-trigger-manual">{t('list.manual_only')}</span>
                        <span className="cw-rail-trigger-meta">{t('list.no_triggers')}</span>
                      </li>
                    ) : triggersByAutomation[automation.id].map((trig) => (
                      <li key={trig.id}>
                        <span className={`cw-trigger-badge cw-trigger-${trig.kind}`}>{t(TRIGGER_KIND_KEY[trig.kind as TriggerKind])}</span>
                        <span className="cw-rail-trigger-meta">{triggerSummaryText(trig, t)}</span>
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
              label={t('list.context_status')}
              value={statusFilter}
              onChange={setStatusFilter}
              options={[
                { value: 'all',       label: t('list.status_all') },
                { value: 'queued',    label: t('status.queued') },
                { value: 'running',   label: t('status.running') },
                { value: 'succeeded', label: t('status.succeeded') },
                { value: 'failed',    label: t('status.failed') },
                { value: 'cancelled', label: t('status.cancelled') },
              ]}
            />
            <FilterSelect<TriggerKind | 'all'>
              label={t('list.context_trigger')}
              value={triggerFilter}
              onChange={setTriggerFilter}
              options={[
                { value: 'all',     label: t('list.trigger_all') },
                { value: 'cron',    label: t('trigger_kind.schedule') },
                { value: 'webhook', label: t('trigger_kind.webhook') },
                { value: 'manual',  label: t('trigger_kind.manual') },
              ]}
            />
            {(statusFilter !== 'all' || triggerFilter !== 'all') && (
              <button
                type="button"
                className="cw-filter-reset"
                onClick={() => { setStatusFilter('all'); setTriggerFilter('all'); }}
              >
                <Icon name="rotate-ccw" size={12} /> {t('list.reset_filter')}
              </button>
            )}
            <span className="cw-runs-count">{t('list.runs_count', { count: visibleRuns.length })}</span>
          </div>

          {visibleRuns.length === 0 ? (
            <div className="cw-runs-empty">
              <Icon name="calendar" size={20} />
              <b>{t('list.empty_title')}</b>
              <p>{t('list.empty_hint')}</p>
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
                        <strong>{automation?.name ?? t('list.deleted')}</strong>
                        <span className="cw-run-attempt">#{shortRunId(run)}</span>
                      </span>
                      <TriggerBadge trigger={run.triggerId ? triggerById[run.triggerId] ?? null : null} />
                      <span className="cw-run-duration">
                        {effectiveStatus(run) === 'queued'
                          ? formatScheduledAt(run.scheduledFor, t)
                          : formatRunDuration(run, t)}
                      </span>
                      <span className="cw-run-when">{formatRunWhen(run, t)}</span>
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
                <Icon name="chevron-left" size={12} /> {t('list.prev')}
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
                {t('list.next')} <Icon name="chevron-right" size={12} />
              </button>
            </footer>
          )}
        </main>

        {isWide && (
          <aside className="cw-run-drawer">
            {selectedRun ? renderRunDetail(selectedRun) : (
              <div className="cw-run-drawer-empty">
                <Icon name="message-square" size={22} />
                <b>{t('list.select_run_title')}</b>
                <p>{t('list.select_run_hint')}</p>
              </div>
            )}
          </aside>
        )}
      </div>

      {pendingManualRun && (
        <ConfirmDialog
          title={t('list.manual_confirm_title')}
          body={t('list.manual_confirm_body', { name: pendingManualRun.name })}
          confirmLabel={t('list.manual_confirm_label')}
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
  const { t } = useTranslation('automation');
  const title = t(STATUS_KEY[status]);
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
  const { t } = useTranslation('automation');
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
  const label = t(TRIGGER_KIND_KEY[kind]);

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
              <span className="cw-trigger-popover-row"><span>{t('trigger_badge.when')}</span><code>{summarizeCron(trigger.spec.expr, t)}</code></span>
              {trigger.spec.tz && (
                <span className="cw-trigger-popover-row"><span>{t('trigger_badge.tz')}</span><code>{trigger.spec.tz}</code></span>
              )}
              {trigger.nextFireAt && (
                <span className="cw-trigger-popover-row"><span>{t('trigger_badge.next')}</span><code>{trigger.nextFireAt.slice(0, 16).replace('T', ' ')}</code></span>
              )}
            </>
          )}
          {trigger.spec.kind === 'webhook' && (
            <span className="cw-trigger-popover-row"><span>{t('trigger_badge.token')}</span><code>whk_{trigger.id.slice(0, 6)}</code></span>
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
  return (
    <IconButton
      icon={icon}
      label={label}
      onClick={onClick}
      disabled={disabled}
      className="cw-rail-action"
      iconSize={13}
      stopPropagation
    />
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
