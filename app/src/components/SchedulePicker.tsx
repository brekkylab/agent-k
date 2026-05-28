// Recurrence builder modeled after Google Calendar's "Custom recurrence"
// dialog. Output is a cron expression, so options that cron can't
// faithfully express (e.g. "every N weeks", "end after N occurrences") are
// intentionally not surfaced. croner is used backend-side, so we lean on
// its `W#N` syntax for "the nth weekday of the month".

import { useEffect, useRef, useState } from 'react';
import { Icon } from '@/components/Icon';

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

const WEEKDAY_NAMES = ['일', '월', '화', '수', '목', '금', '토'];
const NTH_NAMES = ['첫째', '둘째', '셋째', '넷째', '다섯째'];

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

export function summarizeCron(expr: string): string {
  const s = parseCron(expr, '');
  const t = `${pad(s.hour)}:${pad(s.minute)}`;
  switch (s.unit) {
    case 'hour':  return s.interval === 1 ? '매시간' : `${s.interval}시간마다`;
    case 'day':   return `매일 ${t}`;
    case 'week': {
      if (s.weekdays.size === 0) return `매주 ${t}`;
      const list = [...s.weekdays].sort((a, b) => a - b).map((d) => WEEKDAY_NAMES[d]).join(',');
      return `매주 ${list}요일 ${t}`;
    }
    case 'month':
      if (s.monthlyMode === 'day') return `매월 ${s.dayOfMonth}일 ${t}`;
      return `매월 ${NTH_NAMES[Math.max(0, s.nth - 1)] ?? `${s.nth}번째`} ${WEEKDAY_NAMES[s.nthWeekday]}요일 ${t}`;
    case 'custom': return s.customExpr || '(custom)';
  }
}

export function SchedulePicker({
  value, onChange,
}: {
  value: SchedulePickerValue;
  onChange: (next: SchedulePickerValue) => void;
}) {
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
  const summary = summarizeCron(previewCron);
  const intervalDisabled = state.unit !== 'hour';

  const unitLabel: Record<Unit, string> = {
    hour: '시간', day: '일', week: '주', month: '월', custom: '사용자 정의',
  };

  return (
    <div className="cw-schedule-picker">
      {/* Row 1: Repeat every [N] [unit] */}
      <div className="cw-sched-row">
        <span className="cw-sched-label">반복 주기</span>
        <input
          type="number"
          className="cw-sched-num"
          min={1}
          max={24}
          value={state.interval}
          disabled={intervalDisabled}
          aria-disabled={intervalDisabled}
          onChange={(e) => update({ interval: Math.max(1, Math.min(24, parseInt(e.target.value, 10) || 1)) })}
          title={intervalDisabled ? 'cron으로는 매시간 단위에서만 N>1을 지원합니다.' : ''}
        />
        <select
          className="cw-sched-unit"
          value={state.unit}
          onChange={(e) => update({ unit: e.target.value as Unit })}
        >
          <option value="hour">{unitLabel.hour}마다 (Hourly)</option>
          <option value="day">{unitLabel.day} (Daily)</option>
          <option value="week">{unitLabel.week} (Weekly)</option>
          <option value="month">{unitLabel.month} (Monthly)</option>
          <option value="custom">{unitLabel.custom} (Custom cron)</option>
        </select>
      </div>

      {/* Row 2: Monthly mode dropdown */}
      {state.unit === 'month' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">반복 방식</span>
          <select
            className="cw-sched-unit"
            value={state.monthlyMode === 'day' ? `day-${state.dayOfMonth}` : `wk-${state.nth}-${state.nthWeekday}`}
            onChange={(e) => {
              const v = e.target.value;
              if (v.startsWith('day-')) {
                update({ monthlyMode: 'day', dayOfMonth: parseInt(v.slice(4), 10) });
              } else if (v.startsWith('wk-')) {
                const [, nthStr, wdStr] = v.split('-');
                update({ monthlyMode: 'weekday', nth: parseInt(nthStr, 10), nthWeekday: parseInt(wdStr, 10) });
              }
            }}
          >
            <option value={`day-${state.dayOfMonth}`}>매월 {state.dayOfMonth}일</option>
            <option value={`wk-${state.nth}-${state.nthWeekday}`}>
              매월 {NTH_NAMES[state.nth - 1] ?? `${state.nth}번째`} {WEEKDAY_NAMES[state.nthWeekday]}요일
            </option>
          </select>
        </div>
      )}

      {/* Row 2b: Monthly fine-tune controls */}
      {state.unit === 'month' && state.monthlyMode === 'day' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">며칠</span>
          <input
            type="number"
            className="cw-sched-num"
            min={1}
            max={31}
            value={state.dayOfMonth}
            onChange={(e) => update({ dayOfMonth: Math.max(1, Math.min(31, parseInt(e.target.value, 10) || 1)) })}
          />
          <span className="cw-sched-suffix">일</span>
        </div>
      )}
      {state.unit === 'month' && state.monthlyMode === 'weekday' && (
        <div className="cw-sched-row">
          <span className="cw-sched-label">상세</span>
          <select
            className="cw-sched-unit cw-sched-unit-narrow"
            value={state.nth}
            onChange={(e) => update({ nth: parseInt(e.target.value, 10) })}
          >
            {NTH_NAMES.map((n, i) => <option key={i} value={i + 1}>{n}</option>)}
          </select>
          <select
            className="cw-sched-unit cw-sched-unit-narrow"
            value={state.nthWeekday}
            onChange={(e) => update({ nthWeekday: parseInt(e.target.value, 10) })}
          >
            {WEEKDAY_NAMES.map((d, i) => <option key={i} value={i}>{d}요일</option>)}
          </select>
        </div>
      )}

      {/* Row 3: Weekly chips */}
      {state.unit === 'week' && (
        <div className="cw-sched-row cw-sched-row-stack">
          <span className="cw-sched-label">요일</span>
          <div className="cw-weekday-row" role="group" aria-label="요일">
            {WEEKDAY_NAMES.map((label, i) => (
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
          <span className="cw-sched-label">실행 시각</span>
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
        <span className="cw-sched-label">Timezone</span>
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
          <code title="생성될 cron 표현식">{previewCron}</code>
        )}
      </div>
    </div>
  );
}
