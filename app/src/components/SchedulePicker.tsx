// Recurrence builder modeled after Google Calendar's "Custom recurrence"
// dialog. Output is a cron expression, so options that cron can't
// faithfully express (e.g. "every N weeks", "end after N occurrences") are
// intentionally not surfaced. croner is used backend-side, so we lean on
// its `W#N` syntax for "the nth weekday of the month".

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Icon } from '@/components/Icon';
import { Select } from '@/components/Select';

export type SchedulePickerValue = { expr: string; tz: string };

type Unit = 'hour' | 'day' | 'week' | 'month' | 'custom';
type MonthlyMode = 'day' | 'weekday';

interface State {
  unit: Unit;
  interval: number;          // hour only
  hour: number;              // 0-23
  minute: number;            // 0-59
  weekdays: Set<number>;     // 0=Sun..6=Sat (for week)
  monthlyMode: MonthlyMode;  // for month
  dayOfMonth: number;        // for monthlyMode==='day'
  nth: number;               // 1..5 (for weekday mode)
  nthWeekday: number;        // 0..6 (for weekday mode)
  customExpr: string;
  tz: string;
}

function pad(n: number): string { return n.toString().padStart(2, '0'); }

function defaultState(tz: string, expr: string): State {
  return {
    unit: 'day', interval: 1, hour: 9, minute: 0,
    weekdays: new Set([1, 2, 3, 4, 5]),
    monthlyMode: 'day', dayOfMonth: 1, nth: 1, nthWeekday: 1,
    customExpr: expr || '0 9 * * *', tz,
  };
}

export function specToCron(s: State): string {
  switch (s.unit) {
    case 'hour':
      return `0 */${Math.max(1, s.interval)} * * *`;
    case 'day':
      return `${s.minute} ${s.hour} * * *`;
    case 'week': {
      const days = [...s.weekdays].sort((a, b) => a - b);
      const list = days.length ? days.join(',') : '*';
      return `${s.minute} ${s.hour} * * ${list}`;
    }
    case 'month':
      if (s.monthlyMode === 'day') {
        return `${s.minute} ${s.hour} ${s.dayOfMonth} * *`;
      }
      // croner supports `<weekday>#<n>` for "nth weekday of month"
      return `${s.minute} ${s.hour} * * ${s.nthWeekday}#${Math.max(1, Math.min(5, s.nth))}`;
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

  return { ...fallback, unit: 'custom', customExpr: expr };
}

export function summarizeCron(expr: string, t: TFunction<'automation'>): string {
  const s = parseCron(expr, '');
  const time = `${pad(s.hour)}:${pad(s.minute)}`;
  const weekdayArr = t('sched.weekdays', { returnObjects: true }) as string[];
  const nthArr = t('sched.nth', { returnObjects: true }) as string[];
  switch (s.unit) {
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
        return t('sched.summary.monthly_day', { day: s.dayOfMonth, time });
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
  const weekdayArr = t('sched.weekdays', { returnObjects: true }) as string[];
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
  const setTime = (v: string) => {
    const [h, m] = v.split(':').map(Number);
    if (Number.isFinite(h) && Number.isFinite(m)) update({ hour: h, minute: m });
  };

  const previewCron = specToCron(state);
  const summary = summarizeCron(previewCron, t);
  const intervalDisabled = state.unit !== 'hour';

  return (
    <div className="cw-schedule-picker">
      {/* Row 1: Repeat every [N] [unit] */}
      <div className="cw-sched-row">
        <span className="cw-sched-label">{t('sched.repeat_every')}</span>
        <input
          type="number"
          className="cw-sched-num"
          min={1}
          max={24}
          value={state.interval}
          disabled={intervalDisabled}
          aria-disabled={intervalDisabled}
          onChange={(e) => update({ interval: Math.max(1, Math.min(24, parseInt(e.target.value, 10) || 1)) })}
          title={intervalDisabled ? t('sched.interval_hint') : ''}
        />
        <Select<Unit>
          value={state.unit}
          onChange={(unit) => update({ unit })}
          options={[
            { value: 'hour', label: t('sched.opt.hourly') },
            { value: 'day', label: t('sched.opt.daily') },
            { value: 'week', label: t('sched.opt.weekly') },
            { value: 'month', label: t('sched.opt.monthly') },
            { value: 'custom', label: t('sched.opt.custom') },
          ]}
          className="cw-sched-unit"
          triggerClassName="cw-sched-select"
          ariaLabel={t('sched.repeat_every')}
        />
      </div>

      {/* Row 2: Monthly mode dropdown */}
      {state.unit === 'month' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.repeat_mode')}</span>
          <Select
            value={state.monthlyMode === 'day' ? `day-${state.dayOfMonth}` : `wk-${state.nth}-${state.nthWeekday}`}
            onChange={(v) => {
              if (v.startsWith('day-')) {
                update({ monthlyMode: 'day', dayOfMonth: parseInt(v.slice(4), 10) });
              } else if (v.startsWith('wk-')) {
                const [, nthStr, wdStr] = v.split('-');
                update({ monthlyMode: 'weekday', nth: parseInt(nthStr, 10), nthWeekday: parseInt(wdStr, 10) });
              }
            }}
            options={[
              { value: `day-${state.dayOfMonth}`, label: t('sched.monthly_day_opt', { day: state.dayOfMonth }) },
              {
                value: `wk-${state.nth}-${state.nthWeekday}`,
                label: t('sched.monthly_weekday_opt', {
                  nth: nthArr[state.nth - 1] ?? String(state.nth),
                  weekday: weekdayArr[state.nthWeekday],
                }),
              },
            ]}
            className="cw-sched-unit"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.repeat_mode')}
          />
        </div>
      )}

      {/* Row 2b: Monthly fine-tune controls */}
      {state.unit === 'month' && state.monthlyMode === 'day' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.day_of_month')}</span>
          <input
            type="number"
            className="cw-sched-num"
            min={1}
            max={31}
            value={state.dayOfMonth}
            onChange={(e) => update({ dayOfMonth: Math.max(1, Math.min(31, parseInt(e.target.value, 10) || 1)) })}
          />
          {t('sched.day_suffix') !== 'sched.day_suffix' && (
            <span className="cw-sched-suffix">{t('sched.day_suffix')}</span>
          )}
        </div>
      )}
      {state.unit === 'month' && state.monthlyMode === 'weekday' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">{t('sched.detail')}</span>
          <Select<number>
            value={state.nth}
            onChange={(nth) => update({ nth })}
            options={nthArr.map((n, i) => ({ value: i + 1, label: n }))}
            className="cw-sched-unit cw-sched-unit-narrow"
            triggerClassName="cw-sched-select"
            ariaLabel={t('sched.detail')}
          />
          <Select<number>
            value={state.nthWeekday}
            onChange={(nthWeekday) => update({ nthWeekday })}
            options={weekdayArr.map((d, i) => ({ value: i, label: t('sched.weekday_option', { day: d }) }))}
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
          <div className="cw-weekday-row" role="group" aria-label={t('sched.weekday_label')}>
            {weekdayArr.map((label, i) => (
              <button
                key={i}
                type="button"
                className={`cw-weekday-chip ${state.weekdays.has(i) ? 'is-active' : ''}`}
                aria-pressed={state.weekdays.has(i)}
                onClick={() => toggleDay(i)}
              >
                {label}
              </button>
            ))}
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
