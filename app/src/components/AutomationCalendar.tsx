// Month calendar that unifies a project's schedule timeline: past slots show
// the actual runs that happened (real, with status), future slots show
// computed cron occurrences (predicted — nothing persisted, see
// listOccurrences). The split point is "now". Every instant (UTC) is bucketed
// and displayed in the browser's local timezone, matching standard calendars.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import { listOccurrences, listRunsInWindow } from '@/api/automations';
import type { Occurrence, Run, Trigger } from '@/domain/types';

type TriggerKind = 'cron' | 'webhook' | 'manual';

const MAX_PER_CELL = 4;
const DAY_POPOVER_CAP = 15;

// Per-automation accent. Avoids the run-status hues (red 25 / amber 75 /
// green 145) so an accent is never mistaken for a status, but spreads across
// the whole teal→pink arc with strong lightness variation between neighbours
// so the colours stay mutually distinct (not a cramped blue-purple band).
const PALETTE = [
  'oklch(0.66 0.12 180)', // teal
  'oklch(0.78 0.11 208)', // sky (light)
  'oklch(0.55 0.17 240)', // blue
  'oklch(0.46 0.15 268)', // indigo (dark)
  'oklch(0.64 0.20 300)', // violet (bright)
  'oklch(0.50 0.18 325)', // plum (dark)
  'oklch(0.74 0.14 350)', // pink (light)
];

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}
function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}
function addDays(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
}
function addMonths(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth() + n, 1);
}
function localDayKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}
function hhmm(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}
// Slot identity for de-duping a future occurrence against an already-created
// run for the same automation+minute (avoids showing both at the boundary).
function slotKey(automationId: string, iso: string): string {
  return `${automationId}@${iso.slice(0, 16)}`;
}

type CalEntry =
  | { kind: 'run'; time: Date; run: Run }
  | { kind: 'occ'; time: Date; occ: Occurrence };

export function AutomationCalendar({
  projectSlug,
  automationNameById,
  triggerById,
  filterAutomationId,
  statusFilter,
  triggerFilter,
  selectedKey,
  selectedRunId,
  onSelectOccurrence,
  onSelectRun,
}: {
  projectSlug: string;
  automationNameById: Record<string, string>;
  /** Resolves a run's trigger kind for the trigger filter. */
  triggerById: Record<string, Trigger>;
  /** When set, only this automation's fires/runs are shown. */
  filterAutomationId?: string | null;
  /** Status filter, shared with the list view ('all' = no filter). */
  statusFilter: string;
  /** Trigger-kind filter, shared with the list view ('all' = no filter). */
  triggerFilter: string;
  /** `${triggerId}@${fireAt}` of the selected future fire, for highlight. */
  selectedKey?: string | null;
  /** id of the selected past run, for highlight. */
  selectedRunId?: string | null;
  /** Clicking a future fire opens its (predicted) detail. `anchor` is the
   *  clicked element, for positioning a popover in the narrow layout. */
  onSelectOccurrence?: (occ: Occurrence, anchor: HTMLElement) => void;
  /** Clicking a past run opens the same run detail as the list view. */
  onSelectRun?: (run: Run, anchor: HTMLElement) => void;
}) {
  const { t, i18n } = useTranslation('automation');
  const [cursor, setCursor] = useState(() => startOfMonth(new Date()));

  const monthStart = startOfMonth(cursor);
  // Grid spans 6 weeks starting on the Sunday on/before the 1st.
  const gridStart = addDays(monthStart, -monthStart.getDay());
  const gridDays = useMemo(
    () => Array.from({ length: 42 }, (_, i) => addDays(gridStart, i)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [localDayKey(gridStart)],
  );
  const gridEnd = addDays(gridStart, 42); // exclusive

  // Past slots render as real runs, and occurrences are dropped client-side
  // when fireAt <= now — so there's no point expanding the past portion. Clamp
  // the query start to today when today is inside the window; a fully-past
  // window has no future fires at all, so skip the request entirely.
  const todayStart = startOfDay(new Date());
  const occFrom = gridStart > todayStart ? gridStart : todayStart;
  const hasFuture = occFrom < gridEnd;
  const occQuery = useQuery({
    queryKey: ['occurrences', projectSlug, localDayKey(gridStart), localDayKey(occFrom)],
    queryFn: () => listOccurrences(projectSlug, occFrom.toISOString(), gridEnd.toISOString()),
    enabled: hasFuture,
    staleTime: 60_000,
  });

  // Realized (past) runs for the visible window — windowed query so old months
  // aren't limited to the recently-fetched run set. Polls while any run in the
  // window is still in-flight so the calendar reflects status changes live.
  const runsQuery = useQuery({
    queryKey: ['calRuns', projectSlug, localDayKey(gridStart)],
    queryFn: () => listRunsInWindow(projectSlug, gridStart.toISOString(), gridEnd.toISOString()),
    staleTime: 30_000,
    refetchInterval: (q) =>
      (q.state.data ?? []).some((r) => r.status === 'queued' || r.status === 'running')
        ? 4000 : false,
  });

  // Past/future split point. Advanced once a minute (not per render, which
  // would thrash the entriesByDay memo) so a calendar left open across a fire
  // stops showing the now-past occurrence as a prediction — the real run takes
  // over once the run-window query picks it up.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  const matchAutomation = (id: string) => !filterAutomationId || id === filterAutomationId;

  // Assign accent by the automation's rank in the project list (insertion
  // order of automationNameById) rather than hashing its id: collision-free
  // for the first PALETTE.length automations and stable per automation across
  // months/filters. Beyond that, colors repeat (disambiguated by the label).
  const colorByAutomation = useMemo(() => {
    const map = new Map<string, string>();
    Object.keys(automationNameById).forEach((id, i) => map.set(id, PALETTE[i % PALETTE.length]!));
    return map;
  }, [automationNameById]);
  const automationColor = (id: string) => colorByAutomation.get(id) ?? PALETTE[0]!;
  const runTriggerKind = (run: Run): TriggerKind =>
    run.triggerId ? (triggerById[run.triggerId]?.kind as TriggerKind) ?? 'manual' : 'manual';
  // Occurrences are future schedule predictions — shown when the trigger filter
  // admits 'cron' and the status filter is either unset ('all') or the
  // dedicated 'scheduled' pseudo-status (which also hides all real runs, since
  // no run has that status).
  const showOccurrences =
    (triggerFilter === 'all' || triggerFilter === 'cron') &&
    (statusFilter === 'all' || statusFilter === 'scheduled');

  // Merge real runs (past) + predicted occurrences (future) into per-day
  // buckets, honoring the shared status / trigger-kind filters.
  const entriesByDay = useMemo(() => {
    const map = new Map<string, CalEntry[]>();
    const push = (e: CalEntry) => {
      const k = localDayKey(e.time);
      const arr = map.get(k);
      if (arr) arr.push(e);
      else map.set(k, [e]);
    };

    const runSlots = new Set<string>();
    for (const r of runsQuery.data ?? []) {
      if (!matchAutomation(r.automationId)) continue;
      if (statusFilter !== 'all' && r.status !== statusFilter) continue;
      if (triggerFilter !== 'all' && runTriggerKind(r) !== triggerFilter) continue;
      runSlots.add(slotKey(r.automationId, r.scheduledFor));
      push({ kind: 'run', time: new Date(r.scheduledFor), run: r });
    }
    if (showOccurrences) {
      for (const o of occQuery.data?.items ?? []) {
        if (!matchAutomation(o.automationId)) continue;
        if (Date.parse(o.fireAt) <= now) continue; // past → represented by a run
        if (runSlots.has(slotKey(o.automationId, o.fireAt))) continue; // already a run
        push({ kind: 'occ', time: new Date(o.fireAt), occ: o });
      }
    }

    for (const arr of map.values()) arr.sort((a, b) => a.time.getTime() - b.time.getTime());
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runsQuery.data, occQuery.data, triggerById, filterAutomationId, statusFilter, triggerFilter, showOccurrences, now]);

  // Legend reflects what's actually shown (post-filter).
  const legend = useMemo(() => {
    const seen = new Map<string, string>();
    for (const arr of entriesByDay.values()) for (const e of arr) {
      if (e.kind === 'run') seen.set(e.run.automationId, automationNameById[e.run.automationId] ?? e.run.automationId);
      else seen.set(e.occ.automationId, e.occ.automationName);
    }
    return [...seen.entries()];
  }, [entriesByDay, automationNameById]);

  const weekdays = t('sched.weekdays_short', { returnObjects: true }) as string[];

  const monthLabel = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { year: 'numeric', month: 'long' }).format(monthStart),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [i18n.language, monthStart.getFullYear(), monthStart.getMonth()],
  );

  const todayKey = localDayKey(new Date());
  const total = gridDays.reduce((n, d) => n + (entriesByDay.get(localDayKey(d))?.length ?? 0), 0);

  // "+N more" expands the day into a popover listing every entry.
  const [dayPopover, setDayPopover] = useState<{ day: Date; anchor: HTMLElement } | null>(null);

  const pickEntry = (e: CalEntry, anchor: HTMLElement) => {
    if (e.kind === 'run') onSelectRun?.(e.run, anchor);
    else onSelectOccurrence?.(e.occ, anchor);
  };

  // One entry button, shared by the day cells and the "+N more" popover.
  // `onActivate` lets the popover override the detail anchor (to the day cell)
  // and close itself, while cells just select with their own element.
  const renderEntry = (
    e: CalEntry,
    i: number,
    onActivate: (e: CalEntry, el: HTMLElement) => void,
  ) => {
    if (e.kind === 'run') {
      const { run } = e;
      const name = automationNameById[run.automationId] ?? run.automationId;
      const active = selectedRunId != null && run.id === selectedRunId;
      return (
        <button
          key={`r-${run.id}`}
          type="button"
          className={`cw-cal-event cw-cal-run ${active ? 'is-active' : ''}`}
          style={{ ['--cal-accent' as string]: automationColor(run.automationId) }}
          title={`${name} — ${hhmm(e.time)} · ${t(`status.${run.status}`)}`}
          onClick={(ev) => onActivate(e, ev.currentTarget)}
        >
          <span className={`cw-status-dot cw-status-${run.status} is-compact`} />
          <span className="cw-cal-event-time">{hhmm(e.time)}</span>
          <span className="cw-cal-event-name">{name}</span>
        </button>
      );
    }
    const { occ } = e;
    const active = selectedKey != null && `${occ.triggerId}@${occ.fireAt}` === selectedKey;
    return (
      <button
        key={`o-${occ.triggerId}-${i}`}
        type="button"
        className={`cw-cal-event cw-cal-occ ${active ? 'is-active' : ''}`}
        style={{ ['--cal-accent' as string]: automationColor(occ.automationId) }}
        title={`${occ.automationName} — ${hhmm(e.time)}${occ.tz ? ` · ${occ.tz}` : ''}`}
        onClick={(ev) => onActivate(e, ev.currentTarget)}
      >
        <span className="cw-cal-event-time">{hhmm(e.time)}</span>
        <span className="cw-cal-event-name">{occ.automationName}</span>
      </button>
    );
  };

  return (
    <div className="cw-cal">
      <header className="cw-cal-head">
        <div className="cw-cal-nav">
          <button
            type="button"
            className="cw-cal-navbtn"
            aria-label={t('calendar.prev_month')}
            onClick={() => setCursor((c) => addMonths(c, -1))}
          >
            <Icon name="chevron-left" size={16} />
          </button>
          <strong className="cw-cal-month">{monthLabel}</strong>
          <button
            type="button"
            className="cw-cal-navbtn"
            aria-label={t('calendar.next_month')}
            onClick={() => setCursor((c) => addMonths(c, 1))}
          >
            <Icon name="chevron-right" size={16} />
          </button>
          <button
            type="button"
            className="cw-cal-today"
            onClick={() => setCursor(startOfMonth(new Date()))}
          >
            {t('calendar.today')}
          </button>
        </div>
        <span className="cw-cal-count">{t('calendar.fires_count', { count: total })}</span>
      </header>

      {legend.length > 0 && (
        <ul className="cw-cal-legend">
          {legend.map(([id, name]) => (
            <li key={id}>
              <span className="cw-cal-legend-dot" style={{ background: automationColor(id) }} />
              <span>{name}</span>
            </li>
          ))}
        </ul>
      )}

      {occQuery.data?.truncated && (
        <p className="cw-cal-truncated">
          <Icon name="zap" size={12} /> {t('calendar.truncated')}
        </p>
      )}

      <div className="cw-cal-grid" role="grid" aria-label={monthLabel}>
        {weekdays.map((w) => (
          <div key={w} className="cw-cal-weekday" role="columnheader">{w}</div>
        ))}
        {gridDays.map((day) => {
          const key = localDayKey(day);
          const entries = entriesByDay.get(key) ?? [];
          const outside = day.getMonth() !== monthStart.getMonth();
          const isToday = key === todayKey;
          const shown = entries.slice(0, MAX_PER_CELL);
          const overflow = entries.length - shown.length;
          return (
            <div
              key={key}
              role="gridcell"
              className={`cw-cal-cell ${outside ? 'is-outside' : ''} ${isToday ? 'is-today' : ''}`}
            >
              <span className="cw-cal-daynum">{day.getDate()}</span>
              <div className="cw-cal-events">
                {shown.map((e, i) => renderEntry(e, i, pickEntry))}
                {overflow > 0 && (
                  <button
                    type="button"
                    className="cw-cal-more"
                    onClick={(ev) =>
                      setDayPopover({
                        day,
                        anchor: (ev.currentTarget.closest('.cw-cal-cell') as HTMLElement | null) ?? ev.currentTarget,
                      })
                    }
                  >
                    {t('calendar.more', { count: overflow })}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {!occQuery.isLoading && !runsQuery.isLoading && total === 0 && (
        <div className="cw-cal-empty">
          <Icon name="calendar" size={20} />
          <b>{t('calendar.empty_title')}</b>
          <p>{t('calendar.empty_hint')}</p>
        </div>
      )}

      <p className="cw-cal-tznote">{t('calendar.tz_note')}</p>

      {dayPopover && (
        <CalendarDayPopover
          anchor={dayPopover.anchor}
          title={new Intl.DateTimeFormat(i18n.language, { month: 'long', day: 'numeric', weekday: 'long' }).format(dayPopover.day)}
          onClose={() => setDayPopover(null)}
        >
          {(() => {
            const dayEntries = entriesByDay.get(localDayKey(dayPopover.day)) ?? [];
            const shown = dayEntries.slice(0, DAY_POPOVER_CAP);
            const extra = dayEntries.length - shown.length;
            // Anchor the detail to the day cell (survives the popover closing).
            const activate = (entry: CalEntry) => { pickEntry(entry, dayPopover.anchor); setDayPopover(null); };
            return (
              <>
                {shown.map((e, i) => renderEntry(e, i, activate))}
                {extra > 0 && <p className="cw-cal-daypop-more">{t('calendar.more', { count: extra })}</p>}
              </>
            );
          })()}
        </CalendarDayPopover>
      )}
    </div>
  );
}

/** Lists every entry for a day, anchored to its cell. Flips above when there's
 *  not enough room below; scrolls when taller than the available space. */
function CalendarDayPopover({
  anchor,
  title,
  onClose,
  children,
}: {
  anchor: HTMLElement;
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{
    placement: 'below' | 'above';
    top?: number;
    bottom?: number;
    left: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  const recompute = useCallback(() => {
    const r = anchor.getBoundingClientRect();
    const margin = 12;
    const gap = 10;
    const width = Math.max(180, Math.min(280, r.width));
    let left = r.left + r.width / 2 - width / 2;
    left = Math.max(margin, Math.min(left, window.innerWidth - width - margin));
    const spaceBelow = window.innerHeight - r.bottom - gap - margin;
    const spaceAbove = r.top - gap - margin;
    if (spaceBelow >= 200 || spaceBelow >= spaceAbove) {
      setPos({ placement: 'below', top: r.top - gap, left, width, maxHeight: Math.max(160, spaceBelow) });
    } else {
      setPos({ placement: 'above', bottom: window.innerHeight - r.bottom + gap, left, width, maxHeight: Math.max(160, spaceAbove) });
    }
  }, [anchor]);

  useLayoutEffect(() => { recompute(); }, [recompute]);

  useEffect(() => {
    const onScroll = () => recompute();
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', onScroll);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', onScroll);
    };
  }, [recompute]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target) || anchor.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('mousedown', onDown);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onDown);
      window.removeEventListener('keydown', onKey);
    };
  }, [anchor, onClose]);

  if (!pos) return null;
  return createPortal(
    <div
      ref={ref}
      className={`cw-cal-daypop is-${pos.placement}`}
      role="dialog"
      style={{ top: pos.top, bottom: pos.bottom, left: pos.left, width: pos.width, maxHeight: pos.maxHeight }}
    >
      <header className="cw-cal-daypop-head">{title}</header>
      <div className="cw-cal-daypop-list">{children}</div>
    </div>,
    document.body,
  );
}
