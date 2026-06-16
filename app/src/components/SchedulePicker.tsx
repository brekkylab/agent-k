// Recurrence builder modeled after Google Calendar's "Custom recurrence"
// dialog. Output is a cron expression, so options that cron can't
// faithfully express (e.g. "every N weeks", "end after N occurrences") are
// intentionally not surfaced. croner is used backend-side, so we lean on
// its `<weekday>#<n>` syntax (e.g. `4#2` = 2nd Thursday) for "the nth weekday
// of the month", and `<weekday>L` / `L` for the last weekday / last day.

import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Icon } from '@/components/Icon';
import { Select } from '@/components/Select';
import { SegmentedControl } from '@/components/SegmentedControl';

export type SchedulePickerValue = { expr: string; tz: string };

type Unit = 'minute' | 'hour' | 'day' | 'week' | 'month' | 'custom';
type MonthlyMode = 'day' | 'weekday';

// cron's minute field is 0-59, so only divisors of 60 give an evenly-spaced
// "every N minutes" — other steps drift across the hour boundary.
const MINUTE_INTERVALS = [5, 10, 15, 20, 30];

// cron's hour field is 0-23, so only divisors of 24 give an evenly-spaced
// "every N hours" — other steps drift across the midnight boundary.
const HOUR_INTERVALS = [1, 2, 3, 4, 6, 8, 12];

// When the user clears every weekday chip, the schedule falls back to a single
// implied weekday (shown in the UI with a distinct "implied" style).
const WEEKDAY_FALLBACK = 1;  // Monday (0=Sun)

interface State {
  unit: Unit;
  minuteInterval: number;    // for unit==='minute' (divisor of 60)
  interval: number;          // for unit==='hour' (divisor of 24)
  hour: number;              // 0-23
  minute: number;            // 0-59
  weekdays: Set<number>;     // 0=Sun..6=Sat (for week)
  monthlyMode: MonthlyMode;  // for month
  dayOfMonth: number;        // for monthlyMode==='day'
  lastDay: boolean;          // monthlyMode==='day' + last day of month (cron `L`)
  nth: number;               // 1..5 (for weekday mode)
  nthLast: boolean;          // weekday mode + last occurrence in month (cron `<dow>L`)
  nthWeekday: number;        // 0..6 (for weekday mode)
  customExpr: string;
  tz: string;
}

function pad(n: number): string { return n.toString().padStart(2, '0'); }

function defaultState(tz: string, expr: string): State {
  return {
    unit: 'day', minuteInterval: 5, interval: 1,
    hour: 9, minute: 0,
    weekdays: new Set([1, 2, 3, 4, 5]),
    monthlyMode: 'day', dayOfMonth: 1, lastDay: false, nth: 1, nthLast: false, nthWeekday: 1,
    customExpr: expr || '0 9 * * *', tz,
  };
}

export function specToCron(s: State): string {
  switch (s.unit) {
    case 'minute':
      return `*/${Math.max(1, s.minuteInterval)} * * * *`;
    case 'hour':
      return `0 */${Math.max(1, s.interval)} * * *`;
    case 'day':
      return `${s.minute} ${s.hour} * * *`;
    case 'week': {
      // Empty selection is allowed in the UI; fall back to a single weekday
      // (Monday) rather than "every day".
      const days = [...s.weekdays].sort((a, b) => a - b);
      const list = days.length ? days.join(',') : String(WEEKDAY_FALLBACK);
      return `${s.minute} ${s.hour} * * ${list}`;
    }
    case 'month':
      if (s.monthlyMode === 'day') {
        return `${s.minute} ${s.hour} ${s.lastDay ? 'L' : s.dayOfMonth} * *`;
      }
      // croner supports `<weekday>#<n>` for "nth weekday" and `<weekday>L` for
      // "last weekday of month".
      return s.nthLast
        ? `${s.minute} ${s.hour} * * ${s.nthWeekday}L`
        : `${s.minute} ${s.hour} * * ${s.nthWeekday}#${Math.max(1, Math.min(5, s.nth))}`;
    case 'custom':
      return s.customExpr;
  }
}

function parseCron(expr: string, tz: string): State {
  const fallback = defaultState(tz, expr);
  if (!expr) return fallback;
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return { ...fallback, unit: 'custom', customExpr: expr };
  const [m, h, dom, mon, dow] = parts;

  // "*/N * * * *" → every N minutes; "* * * * *" → every minute
  const minutely = /^\*\/(\d+)$/.exec(m);
  if (minutely && h === '*' && dom === '*' && mon === '*' && dow === '*') {
    return { ...fallback, unit: 'minute', minuteInterval: parseInt(minutely[1], 10) };
  }
  if (m === '*' && h === '*' && dom === '*' && mon === '*' && dow === '*') {
    return { ...fallback, unit: 'minute', minuteInterval: 1 };
  }

  // "0 */N * * *" → every N hours; "0 * * * *" → every 1 hour
  const hourly = /^\*\/(\d+)$/.exec(h);
  if (m === '0' && hourly && dom === '*' && mon === '*' && dow === '*') {
    return { ...fallback, unit: 'hour', interval: parseInt(hourly[1], 10) };
  }
  if (m === '0' && h === '*' && dom === '*' && mon === '*' && dow === '*') {
    return { ...fallback, unit: 'hour', interval: 1 };
  }

  const mNum = /^\d+$/.test(m) ? +m : null;
  const hNum = /^\d+$/.test(h) ? +h : null;

  if (mNum !== null && hNum !== null && dom === '*' && mon === '*' && dow === '*') {
    return { ...fallback, unit: 'day', hour: hNum, minute: mNum };
  }

  const nthMatch = /^(\d+)#(\d+)$/.exec(dow);
  if (mNum !== null && hNum !== null && dom === '*' && mon === '*' && nthMatch) {
    return {
      ...fallback, unit: 'month',
      hour: hNum, minute: mNum,
      monthlyMode: 'weekday',
      nthWeekday: parseInt(nthMatch[1], 10),
      nth: parseInt(nthMatch[2], 10),
    };
  }

  // "M H * * <dow>L" → the last <weekday> of the month.
  const lastDow = /^(\d+)L$/.exec(dow);
  if (mNum !== null && hNum !== null && dom === '*' && mon === '*' && lastDow) {
    return {
      ...fallback, unit: 'month',
      hour: hNum, minute: mNum,
      monthlyMode: 'weekday',
      nthWeekday: parseInt(lastDow[1], 10),
      nthLast: true,
    };
  }

  if (mNum !== null && hNum !== null && dom === '*' && mon === '*' && /^[\d,]+$/.test(dow)) {
    const set = new Set(dow.split(',').map(Number).filter((n) => n >= 0 && n <= 6));
    return { ...fallback, unit: 'week', hour: hNum, minute: mNum, weekdays: set };
  }

  if (mNum !== null && hNum !== null && /^\d+$/.test(dom) && mon === '*' && dow === '*') {
    return {
      ...fallback, unit: 'month',
      hour: hNum, minute: mNum,
      monthlyMode: 'day',
      dayOfMonth: +dom,
    };
  }

  // "M H L * *" → the last day of the month.
  if (mNum !== null && hNum !== null && dom === 'L' && mon === '*' && dow === '*') {
    return {
      ...fallback, unit: 'month',
      hour: hNum, minute: mNum,
      monthlyMode: 'day',
      lastDay: true,
    };
  }

  return { ...fallback, unit: 'custom', customExpr: expr };
}

export function summarizeCron(expr: string, t: TFunction<'automation'>): string {
  const s = parseCron(expr, '');
  // On the hour → "9시" / "9 o'clock"; otherwise "09:30".
  const fmtTime = (h: number, m: number) =>
    (m === 0 ? t('sched.summary.oclock', { h }) : `${pad(h)}:${pad(m)}`);
  const time = fmtTime(s.hour, s.minute);
  const weekdayArr = t('sched.weekdays_short', { returnObjects: true }) as string[];
  const nthArr = t('sched.nth', { returnObjects: true }) as string[];
  switch (s.unit) {
    case 'minute':
      return s.minuteInterval === 1
        ? t('sched.summary.every_minute')
        : t('sched.summary.every_n_minutes', { n: s.minuteInterval });
    case 'hour':
      return s.interval === 1
        ? t('sched.summary.every_hour')
        : t('sched.summary.every_n_hours', { n: s.interval });
    case 'day':
      return t('sched.summary.daily', { time });
    case 'week': {
      if (s.weekdays.size === 0) return t('sched.summary.weekly_all', { time });
      const days = [...s.weekdays].sort((a, b) => a - b).map((d) => weekdayArr[d]).join(',');
      return t('sched.summary.weekly', { days, time });
    }
    case 'month':
      if (s.monthlyMode === 'day') {
        return s.lastDay
          ? t('sched.summary.monthly_last', { time })
          : t('sched.summary.monthly_day', { day: s.dayOfMonth, time });
      }
      if (s.nthLast) {
        return t('sched.summary.monthly_weekday_last', {
          weekday: weekdayArr[s.nthWeekday],
          time,
        });
      }
      return t('sched.summary.monthly_weekday', {
        nth: nthArr[s.nth - 1] ?? String(s.nth),
        weekday: weekdayArr[s.nthWeekday],
        time,
      });
    case 'custom':
      return s.customExpr || '(custom)';
  }
}

export function SchedulePicker({
  value, onChange,
}: {
  value: SchedulePickerValue;
  onChange: (next: SchedulePickerValue) => void;
}) {
  const { t } = useTranslation('automation');
  const weekdayArr = t('sched.weekdays_short', { returnObjects: true }) as string[];
  const weekdayLongArr = t('sched.weekdays', { returnObjects: true }) as string[];
  const nthArr = t('sched.nth', { returnObjects: true }) as string[];
  const [state, setState] = useState<State>(() => parseCron(value.expr, value.tz));

  // Notify upstream whenever state changes; skip the initial mount.
  const mountedRef = useRef(false);
  useEffect(() => {
    if (!mountedRef.current) { mountedRef.current = true; return; }
    onChange({ expr: specToCron(state), tz: state.tz });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state]);

  const update = (patch: Partial<State>) => setState((prev) => ({ ...prev, ...patch }));
  const toggleDay = (d: number) => {
    setState((prev) => {
      const next = new Set(prev.weekdays);
      if (next.has(d)) next.delete(d);
      else next.add(d);
      return { ...prev, weekdays: next };
    });
  };

  // Drag-to-select for the weekday chips: pointerdown decides
  // the paint mode from the first chip's state (add if it was off, remove if it
  // was on); every chip the pointer then travels over is forced to that state.
  // elementFromPoint keeps it working for touch, where the pressed chip
  // implicitly captures the pointer.
  const dragMode = useRef<'add' | 'remove' | null>(null);
  useEffect(() => {
    const end = () => { dragMode.current = null; };
    window.addEventListener('pointerup', end);
    window.addEventListener('pointercancel', end);
    return () => {
      window.removeEventListener('pointerup', end);
      window.removeEventListener('pointercancel', end);
    };
  }, []);
  const paintWeekday = (v: number, mode: 'add' | 'remove') => {
    setState((prev) => {
      const has = prev.weekdays.has(v);
      if (mode === 'add' ? has : !has) return prev;
      const next = new Set(prev.weekdays);
      if (mode === 'add') next.add(v);
      else next.delete(v);
      return { ...prev, weekdays: next };
    });
  };
  const readWeekdayEl = (x: number, y: number): HTMLElement | null =>
    document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-weekday]') ?? null;
  const weekdaysDrag = {
    onPointerDown: (e: ReactPointerEvent) => {
      const el = readWeekdayEl(e.clientX, e.clientY);
      if (!el) return;
      const v = Number(el.dataset.weekday);
      // preventDefault stops text selection / a spurious synthetic click, but
      // also suppresses the button's implicit focus — restore it explicitly so
      // keyboard users can keep operating the chip after a click.
      e.preventDefault();
      el.focus();
      const mode: 'add' | 'remove' = state.weekdays.has(v) ? 'remove' : 'add';
      dragMode.current = mode;
      paintWeekday(v, mode);
    },
    onPointerMove: (e: ReactPointerEvent) => {
      if (!dragMode.current) return;
      const el = readWeekdayEl(e.clientX, e.clientY);
      if (el) paintWeekday(Number(el.dataset.weekday), dragMode.current);
    },
  };
  const setTime = (v: string) => {
    const [h, m] = v.split(':').map(Number);
    if (Number.isFinite(h) && Number.isFinite(m)) update({ hour: h, minute: m });
  };

  const previewCron = specToCron(state);
  const summary = summarizeCron(previewCron, t);

  return (
    <div className="cw-schedule-picker">
      {/* Row 1: Repeat every [unit] */}
      <div className="cw-sched-row">
        <span className="cw-sched-label">{t('sched.repeat_every')}</span>
        <SegmentedControl<Unit>
          value={state.unit}
          onChange={(unit) => update({ unit })}
          options={[
            { value: 'minute', label: t('sched.opt.minutely') },
            { value: 'hour', label: t('sched.opt.hourly') },
            { value: 'day', label: t('sched.opt.daily') },
            { value: 'week', label: t('sched.opt.weekly') },
            { value: 'month', label: t('sched.opt.monthly') },
            { value: 'custom', label: t('sched.opt.custom') },
          ]}
          ariaLabel={t('sched.repeat_every')}
        />
      </div>

      {/* every-N-minutes (divisors of 60 only) */}
      {state.unit === 'minute' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.interval_label')}</span>
          <Select<number>
            value={state.minuteInterval}
            onChange={(minuteInterval) => update({ minuteInterval })}
            // Keep a legacy non-divisor value visible until a preset is picked.
            options={(MINUTE_INTERVALS.includes(state.minuteInterval)
              ? MINUTE_INTERVALS
              : [...MINUTE_INTERVALS, state.minuteInterval].sort((a, b) => a - b)
            ).map((n) => ({ value: n, label: String(n) }))}
            className="cw-sched-unit-tiny"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.interval_label')}
          />
          <span className="cw-sched-suffix">{t('sched.minute_suffix')}</span>
        </div>
      )}

      {/* every-N-hours (divisors of 24 only) */}
      {state.unit === 'hour' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.interval_label')}</span>
          <Select<number>
            value={state.interval}
            onChange={(interval) => update({ interval })}
            // Keep a legacy non-divisor value (e.g. parsed `*/5`) visible until
            // the user picks a divisor, instead of rendering a blank trigger.
            options={(HOUR_INTERVALS.includes(state.interval)
              ? HOUR_INTERVALS
              : [...HOUR_INTERVALS, state.interval].sort((a, b) => a - b)
            ).map((n) => ({ value: n, label: String(n) }))}
            className="cw-sched-unit-tiny"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.interval_label')}
          />
          <span className="cw-sched-suffix">{t('sched.interval_suffix')}</span>
        </div>
      )}

      {/* Row 2: Monthly mode dropdown */}
      {state.unit === 'month' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.repeat_mode')}</span>
          <SegmentedControl<MonthlyMode>
            value={state.monthlyMode}
            onChange={(monthlyMode) => update({ monthlyMode })}
            options={[
              { value: 'day', label: t('sched.monthly_mode.day') },
              { value: 'weekday', label: t('sched.monthly_mode.weekday') },
            ]}
            ariaLabel={t('sched.repeat_mode')}
          />
        </div>
      )}

      {/* Row 2b: Monthly fine-tune controls */}
      {state.unit === 'month' && state.monthlyMode === 'day' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.day_of_month')}</span>
          <Select<string>
            value={state.lastDay ? 'L' : String(state.dayOfMonth)}
            onChange={(v) => {
              if (v === 'L') update({ lastDay: true });
              else update({ lastDay: false, dayOfMonth: parseInt(v, 10) });
            }}
            options={[
              ...Array.from({ length: 31 }, (_, i) => ({
                value: String(i + 1),
                label: t('sched.day_label', { day: i + 1 }),
              })),
              { value: 'L', label: t('sched.last_day') },
            ]}
            className="cw-sched-unit-tiny"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.day_of_month')}
          />
        </div>
      )}
      {state.unit === 'month' && state.monthlyMode === 'weekday' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.detail')}</span>
          <Select<string>
            value={state.nthLast ? 'L' : String(state.nth)}
            onChange={(v) => {
              if (v === 'L') update({ nthLast: true });
              else update({ nthLast: false, nth: parseInt(v, 10) });
            }}
            options={[
              ...nthArr.map((n, i) => ({ value: String(i + 1), label: n })),
              { value: 'L', label: t('sched.nth_last') },
            ]}
            className="cw-sched-unit-tiny"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.detail')}
          />
          <Select<number>
            value={state.nthWeekday}
            onChange={(nthWeekday) => update({ nthWeekday })}
            options={weekdayLongArr.map((d, i) => ({ value: i, label: d }))}
            className="cw-sched-unit cw-sched-unit-narrow"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.weekday_label')}
          />
        </div>
      )}

      {/* Row 3: Weekly chips */}
      {state.unit === 'week' && (
        <div className="cw-sched-row cw-sched-row-stack">
          <span className="cw-sched-label">{t('sched.weekday_label')}</span>
          <div
            className="cw-weekday-row"
            role="group"
            aria-label={t('sched.weekday_label')}
            onPointerDown={weekdaysDrag.onPointerDown}
            onPointerMove={weekdaysDrag.onPointerMove}
          >
            {weekdayArr.map((label, i) => {
              const implied = state.weekdays.size === 0 && i === WEEKDAY_FALLBACK;
              // The implied fallback day actually runs, so report it as pressed
              // (with a hint that it's the implicit default) rather than leaving
              // assistive tech to announce "no day selected".
              return (
                <button
                  key={i}
                  type="button"
                  data-weekday={i}
                  className={`cw-weekday-chip ${
                    state.weekdays.has(i) ? 'is-active' : implied ? 'is-implied' : ''
                  }`}
                  aria-pressed={state.weekdays.has(i) || implied}
                  aria-label={implied ? t('sched.weekday_implied', { day: label }) : undefined}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      toggleDay(i);
                    }
                  }}
                >
                  {label}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Row 4: Time-of-day */}
      {(state.unit === 'day' || state.unit === 'week' || state.unit === 'month') && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.run_time')}</span>
          <input
            type="time"
            className="cw-sched-time"
            value={`${pad(state.hour)}:${pad(state.minute)}`}
            onChange={(e) => setTime(e.target.value)}
          />
        </div>
      )}

      {/* Custom raw cron */}
      {state.unit === 'custom' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">Cron</span>
          <input
            type="text"
            className="cw-sched-text"
            value={state.customExpr}
            onChange={(e) => update({ customExpr: e.target.value })}
            placeholder="0 9 * * 1,3,5"
          />
        </div>
      )}

      {/* Timezone */}
      <div className="cw-sched-row">
        <span className="cw-sched-label">{t('sched.timezone')}</span>
        <input
          type="text"
          className="cw-sched-text"
          value={state.tz}
          onChange={(e) => update({ tz: e.target.value })}
          placeholder="Asia/Seoul"
        />
      </div>

      {/* Summary */}
      <div className="cw-schedule-summary">
        <Icon name="calendar" size={14} />
        <span>{summary}</span>
        {state.unit !== 'custom' && (
          <code title={t('sched.cron_preview_title')}>{previewCron}</code>
        )}
      </div>
    </div>
  );
}
