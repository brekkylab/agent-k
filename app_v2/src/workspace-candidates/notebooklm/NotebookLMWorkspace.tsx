// Workspace candidate B — NotebookLM archetype.
// 3-pane: Sources (grounding scope via checkboxes) | Chat (cited answers) | Studio (outputs).
// Flow-choreography mockup: all state is in-memory; timers fake analysis/generation.

import { useEffect, useRef, useState, type CSSProperties } from 'react';
import {
  DISCOVER_SUGGESTIONS,
  INITIAL_SOURCES,
  KIND_FILTER_LABELS,
  NB_TYPE_ICON,
  STUDIO_PALETTE,
  answerFor,
  studioBody,
  type NbAnswerSeg,
  type NbSource,
  type NbSourceGroup,
  type NbSourceKind,
  type NbSourceType,
  type NbStudioKind,
} from './data';
import './notebooklm.css';

interface ChatMsg {
  id: number;
  role: 'user' | 'ai';
  text?: string;        // user
  segs?: NbAnswerSeg[]; // ai
  shown?: number;       // ai: progressive segment reveal
  // Snapshot of which sources were checked at send time — for citedBy tracking.
  checkedAtSend?: string[];
}

interface StudioOut {
  id: number;
  kind: NbStudioKind | 'note';
  icon: string;
  title: string;
  status: 'gen' | 'done';
  body: string;
  meta: string;
}

// Conflict popover state: which segment's conflict pill is open.
interface ConflictPopover {
  msgId: number;
  segIdx: number;
  anchorRect: DOMRect;
}

let nextId = 1;
const uid = () => nextId++;

// All distinct groups in display order.
const SOURCE_GROUPS: NbSourceGroup[] = ['재무', '계약', '제품', '받은 재료'];

// Filter chips in order.
const KIND_FILTERS = ['all', 'drive', 'gmail', 'slack', 'jira', 'upload', 'session'] as const;
type KindFilter = (typeof KIND_FILTERS)[number];

export function NotebookLMWorkspace() {
  const [sources, setSources] = useState<NbSource[]>(INITIAL_SOURCES);

  // "받은 재료" sources start unchecked.
  const [checked, setChecked] = useState<Set<string>>(
    () => new Set(INITIAL_SOURCES.filter((s) => !s.unconfirmed).map((s) => s.id)),
  );

  // Track which sources were approved from "받은 재료" so we only toast once.
  const [approvedDeposits, setApprovedDeposits] = useState<Set<string>>(() => new Set());

  const [guideId, setGuideId] = useState<string | null>(null);
  const [guideHot, setGuideHot] = useState(false);
  const [leftCollapsed, setLeftCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);

  // Left-pane kind filter.
  const [kindFilter, setKindFilter] = useState<KindFilter>('all');

  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);

  const [outputs, setOutputs] = useState<StudioOut[]>([]);
  const [preview, setPreview] = useState<StudioOut | null>(null);

  const [addOpen, setAddOpen] = useState(false);
  const [addTab, setAddTab] = useState<'file' | 'link' | 'text'>('file');
  const [linkVal, setLinkVal] = useState('');
  const [textVal, setTextVal] = useState('');
  const [uploadPct, setUploadPct] = useState<number | null>(null);

  const [discoverOpen, setDiscoverOpen] = useState(false);
  const [pickedSuggestions, setPickedSuggestions] = useState<Set<string>>(
    () => new Set(DISCOVER_SUGGESTIONS.map((s) => s.id)),
  );

  const [toast, setToast] = useState<string | null>(null);

  // Conflict popover: null = closed.
  const [conflictPopover, setConflictPopover] = useState<ConflictPopover | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const timers = useRef<number[]>([]);
  const later = (fn: () => void, ms: number) => {
    timers.current.push(window.setTimeout(fn, ms));
  };
  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: 'smooth' });
  }, [msgs, busy]);

  // Close conflict popover on outside click.
  useEffect(() => {
    if (!conflictPopover) return;
    const handler = () => setConflictPopover(null);
    window.addEventListener('click', handler);
    return () => window.removeEventListener('click', handler);
  }, [conflictPopover]);

  const checkedSources = sources.filter((s) => checked.has(s.id) && !s.analyzing);
  const guide = sources.find((s) => s.id === guideId) ?? null;

  // Count how many times each source was cited this session.
  const sessionCiteCount = new Map<string, number>();
  for (const m of msgs) {
    if (m.role !== 'ai' || !m.segs) continue;
    for (const seg of m.segs) {
      for (const c of seg.cites ?? []) {
        sessionCiteCount.set(c, (sessionCiteCount.get(c) ?? 0) + 1);
      }
    }
  }
  // Total citations in this session for the pane header.
  const totalSessionCites = Array.from(sessionCiteCount.values()).reduce((a, b) => a + b, 0);

  // Stale sources inside the current checked scope.
  const staleScopedSources = checkedSources.filter((s) => s.fresh?.state === 'stale');

  function showToast(text: string) {
    setToast(text);
    later(() => setToast(null), 2400);
  }

  // ---------- sources ----------

  function toggleCheck(id: string) {
    // Decide the new checked state outside the updater (StrictMode purity rule).
    const willCheck = !checked.has(id);
    const src = sources.find((s) => s.id === id);

    if (willCheck && src?.unconfirmed && !approvedDeposits.has(id)) {
      setApprovedDeposits((prev) => new Set(prev).add(id));
      showToast('받은 재료를 대화 범위로 승인했어요');
    }

    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  /** Toggle all sources in a group (group-level checkbox). */
  function toggleGroup(group: NbSourceGroup) {
    const groupSrcs = sources.filter((s) => s.group === group && !s.analyzing);
    const allChecked = groupSrcs.every((s) => checked.has(s.id));
    // Detect any unconfirmed deposits being approved.
    const newlyApproved = allChecked
      ? []
      : groupSrcs.filter((s) => s.unconfirmed && !approvedDeposits.has(s.id));
    if (newlyApproved.length > 0) {
      setApprovedDeposits((prev) => {
        const next = new Set(prev);
        newlyApproved.forEach((s) => next.add(s.id));
        return next;
      });
      showToast('받은 재료를 대화 범위로 승인했어요');
    }
    setChecked((prev) => {
      const next = new Set(prev);
      if (allChecked) {
        groupSrcs.forEach((s) => next.delete(s.id));
      } else {
        groupSrcs.forEach((s) => next.add(s.id));
      }
      return next;
    });
  }

  function toggleAll() {
    const analyzed = sources.filter((s) => !s.analyzing);
    setChecked((prev) =>
      prev.size >= analyzed.length ? new Set() : new Set(analyzed.map((s) => s.id)),
    );
  }

  /** Add a source in the "analyzing" state, then fake summary/topic extraction. */
  function addSource(partial: {
    type: NbSourceType;
    title: string;
    origin: string;
    group?: NbSourceGroup;
    sourceKind?: NbSourceKind;
    summary?: string;
    topics?: string[];
    quote?: string;
  }) {
    const id = `nb-added-${uid()}`;
    setSources((prev) => [
      ...prev,
      {
        id,
        type: partial.type,
        sourceKind: partial.sourceKind ?? 'upload',
        group: partial.group ?? '제품',
        title: partial.title,
        origin: partial.origin,
        modifiedAt: '방금',
        summary: partial.summary ?? '',
        topics: partial.topics ?? [],
        quote: partial.quote ?? '',
        analyzing: true,
      },
    ]);
    later(() => {
      setSources((prev) =>
        prev.map((s) =>
          s.id === id
            ? {
                ...s,
                analyzing: false,
                summary:
                  s.summary ||
                  `방금 추가된 자료의 자동 요약입니다. ${s.title}의 핵심 내용을 추출해 대화 근거로 쓸 수 있게 준비했어요.`,
                topics: s.topics.length > 0 ? s.topics : ['핵심 요지', '관련 일정', '후속 액션'],
                quote:
                  s.quote ||
                  '(대표 구절) 이 자료에서 인용 점프 시 강조될 대목이 여기에 표시됩니다.',
              }
            : s,
        ),
      );
      setChecked((prev) => new Set(prev).add(id));
      showToast('요약과 핵심 토픽을 추출했어요 — 대화 범위에 포함됩니다');
    }, 1700);
  }

  function openGuide(id: string, hot = false) {
    if (leftCollapsed) setLeftCollapsed(false);
    setGuideId(id);
    setGuideHot(hot);
    if (hot) later(() => setGuideHot(false), 1600);
  }

  // ---------- chat ----------

  function ask(question: string) {
    const q = question.trim();
    if (!q || busy || checkedSources.length === 0) return;
    const checkedIds = checkedSources.map((s) => s.id);
    setInput('');
    setMsgs((prev) => [...prev, { id: uid(), role: 'user', text: q, checkedAtSend: checkedIds }]);
    setBusy(true);
    later(() => {
      const segs = answerFor(q, checkedSources);
      const msgId = uid();
      setMsgs((prev) => [...prev, { id: msgId, role: 'ai', segs, shown: 0, checkedAtSend: checkedIds }]);
      // Progressive reveal, one segment at a time.
      segs.forEach((_, i) => {
        later(() => {
          setMsgs((prev) =>
            prev.map((m) => (m.id === msgId ? { ...m, shown: i + 1 } : m)),
          );
          if (i === segs.length - 1) {
            setBusy(false);
            // Deposit: append answer snippet to citedBy for each cited source.
            const firstUserTurn = q.slice(0, 40);
            const allCited = new Set(segs.flatMap((s) => s.cites ?? []));
            setSources((prev) =>
              prev.map((s) =>
                allCited.has(s.id)
                  ? { ...s, citedBy: [...(s.citedBy ?? []), firstUserTurn + '…'] }
                  : s,
              ),
            );
          }
        }, 380 * (i + 1));
      });
    }, 750);
  }

  function askTopic(src: NbSource, topic: string) {
    ask(`${src.title.replace(/\.[a-z]+$/i, '')}에서 "${topic}" 부분을 자세히 설명해줘`);
  }

  function saveNote(m: ChatMsg) {
    const text = (m.segs ?? []).map((s) => s.text).join('');
    setOutputs((prev) => [
      {
        id: uid(),
        kind: 'note',
        icon: '📌',
        title: `노트 — ${text.slice(0, 24)}…`,
        status: 'done',
        body: text,
        meta: '채팅 응답에서 저장 · 방금',
      },
      ...prev,
    ]);
    if (rightCollapsed) setRightCollapsed(false);
    showToast('스튜디오에 노트로 저장했어요');
  }

  function promoteNote(out: StudioOut) {
    addSource({
      type: 'text',
      title: out.title.replace(/^노트 — /, '노트: '),
      origin: '스튜디오 노트',
      summary: out.body,
      topics: ['저장한 인사이트'],
      quote: out.body.slice(0, 120),
    });
    setPreview(null);
    showToast('노트를 소스로 승격했어요 — 출력이 다시 입력이 됩니다');
  }

  // ---------- studio ----------

  function generate(kind: NbStudioKind) {
    if (checkedSources.length === 0) return;
    const pal = STUDIO_PALETTE.find((p) => p.kind === kind)!;
    const id = uid();
    setOutputs((prev) => [
      { id, kind, icon: pal.icon, title: pal.label, status: 'gen', body: '', meta: '생성 중…' },
      ...prev,
    ]);
    later(() => {
      setOutputs((prev) =>
        prev.map((o) =>
          o.id === id
            ? {
                ...o,
                status: 'done',
                body: studioBody(kind, checkedSources),
                meta: `${checkedSources.length}개 소스 기반 · 방금`,
              }
            : o,
        ),
      );
    }, 2100);
  }

  // ---------- add dialog ----------

  function startUpload() {
    if (uploadPct !== null) return;
    setUploadPct(0);
    const tick = () => {
      setUploadPct((p) => {
        const next = (p ?? 0) + 16;
        if (next >= 100) {
          later(() => {
            setUploadPct(null);
            setAddOpen(false);
            addSource({
              type: 'slides',
              title: '제품 로드맵 v2.pptx',
              origin: '업로드',
              topics: ['하반기 마일스톤', '기능 우선순위'],
            });
          }, 250);
          return 100;
        }
        later(tick, 180);
        return next;
      });
    };
    later(tick, 180);
  }

  function addLinks() {
    const urls = linkVal.split(/[\n\s]+/).map((u) => u.trim()).filter(Boolean);
    if (urls.length === 0) return;
    urls.forEach((u) => {
      let host = u;
      try {
        host = new URL(u.startsWith('http') ? u : `https://${u}`).hostname;
      } catch {
        /* keep raw string as title */
      }
      addSource({ type: 'link', title: host, origin: '웹 링크', sourceKind: 'upload' });
    });
    setLinkVal('');
    setAddOpen(false);
    if (urls.length > 1) showToast(`URL ${urls.length}개를 각각 개별 소스로 분해했어요`);
  }

  function addText() {
    if (!textVal.trim()) return;
    addSource({
      type: 'text',
      title: `붙여넣은 텍스트 — ${textVal.trim().slice(0, 18)}…`,
      origin: '직접 입력',
      summary: textVal.trim(),
    });
    setTextVal('');
    setAddOpen(false);
  }

  function addFromDiscover() {
    DISCOVER_SUGGESTIONS.filter((s) => pickedSuggestions.has(s.id)).forEach((s) =>
      addSource({ type: s.type, title: s.title, origin: '웹 · 탐색', summary: s.reason }),
    );
    setDiscoverOpen(false);
  }

  // ---------- render helpers ----------

  /** Stable per-message citation numbering (order of first appearance). */
  function citeIndex(segs: NbAnswerSeg[]): Map<string, number> {
    const map = new Map<string, number>();
    segs.forEach((seg) =>
      seg.cites?.forEach((c) => {
        if (!map.has(c)) map.set(c, map.size + 1);
      }),
    );
    return map;
  }

  const suggestedQuestions = checkedSources
    .flatMap((s) => s.topics.map((t) => ({ src: s, topic: t })))
    .slice(0, 4);

  const allChecked = checked.size >= sources.filter((s) => !s.analyzing).length;

  // Filter sources by the active kind chip, then group them.
  function visibleSources(group: NbSourceGroup): NbSource[] {
    return sources.filter(
      (s) => s.group === group && (kindFilter === 'all' || s.sourceKind === kindFilter),
    );
  }

  function groupAllChecked(group: NbSourceGroup): boolean {
    const vs = visibleSources(group).filter((s) => !s.analyzing);
    return vs.length > 0 && vs.every((s) => checked.has(s.id));
  }

  function groupSomeChecked(group: NbSourceGroup): boolean {
    const vs = visibleSources(group).filter((s) => !s.analyzing);
    return vs.some((s) => checked.has(s.id)) && !groupAllChecked(group);
  }

  /** Render the Source Guide extended view. */
  function renderGuide() {
    if (!guide) return null;
    const citeCount = sessionCiteCount.get(guide.id) ?? 0;
    const relatedSources = (guide.related ?? [])
      .map((id) => sources.find((s) => s.id === id))
      .filter(Boolean) as NbSource[];

    return (
      <div className="nbw-pane-body">
        <button type="button" className="nbw-guide-back" onClick={() => setGuideId(null)}>
          ← 소스 목록
        </button>
        <div className="nbw-guide-title">
          <span>{NB_TYPE_ICON[guide.type]}</span>
          <span>{guide.title}</span>
        </div>
        <div className="nbw-guide-origin">
          {guide.origin} · {guide.modifiedAt} · 대화 범위 {checked.has(guide.id) ? '포함' : '제외'}
        </div>

        {/* Freshness badge */}
        {guide.fresh && (
          <div className={`nbw-fresh-badge ${guide.fresh.state}`}>
            {guide.fresh.state === 'ok' ? '✅' : '❔'} {guide.fresh.label}
          </div>
        )}

        {/* Extracted facts table */}
        {guide.facts && guide.facts.length > 0 && (
          <>
            <div className="nbw-guide-sec">추출된 사실</div>
            <table className="nbw-facts-table">
              <tbody>
                {guide.facts.map((f) => (
                  <tr key={f.label}>
                    <td className="label">{f.label}</td>
                    <td className="value">{f.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}

        {/* Auto summary */}
        <div className="nbw-guide-sec">자동 요약</div>
        <p className="nbw-guide-summary">{guide.summary}</p>

        {/* Full body with quote highlight */}
        {guide.body && (
          <>
            <div className="nbw-guide-sec">본문</div>
            <div className={`nbw-guide-body${guideHot ? ' is-hot-body' : ''}`}>
              {guide.body.split(guide.quote).map((part, i, arr) => (
                <span key={i}>
                  {part}
                  {i < arr.length - 1 && (
                    <mark className={guideHot ? 'is-hot' : ''}>{guide.quote}</mark>
                  )}
                </span>
              ))}
            </div>
          </>
        )}

        {/* Topics */}
        <div className="nbw-guide-sec">핵심 토픽 — 누르면 질문이 됩니다</div>
        <div className="nbw-topics">
          {guide.topics.map((t) => (
            <button key={t} type="button" className="nbw-topic" onClick={() => askTopic(guide, t)}>
              {t}
            </button>
          ))}
        </div>

        {/* Quote (citation jump target) */}
        <div className="nbw-guide-sec">대표 구절 (인용 점프 위치)</div>
        <p className={`nbw-quote${guideHot ? ' is-hot' : ''}`}>
          <mark>{guide.quote}</mark>
        </p>

        {/* Related documents */}
        {relatedSources.length > 0 && (
          <>
            <div className="nbw-guide-sec">관련 문서</div>
            <div className="nbw-related-list">
              {relatedSources.map((r) => (
                <button
                  key={r.id}
                  type="button"
                  className="nbw-related-item"
                  onClick={() => openGuide(r.id)}
                >
                  <span>{NB_TYPE_ICON[r.type]}</span>
                  <span>{r.title}</span>
                </button>
              ))}
            </div>
          </>
        )}

        {/* citedBy: past answer snippets + session count */}
        {((guide.citedBy && guide.citedBy.length > 0) || citeCount > 0) && (
          <>
            <div className="nbw-guide-sec">이 문서를 인용한 답변</div>
            {guide.citedBy && guide.citedBy.length > 0 && (
              <ul className="nbw-cited-by-list">
                {guide.citedBy.map((snip, i) => (
                  <li key={i} className="nbw-cited-by-item">"{snip}"</li>
                ))}
              </ul>
            )}
            {citeCount > 0 && (
              <div className="nbw-session-cite-note">이번 세션 인용 {citeCount}회</div>
            )}
          </>
        )}
      </div>
    );
  }

  return (
    <div className="nbw-root" onClick={() => conflictPopover && setConflictPopover(null)}>
      {/* ============ Sources pane ============ */}
      <aside className={`nbw-pane${leftCollapsed ? ' is-collapsed' : ''}`}>
        <div className="nbw-pane-head">
          <div className="nbw-pane-title">
            {!leftCollapsed && (
              <span>
                소스 <span className="nbw-count">{sources.length}</span>
                {totalSessionCites > 0 && (
                  <span className="nbw-session-cite-badge">이번 세션 인용 {totalSessionCites}회</span>
                )}
              </span>
            )}
          </div>
          <button
            type="button"
            className="nbw-collapse"
            aria-label={leftCollapsed ? '소스 패널 펼치기' : '소스 패널 접기'}
            onClick={() => setLeftCollapsed((v) => !v)}
          >
            {leftCollapsed ? '»' : '«'}
          </button>
        </div>

        {!guide ? (
          <>
            <div className="nbw-pane-actions">
              <button type="button" className="nbw-btn primary" onClick={() => setAddOpen(true)}>
                ＋ 추가
              </button>
              <button type="button" className="nbw-btn" onClick={() => setDiscoverOpen(true)}>
                ✨ 탐색
              </button>
            </div>

            {/* Kind filter chips */}
            <div className="nbw-kind-filters">
              {KIND_FILTERS.map((k) => (
                <button
                  key={k}
                  type="button"
                  className={`nbw-kind-chip${kindFilter === k ? ' is-active' : ''}`}
                  onClick={() => setKindFilter(k)}
                >
                  {KIND_FILTER_LABELS[k as NbSourceKind | 'all']}
                </button>
              ))}
            </div>

            <div className="nbw-pane-body">
              <label className="nbw-selectall">
                <input type="checkbox" checked={allChecked} onChange={toggleAll} />
                모든 소스 선택 · {checked.size}개가 대화 범위
              </label>

              {/* Grouped source rows */}
              {SOURCE_GROUPS.map((group) => {
                const groupSrcs = visibleSources(group);
                if (groupSrcs.length === 0) return null;
                const gChecked = groupAllChecked(group);
                const gIndeterminate = groupSomeChecked(group);
                return (
                  <div key={group} className="nbw-group">
                    <div className="nbw-group-header">
                      <input
                        type="checkbox"
                        checked={gChecked}
                        ref={(el) => {
                          if (el) el.indeterminate = gIndeterminate;
                        }}
                        onChange={() => toggleGroup(group)}
                        aria-label={`${group} 그룹 전체 선택`}
                      />
                      <span className="nbw-group-label">{group}</span>
                    </div>
                    {groupSrcs.map((s) => (
                      <div
                        key={s.id}
                        className={`nbw-src-row${guideId === s.id ? ' is-open' : ''}${s.unconfirmed && !approvedDeposits.has(s.id) ? ' is-unconfirmed' : ''}`}
                        onClick={() => !s.analyzing && openGuide(s.id)}
                      >
                        <input
                          type="checkbox"
                          checked={checked.has(s.id)}
                          disabled={s.analyzing}
                          onClick={(e) => e.stopPropagation()}
                          onChange={() => toggleCheck(s.id)}
                          aria-label={`${s.title} 대화 범위에 포함`}
                        />
                        <span className="nbw-src-ic">{NB_TYPE_ICON[s.type]}</span>
                        <span className="nbw-src-txt">
                          <span className="nbw-src-title">
                            {s.title}
                            {s.unconfirmed && !approvedDeposits.has(s.id) && (
                              <span className="nbw-unconfirmed-tag">미확인</span>
                            )}
                          </span>
                          {s.analyzing ? (
                            <span className="nbw-analyzing">
                              <span className="nbw-spin" /> 분석 중 — 요약 추출
                            </span>
                          ) : (
                            <span className="nbw-src-meta">
                              {s.origin} · {s.modifiedAt}
                              {s.fresh?.state === 'stale' && (
                                <span className="nbw-stale-dot" title={s.fresh.label}>●</span>
                              )}
                            </span>
                          )}
                        </span>
                      </div>
                    ))}
                  </div>
                );
              })}
            </div>
          </>
        ) : (
          /* -------- Source Guide (inline pane switch, not a modal) -------- */
          renderGuide()
        )}
      </aside>

      {/* ============ Chat (center) ============ */}
      <section className="nbw-chat">
        <div className="nbw-chat-head">
          <h2>워크스페이스 대화</h2>
          <span className="nbw-scope">{checkedSources.length}개 소스 범위</span>
        </div>

        <div className="nbw-chat-scroll" ref={scrollRef}>
          <div className="nbw-chat-inner">
            {msgs.length === 0 && (
              <div className="nbw-overview">
                <h3>소스 개요</h3>
                <p>
                  {checkedSources.length}개 소스가 대화 범위에 있어요 — 재무(2분기 실적·인프라 견적),
                  계약(B사 초안), 제품(결제 성능·API 전환) 세 갈래입니다. 아래 제안을 누르거나 직접
                  질문해 보세요. 답변의 인용 번호를 누르면 원문 대목으로 이동합니다.
                </p>
                <div className="nbw-topics">
                  {suggestedQuestions.map(({ src, topic }) => (
                    <button
                      key={`${src.id}-${topic}`}
                      type="button"
                      className="nbw-topic"
                      onClick={() => askTopic(src, topic)}
                    >
                      {topic}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {msgs.map((m) =>
              m.role === 'user' ? (
                <div key={m.id} className="nbw-msg user">
                  <div className="nbw-bubble">{m.text}</div>
                </div>
              ) : (
                <div key={m.id} className="nbw-msg ai">
                  <span className="nbw-ai-mark">🤖</span>
                  <div className="nbw-bubble">
                    {(() => {
                      const idx = citeIndex(m.segs ?? []);
                      return (m.segs ?? []).slice(0, m.shown ?? 0).map((seg, segI) => {
                        const hasConflict = !!seg.conflictWith;
                        return (
                          <span key={segI}>
                            {seg.text}
                            {seg.cites?.map((c) => {
                              const src = sources.find((s) => s.id === c);
                              return (
                                <button
                                  key={c}
                                  type="button"
                                  className="nbw-cite"
                                  title={src ? `${src.title} — 원문으로 이동` : ''}
                                  onClick={() => openGuide(c, true)}
                                >
                                  {idx.get(c)}
                                </button>
                              );
                            })}
                            {/* Conflict pill — only on segments that have conflictWith */}
                            {hasConflict && (
                              <button
                                type="button"
                                className="nbw-conflict-pill"
                                title="인용 출처 간 충돌"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
                                  setConflictPopover((prev) =>
                                    prev?.msgId === m.id && prev?.segIdx === segI
                                      ? null
                                      : { msgId: m.id, segIdx: segI, anchorRect: rect },
                                  );
                                }}
                              >
                                ⚠
                              </button>
                            )}
                          </span>
                        );
                      });
                    })()}
                    {(m.shown ?? 0) >= (m.segs?.length ?? 0) && (
                      <div className="nbw-msg-tools">
                        <button type="button" className="nbw-mini" onClick={() => saveNote(m)}>
                          📌 노트로 저장
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              ),
            )}

            {busy && msgs[msgs.length - 1]?.role === 'user' && (
              <div className="nbw-msg ai">
                <span className="nbw-ai-mark">🤖</span>
                <div className="nbw-bubble">
                  <span className="nbw-typing"><i /><i /><i /></span>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Conflict popover (rendered in chat section, position fixed to anchor) */}
        {conflictPopover && (() => {
          const msg = msgs.find((m) => m.id === conflictPopover.msgId);
          const seg = msg?.segs?.[conflictPopover.segIdx];
          const cf = seg?.conflictWith;
          if (!cf) return null;
          const srcA = sources.find((s) => s.id === cf.srcA);
          const srcB = sources.find((s) => s.id === cf.srcB);
          const { anchorRect } = conflictPopover;
          return (
            <div
              className="nbw-conflict-popover"
              style={{
                position: 'fixed',
                top: anchorRect.bottom + 6,
                left: Math.max(8, anchorRect.left - 160),
              }}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="nbw-conflict-popover-title">⚠ 인용 출처 간 충돌</div>
              <div className="nbw-conflict-row">
                <span className="src-label">{srcA?.title ?? cf.srcA}</span>
                <span className="src-val">"{cf.valA}"</span>
              </div>
              <div className="nbw-conflict-vs">vs</div>
              <div className="nbw-conflict-row">
                <span className="src-label">{srcB?.title ?? cf.srcB}</span>
                <span className="src-val">"{cf.valB}"</span>
              </div>
              <div className="nbw-conflict-hint">{cf.hint}</div>
            </div>
          );
        })()}

        <div className="nbw-composer">
          {/* Stale scope warning — shown above composer when stale sources are in scope */}
          {staleScopedSources.length > 0 && (
            <div className="nbw-stale-warn">
              {staleScopedSources.map((s) => (
                <span key={s.id}>
                  ⚠ 대화 범위에 오래된 재료{' '}
                  <button
                    type="button"
                    className="nbw-stale-link"
                    onClick={() => openGuide(s.id)}
                  >
                    {s.title.length > 22 ? s.title.slice(0, 22) + '…' : s.title}
                  </button>
                  {' '}— {s.fresh?.label}
                </span>
              ))}
            </div>
          )}

          <div className="nbw-composer-box">
            <textarea
              rows={1}
              placeholder={
                checkedSources.length === 0
                  ? '소스를 먼저 선택하세요'
                  : `${checkedSources.length}개 소스에 대해 질문하기…`
              }
              value={input}
              disabled={checkedSources.length === 0}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  ask(input);
                }
              }}
            />
            <button
              type="button"
              className="nbw-send"
              disabled={!input.trim() || busy || checkedSources.length === 0}
              onClick={() => ask(input)}
            >
              전송
            </button>
          </div>
          <p className={`nbw-composer-hint${checkedSources.length === 0 ? ' warn' : ''}`}>
            {checkedSources.length === 0
              ? '⚠ 대화 범위가 비어 있어요 — 좌측에서 소스를 1개 이상 선택하세요.'
              : '답변은 선택된 소스에만 근거합니다 · 인용 번호를 누르면 원문 대목이 열립니다'}
          </p>
        </div>
      </section>

      {/* ============ Studio pane ============ */}
      <aside className={`nbw-pane nbw-studio${rightCollapsed ? ' is-collapsed' : ''}`}>
        <div className="nbw-pane-head">
          <button
            type="button"
            className="nbw-collapse"
            aria-label={rightCollapsed ? '스튜디오 펼치기' : '스튜디오 접기'}
            onClick={() => setRightCollapsed((v) => !v)}
          >
            {rightCollapsed ? '«' : '»'}
          </button>
          <div className="nbw-pane-title">
            {!rightCollapsed && (
              <span>
                스튜디오 <span className="nbw-count">{outputs.length}</span>
              </span>
            )}
          </div>
        </div>
        <div className="nbw-pane-body">
          <div className="nbw-palette">
            {STUDIO_PALETTE.map((p) => (
              <button
                key={p.kind}
                type="button"
                disabled={checkedSources.length === 0}
                onClick={() => generate(p.kind)}
              >
                <span className="ic">{p.icon}</span>
                {p.label}
              </button>
            ))}
          </div>

          {outputs.length === 0 ? (
            <p className="nbw-empty">
              선택한 소스로 산출물을 만들어요.
              <br />
              생성물은 여기 쌓이고, 노트는 다시 소스로 승격할 수 있어요.
            </p>
          ) : (
            outputs.map((o) => (
              <div
                key={o.id}
                className={`nbw-out${o.status === 'gen' ? ' is-gen' : ''}`}
                onClick={() => o.status === 'done' && setPreview(o)}
              >
                <div className="nbw-out-title">
                  <span>{o.icon}</span>
                  <span>{o.title}</span>
                </div>
                <div className="nbw-out-meta">{o.meta}</div>
                {o.status === 'gen' && <div className="nbw-shimmer" />}
              </div>
            ))
          )}
        </div>
      </aside>

      {/* ============ Add dialog ============ */}
      {addOpen && (
        <div className="nbw-overlay" onClick={() => uploadPct === null && setAddOpen(false)}>
          <div className="nbw-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>소스 추가</h3>
            <p className="sub">추가하는 즉시 자동으로 요약과 핵심 토픽을 추출합니다.</p>
            <div className="nbw-tabs">
              {(
                [
                  ['file', '파일 업로드'],
                  ['link', '링크'],
                  ['text', '텍스트 붙여넣기'],
                ] as const
              ).map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  className={`nbw-tab${addTab === k ? ' is-active' : ''}`}
                  onClick={() => setAddTab(k)}
                >
                  {label}
                </button>
              ))}
            </div>

            {addTab === 'file' && (
              <>
                <div className="nbw-drop" onClick={startUpload} role="button" tabIndex={0}>
                  ☁️ 클릭하거나 파일을 끌어다 놓으세요
                  <br />
                  <span style={{ fontSize: 11, color: 'var(--cw-ink-faint)' }}>
                    PDF · DOCX · PPTX · XLSX · MD (데모: 샘플 파일이 업로드됩니다)
                  </span>
                </div>
                {uploadPct !== null && (
                  <div className="nbw-progress">
                    <div style={{ '--pct': uploadPct / 100 } as CSSProperties} />
                  </div>
                )}
              </>
            )}

            {addTab === 'link' && (
              <>
                <textarea
                  className="nbw-field"
                  rows={3}
                  placeholder={'https://…\n여러 URL은 줄바꿈으로 구분하면 각각 개별 소스가 됩니다'}
                  value={linkVal}
                  onChange={(e) => setLinkVal(e.target.value)}
                />
                <div className="nbw-dialog-actions">
                  <button type="button" className="nbw-btn" onClick={() => setAddOpen(false)}>
                    취소
                  </button>
                  <button type="button" className="nbw-btn primary" onClick={addLinks}>
                    추가
                  </button>
                </div>
              </>
            )}

            {addTab === 'text' && (
              <>
                <textarea
                  className="nbw-field"
                  rows={5}
                  placeholder="메모·회의 내용·아이디어를 붙여넣으면 하나의 소스가 됩니다"
                  value={textVal}
                  onChange={(e) => setTextVal(e.target.value)}
                />
                <div className="nbw-dialog-actions">
                  <button type="button" className="nbw-btn" onClick={() => setAddOpen(false)}>
                    취소
                  </button>
                  <button type="button" className="nbw-btn primary" onClick={addText}>
                    추가
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {/* ============ Discover dialog ============ */}
      {discoverOpen && (
        <div className="nbw-overlay" onClick={() => setDiscoverOpen(false)}>
          <div className="nbw-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>✨ 소스 탐색</h3>
            <p className="sub">현재 소스들을 근거로, 함께 보면 좋은 웹 자료를 찾았어요.</p>
            {DISCOVER_SUGGESTIONS.map((s) => (
              <label key={s.id} className="nbw-suggest">
                <input
                  type="checkbox"
                  checked={pickedSuggestions.has(s.id)}
                  onChange={() =>
                    setPickedSuggestions((prev) => {
                      const next = new Set(prev);
                      if (next.has(s.id)) next.delete(s.id);
                      else next.add(s.id);
                      return next;
                    })
                  }
                />
                <span>
                  <span style={{ fontSize: 13, fontWeight: 600 }}>
                    {NB_TYPE_ICON[s.type]} {s.title}
                  </span>
                  <div className="why">추천 이유 — {s.reason}</div>
                </span>
              </label>
            ))}
            <div className="nbw-dialog-actions">
              <button type="button" className="nbw-btn" onClick={() => setDiscoverOpen(false)}>
                닫기
              </button>
              <button type="button" className="nbw-btn primary" onClick={addFromDiscover}>
                선택 항목 추가 ({pickedSuggestions.size})
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ============ Output preview modal ============ */}
      {preview && (
        <div className="nbw-overlay" onClick={() => setPreview(null)}>
          <div className="nbw-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>
              {preview.icon} {preview.title}
            </h3>
            <p className="sub">{preview.meta}</p>
            <div className={`nbw-preview-body${preview.kind === 'mindmap' ? ' mono' : ''}`}>
              {preview.body}
            </div>
            <div className="nbw-dialog-actions">
              {preview.kind === 'note' && (
                <button type="button" className="nbw-btn" onClick={() => promoteNote(preview)}>
                  ⤴ 소스로 승격
                </button>
              )}
              <button type="button" className="nbw-btn primary" onClick={() => setPreview(null)}>
                닫기
              </button>
            </div>
          </div>
        </div>
      )}

      {toast && <div className="nbw-toast">{toast}</div>}
    </div>
  );
}
