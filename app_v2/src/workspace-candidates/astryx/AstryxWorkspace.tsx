// Workspace candidate E — Astryx component philosophy.
// Standalone implementation: same state/logic/data as CultivateWorkspace (D),
// but markup and class names rebuilt to Astryx component anatomy.
// Data shared from ../cultivate/data (no duplication).
// Classes: axw-* prefix. Styles: ./axw.css

import { useEffect, useRef, useState } from 'react';
import {
  CONFLICT,
  CONTRACTS_TEMPLATE,
  CRM,
  INDEX_DOCS,
  INITIAL_DIGEST,
  INITIAL_INBOX,
  MEETINGS,
  SOURCES,
  STRUCTURE_PROPOSAL,
  contextAnswer,
  type CvCatalogItem,
  type CvCollection,
  type CvDigest,
  type CvInboxItem,
  type CvIndexDoc,
  type CvRecord,
  type CvSource,
} from '../cultivate/data';
import './axw.css';

// ---------- Types (identical to D) ----------

type View = 'inbox' | 'index' | string;

type Detail =
  | { kind: 'prov'; colName: string; record: CvRecord; fieldKey: string; fieldLabel: string }
  | { kind: 'gap'; colId: string; record: CvRecord }
  | { kind: 'conflict' }
  | { kind: 'doc'; item: CvCatalogItem; source: CvSource }
  | null;

type AstryxTheme = 'default' | 'neutral';
type AstryxScheme = 'light' | 'dark';

let seq = 1;
const uid = () => seq++;

// ---------- Component ----------

export function AstryxWorkspace() {
  // Theme controls
  const [theme, setTheme] = useState<AstryxTheme>('default');
  const [scheme, setScheme] = useState<AstryxScheme>('light');

  // Workspace state — identical logic to CultivateWorkspace
  const [view, setView] = useState<View>('inbox');
  const [inbox, setInbox] = useState<CvInboxItem[]>(INITIAL_INBOX);
  const [collections, setCollections] = useState<CvCollection[]>([MEETINGS, CRM]);
  const [indexDocs, setIndexDocs] = useState<CvIndexDoc[]>(INDEX_DOCS);
  const [digest, setDigest] = useState<CvDigest>(INITIAL_DIGEST);
  const [grewKey, setGrewKey] = useState(0);
  const [proposal, setProposal] = useState<'pending' | 'extracting' | 'done' | 'dismissed'>('pending');
  const [newIds, setNewIds] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<Detail>(null);
  const [askLog, setAskLog] = useState<{ q: string; a: string }[]>([]);
  const [askInput, setAskInput] = useState('');
  const [searchQ, setSearchQ] = useState('');
  const [pulled, setPulled] = useState<Set<string>>(new Set());
  const [uploadOpen, setUploadOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  // Timer cleanup (StrictMode-safe — collect handles in a ref)
  const timers = useRef<number[]>([]);
  const later = (fn: () => void, ms: number) => {
    timers.current.push(window.setTimeout(fn, ms));
  };
  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), []);

  // ---------- helpers ----------

  function showToast(text: string) {
    setToast(text);
    later(() => setToast(null), 2600);
  }

  function markNew(id: string) {
    setNewIds((prev) => new Set(prev).add(id));
    later(() =>
      setNewIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      }),
      2600,
    );
  }

  function grow(patch: Partial<CvDigest>) {
    setDigest((d) => ({
      inflow: d.inflow + (patch.inflow ?? 0),
      promoted: d.promoted + (patch.promoted ?? 0),
      conflicts: d.conflicts + (patch.conflicts ?? 0),
      gaps: d.gaps + (patch.gaps ?? 0),
    }));
    setGrewKey((k) => k + 1);
  }

  // ---------- inbox triage ----------

  function approve(item: CvInboxItem) {
    setInbox((prev) => prev.filter((i) => i.id !== item.id));
    const rec = item.suggestion.record;
    if (rec) {
      const col = collections.find((c) => c.id === rec.collectionId);
      if (!col) return;
      const keyField = col.fields[0].key;
      const existing = col.records.find((r) => r.values[keyField] === rec.values[keyField]);
      const hadGap = Boolean(existing?.gap);
      const targetId = existing ? existing.id : `rec-${uid()}`;
      const provPatch = { [rec.provField]: { sourceTitle: item.title, quote: rec.quote } };

      setCollections((prev) =>
        prev.map((c) => {
          if (c.id !== rec.collectionId) return c;
          if (existing) {
            return {
              ...c,
              records: c.records.map((r): CvRecord =>
                r.id === existing.id
                  ? { ...r, values: { ...r.values, ...rec.values }, prov: { ...r.prov, ...provPatch }, gap: undefined }
                  : r,
              ),
            };
          }
          return { ...c, records: [...c.records, { id: targetId, values: rec.values, prov: provPatch }] };
        }),
      );
      markNew(targetId);
      grow({ promoted: 1, gaps: hadGap ? -1 : 0 });
      showToast(`「${item.suggestion.target}」에 근거로 승격했어요 — 출처가 함께 기록됩니다`);
      setView(rec.collectionId);
    } else {
      const id = `ix-${uid()}`;
      setIndexDocs((prev) => [
        { id, icon: item.icon, title: item.title, origin: item.origin.split(' ·')[0], indexedAt: '방금' },
        ...prev,
      ]);
      markNew(id);
      grow({ promoted: 1 });
      showToast('지식 인덱스에 색인했어요 — BM25 검색 대상이 됩니다');
      setView('index');
    }
  }

  function setAside(item: CvInboxItem) {
    setInbox((prev) => prev.filter((i) => i.id !== item.id));
    showToast('보류했어요 — 내일 다시 표시됩니다');
  }

  function pullIn(item: CvCatalogItem, source: CvSource) {
    if (pulled.has(item.id)) return;
    const alreadyQueued = inbox.some((i) => i.title === item.title);
    setPulled((prev) => new Set(prev).add(item.id));
    if (alreadyQueued) {
      showToast('이미 인박스에 있어요 — 승인 대기 중');
      setView('inbox');
      return;
    }
    setInbox((prev) => [
      {
        id: `in-pull-${uid()}`,
        icon: item.icon,
        title: item.title,
        origin: `${source.name} · 직접 가져옴`,
        arrivedAt: '방금',
        suggestion: { target: '지식 인덱스', reason: '카탈로그에서 사용자가 직접 선택한 자료' },
      },
      ...prev,
    ]);
    grow({ inflow: 1 });
    showToast('인박스로 가져왔어요 — 승인하면 인덱스에 색인됩니다');
  }

  function upload() {
    setUploadOpen(false);
    setInbox((prev) => [
      {
        id: `in-up-${uid()}`,
        icon: '📄',
        title: '보안 점검 결과 요약.pdf',
        origin: '업로드 · 직접 추가',
        arrivedAt: '방금',
        suggestion: { target: '지식 인덱스', reason: '사용자가 직접 업로드한 자료' },
      },
      ...prev,
    ]);
    grow({ inflow: 1 });
    setView('inbox');
    showToast('업로드했어요 — 인박스에서 승인하면 색인됩니다');
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (view !== 'inbox' || inbox.length === 0) return;
      const target = e.target as HTMLElement;
      if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA') return;
      if (e.key === '1') approve(inbox[0]);
      if (e.key === '2') setAside(inbox[0]);
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // ---------- structure proposal ----------

  function approveProposal() {
    setProposal('extracting');
    later(() => {
      setCollections((prev) => [...prev, CONTRACTS_TEMPLATE]);
      CONTRACTS_TEMPLATE.records.forEach((r) => markNew(r.id));
      setProposal('done');
      grow({ promoted: 2 });
      setView(CONTRACTS_TEMPLATE.id);
      showToast('「계약」 컬렉션이 켜졌어요 — 문서 3건에서 레코드 2건을 추출했습니다');
    }, 1900);
  }

  // ---------- ask-in-context ----------

  function contextLabel(): string {
    if (!detail) return '워크스페이스';
    if (detail.kind === 'prov') return `${detail.colName} · ${detail.fieldLabel}`;
    if (detail.kind === 'gap') return detail.record.values[Object.keys(detail.record.values)[0]];
    if (detail.kind === 'doc') return detail.item.title;
    return '충돌 항목';
  }

  function ask(q: string) {
    const question = q.trim();
    if (!question) return;
    setAskInput('');
    const ctx = contextLabel();
    setAskLog((prev) => [...prev, { q: question, a: '' }]);
    later(() => {
      setAskLog((prev) =>
        prev.map((e, i) => (i === prev.length - 1 ? { ...e, a: contextAnswer(question, ctx) } : e)),
      );
    }, 650);
  }

  function openDetail(d: Detail) {
    setDetail(d);
    setAskLog([]);
    setAskInput('');
  }

  // ---------- derived ----------

  const activeCollection = collections.find((c) => c.id === view) ?? null;
  const activeSource = SOURCES.find((s) => s.id === view) ?? null;
  const filteredDocs = indexDocs.filter((d) =>
    d.title.toLowerCase().includes(searchQ.trim().toLowerCase()),
  );
  const firstGap = collections.flatMap((c) => c.records.filter((r) => r.gap).map((r) => ({ c, r })))[0];

  // ---------- render ----------

  return (
    <div
      className="axw-root"
      data-theme={theme === 'neutral' ? 'neutral' : undefined}
      data-scheme={scheme === 'dark' ? 'dark' : undefined}
      style={{ position: 'relative' }}
    >
      {/* ── Floating theme controls — Astryx SegmentedControl anatomy ── */}
      <div className="axw-controls" title="Astryx 테마 = 컴포넌트 철학 + 토큰">
        <span className="axw-controls-label">Astryx</span>
        <div className="axw-seg" role="group" aria-label="테마 선택">
          <button
            type="button"
            className={`axw-seg-btn${theme === 'default' ? ' is-active' : ''}`}
            onClick={() => setTheme('default')}
          >
            Default
          </button>
          <button
            type="button"
            className={`axw-seg-btn${theme === 'neutral' ? ' is-active' : ''}`}
            onClick={() => setTheme('neutral')}
          >
            Neutral
          </button>
        </div>
        <div className="axw-seg" role="group" aria-label="컬러 스킴 선택">
          <button
            type="button"
            className={`axw-seg-btn${scheme === 'light' ? ' is-active' : ''}`}
            onClick={() => setScheme('light')}
            aria-label="라이트 모드"
          >
            ☀
          </button>
          <button
            type="button"
            className={`axw-seg-btn${scheme === 'dark' ? ' is-active' : ''}`}
            onClick={() => setScheme('dark')}
            aria-label="다크 모드"
          >
            ☾
          </button>
        </div>
      </div>

      {/* ════ SideNav — Astryx SideNav anatomy ════ */}
      <nav className="axw-nav">
        <button
          type="button"
          className={`axw-nav-item${view === 'inbox' ? ' is-active' : ''}`}
          onClick={() => setView('inbox')}
        >
          <span>📥</span>
          <span className="nm">인박스</span>
          <span className={`axw-nav-count${inbox.length > 0 ? ' hot' : ''}`}>{inbox.length}</span>
        </button>

        <div className="axw-nav-sec">컬렉션</div>
        <button
          type="button"
          className={`axw-nav-item${view === 'index' ? ' is-active' : ''}`}
          onClick={() => setView('index')}
        >
          <span>🗂️</span>
          <span className="nm">#0 지식 인덱스</span>
          <span className="axw-nav-count">{indexDocs.length}</span>
        </button>
        {collections.map((c) => (
          <button
            key={c.id}
            type="button"
            className={`axw-nav-item${view === c.id ? ' is-active' : ''}${c.id === 'col-contracts' ? ' is-new' : ''}`}
            onClick={() => setView(c.id)}
          >
            <span>{c.icon}</span>
            <span className="nm">{c.name}</span>
            <span className="axw-nav-count">{c.records.length}</span>
          </button>
        ))}
        <button
          type="button"
          className="axw-nav-add"
          onClick={() => showToast('템플릿 라이브러리 — 스키마를 골라 켜는 화면 (데모 범위 밖)')}
        >
          ＋ 템플릿에서 컬렉션 켜기
        </button>

        <div className="axw-nav-sec">소스 (파이프)</div>
        {SOURCES.map((s) => (
          <button
            key={s.id}
            type="button"
            className={`axw-nav-item${view === s.id ? ' is-active' : ''}`}
            onClick={() => setView(s.id)}
          >
            {/* Astryx StatusDot */}
            <span className={`axw-status-dot${s.status === 'paused' ? ' paused' : ''}`} />
            <span className="nm">{s.name}</span>
            <span className="axw-sync-label">{s.lastSync}</span>
          </button>
        ))}
        <button type="button" className="axw-nav-add" onClick={() => setUploadOpen(true)}>
          ＋ 파일 업로드 · 소스 연결
        </button>
      </nav>

      {/* ════ Main ════ */}
      <section className="axw-main">
        <div className="axw-main-scroll">

          {/* Growth digest — Astryx MetadataList/Token pill, tabular-nums */}
          <div className="axw-digest">
            <span className="axw-digest-label">오늘의 성장</span>
            <span className="axw-stat" key={`in-${grewKey}`}>
              유입 <b>{digest.inflow}</b>
            </span>
            <span className={`axw-stat${grewKey > 0 ? ' grew' : ''}`} key={`pr-${grewKey}`}>
              근거 승격 <b>{digest.promoted}</b>
            </span>
            {/* Semantic warn badge: saturated bg + warn-text */}
            <span
              className="axw-stat warn clickable"
              onClick={() => openDetail({ kind: 'conflict' })}
              title="충돌 상세 보기"
            >
              충돌 <b>{digest.conflicts}</b>
            </span>
            <span
              className="axw-stat warn clickable"
              onClick={() =>
                firstGap && openDetail({ kind: 'gap', colId: firstGap.c.id, record: firstGap.r })
              }
              title="공백 상세 보기"
            >
              공백 <b>{digest.gaps}</b>
            </span>
          </div>

          {/* Structure proposal */}
          {proposal === 'pending' && (
            <div className="axw-proposal">
              <h4>{STRUCTURE_PROPOSAL.title}</h4>
              <p>{STRUCTURE_PROPOSAL.reason}</p>
              <div className="axw-proposal-files">
                감지된 문서: {STRUCTURE_PROPOSAL.detected.join(' · ')}
              </div>
              <div className="axw-proposal-actions">
                <button type="button" className="axw-btn primary" onClick={approveProposal}>
                  켜기 — 자동으로 채우기
                </button>
                <button type="button" className="axw-btn" onClick={() => setProposal('dismissed')}>
                  지금은 안 함
                </button>
              </div>
            </div>
          )}
          {proposal === 'extracting' && (
            <div className="axw-proposal">
              <div className="axw-extracting">
                <span className="axw-spin" />
                계약 문서 3건에서 레코드를 추출하는 중… 필드마다 출처를 기록합니다.
              </div>
            </div>
          )}

          {/* ── Inbox view ── */}
          {view === 'inbox' && (
            <>
              <div className="axw-section-head">
                <h2>인박스</h2>
                <span className="axw-section-sub">새로 유입된 자료 — 승인하면 근거로 승격됩니다</span>
              </div>
              {inbox.length === 0 ? (
                <div className="axw-empty">
                  모두 처리했어요 🌿 새 자료가 들어오면 여기로 옵니다.
                </div>
              ) : (
                inbox.map((item, i) => (
                  <div key={item.id} className="axw-inbox-item">
                    <span className="axw-inbox-icon">{item.icon}</span>
                    <div className="axw-inbox-body">
                      <div className="axw-inbox-title">{item.title}</div>
                      <div className="axw-inbox-meta">
                        {item.origin} · {item.arrivedAt}
                      </div>
                      <div className="axw-suggest">
                        <span className="axw-suggest-to">→ {item.suggestion.target}</span>{' '}
                        <span className="axw-suggest-why">제안 근거 — {item.suggestion.reason}</span>
                      </div>
                      <div className="axw-inbox-actions">
                        <button
                          type="button"
                          className="axw-btn primary"
                          onClick={() => approve(item)}
                        >
                          승인{i === 0 && <span className="axw-kbd">1</span>}
                        </button>
                        <button
                          type="button"
                          className="axw-btn"
                          onClick={() => setAside(item)}
                        >
                          보류{i === 0 && <span className="axw-kbd">2</span>}
                        </button>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </>
          )}

          {/* ── Knowledge index view ── */}
          {view === 'index' && (
            <>
              <div className="axw-section-head">
                <h2>#0 지식 인덱스</h2>
                <span className="axw-section-sub axw-sync-live">
                  <span className="axw-status-dot" />
                  동기화 2분 전 · 문서 {indexDocs.length} · tantivy BM25
                </span>
              </div>
              <div className="axw-search-bar">
                <input
                  className="axw-search-input"
                  placeholder="인덱스 전체에서 검색 (BM25)…"
                  value={searchQ}
                  onChange={(e) => setSearchQ(e.target.value)}
                />
              </div>
              <p className="axw-note">
                모든 유입 자료의 기본 착지점 — 타입 컬렉션으로 올라가지 않아도 검색으로 회수됩니다.
              </p>
              {filteredDocs.map((d) => (
                <div key={d.id} className={`axw-doc-row${newIds.has(d.id) ? ' is-new' : ''}`}>
                  <span>{d.icon}</span>
                  <span className="axw-doc-title">{d.title}</span>
                  <span className="axw-doc-meta">
                    {d.origin} · 색인 {d.indexedAt}
                  </span>
                </div>
              ))}
              {filteredDocs.length === 0 && (
                <div className="axw-empty">"{searchQ}" 검색 결과가 없어요.</div>
              )}
            </>
          )}

          {/* ── Source catalog (pull side) ── */}
          {activeSource && (
            <>
              <div className="axw-section-head">
                <h2>
                  {activeSource.icon} {activeSource.name}
                </h2>
                <span className="axw-section-sub axw-sync-live">
                  <span className={`axw-status-dot${activeSource.status === 'paused' ? ' paused' : ''}`} />
                  {activeSource.status === 'paused' ? '자동 유입 일시정지' : '자동 유입 중'} · 동기화{' '}
                  {activeSource.lastSync}
                </span>
              </div>
              <p className="axw-note">
                이 소스에 존재하는 것들의 카탈로그 — 자동으로 안 들어온 것도 직접 가져오거나, 유입 없이 대화에서 언급할 수 있어요.
              </p>
              {activeSource.catalog.map((item) => {
                const isIn = item.ingested;
                const isPending = pulled.has(item.id);
                return (
                  <div key={item.id} className="axw-doc-row has-acts">
                    <span>{item.icon}</span>
                    <span className="axw-doc-title">{item.title}</span>
                    <div className="axw-doc-sub">
                      {/* Category badge: tint bg + color text (catalog badge rule) */}
                      <span className={`axw-badge${isIn ? ' cat-in' : isPending ? ' cat-pending' : ' cat-none'}`}>
                        {isIn ? '✓ 인덱스됨' : isPending ? '인박스 대기' : '미인덱스'}
                      </span>
                      <span className="axw-doc-meta">{item.modified}</span>
                      <span className="axw-doc-acts">
                        {!isIn && (
                          <button
                            type="button"
                            className="axw-btn sm"
                            disabled={isPending}
                            onClick={() => pullIn(item, activeSource)}
                          >
                            가져오기
                          </button>
                        )}
                        <button
                          type="button"
                          className="axw-btn sm"
                          title="유입 없이 이 문서를 맥락으로 대화"
                          onClick={() => openDetail({ kind: 'doc', item, source: activeSource })}
                        >
                          💬 언급
                        </button>
                      </span>
                    </div>
                  </div>
                );
              })}
            </>
          )}

          {/* ── Typed collection: evidence table — Astryx Table anatomy ── */}
          {activeCollection && (
            <>
              <div className="axw-section-head">
                <h2>
                  {activeCollection.icon} {activeCollection.name}
                </h2>
                <span className="axw-section-sub">
                  선언된 스키마 · 채움은 자동 · 값을 누르면 출처가 열립니다
                </span>
              </div>
              <div className="axw-tablewrap">
                <table className="axw-table">
                  <thead>
                    <tr>
                      {activeCollection.fields.map((f) => (
                        <th key={f.key}>{f.label}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {activeCollection.records.map((r) => (
                      <tr key={r.id} className={newIds.has(r.id) ? 'is-new' : ''}>
                        {activeCollection.fields.map((f, fi) => {
                          const hasProv = Boolean(r.prov[f.key]);
                          return (
                            <td key={f.key}>
                              {hasProv ? (
                                /* Provenance: Astryx Token chip feel — dotted underline */
                                <span
                                  className="axw-prov-cell"
                                  role="button"
                                  tabIndex={0}
                                  onClick={() =>
                                    openDetail({
                                      kind: 'prov',
                                      colName: activeCollection.name,
                                      record: r,
                                      fieldKey: f.key,
                                      fieldLabel: f.label,
                                    })
                                  }
                                >
                                  {r.values[f.key]}
                                </span>
                              ) : (
                                r.values[f.key]
                              )}
                              {fi === 0 && r.gap && (
                                /* Gap: semantic warn badge — saturated bg + contrast text */
                                <span
                                  className="axw-badge sem-warn"
                                  style={{ marginLeft: 6, cursor: 'pointer' }}
                                  role="button"
                                  tabIndex={0}
                                  onClick={() =>
                                    openDetail({ kind: 'gap', colId: activeCollection.id, record: r })
                                  }
                                >
                                  ⚠ 공백
                                </span>
                              )}
                            </td>
                          );
                        })}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </div>
      </section>

      {/* ════ Detail panel — shadow-med, bezel in dark ════ */}
      <aside className="axw-detail">
        <div className="axw-detail-head">
          <span>
            {detail === null && '상세'}
            {detail?.kind === 'prov' && '출처 (provenance)'}
            {detail?.kind === 'gap' && '공백 플래그'}
            {detail?.kind === 'conflict' && '충돌'}
            {detail?.kind === 'doc' && '문서 언급'}
          </span>
          {detail && (
            <button type="button" className="axw-btn ghost" onClick={() => setDetail(null)}>
              ✕
            </button>
          )}
        </div>

        <div className="axw-detail-body">
          {detail === null && (
            <div className="axw-empty-detail">
              테이블의 값·⚠ 공백·충돌 숫자를 누르면
              <br />
              근거와 출처가 여기 열립니다.
            </div>
          )}

          {detail?.kind === 'prov' && (
            <>
              <div className="axw-detail-sec">
                {detail.colName} · {detail.fieldLabel}
              </div>
              <div className="axw-detail-value">{detail.record.values[detail.fieldKey]}</div>
              <div className="axw-detail-sec">출처 구절</div>
              <div className="axw-prov-quote">
                <mark>{detail.record.prov[detail.fieldKey]?.quote}</mark>
              </div>
              <div className="axw-prov-src">📄 {detail.record.prov[detail.fieldKey]?.sourceTitle}</div>
              <div className="axw-detail-actions">
                <button
                  type="button"
                  className="axw-btn"
                  onClick={() => showToast('원문 미리보기 — 기존 프리뷰 컴포넌트 연결 지점 (데모)')}
                >
                  원문 열기
                </button>
                <button
                  type="button"
                  className="axw-btn"
                  onClick={() => showToast('값 수정 — 사용자가 고치면 추출 이력에 기록됩니다 (데모)')}
                >
                  값 고치기
                </button>
              </div>
            </>
          )}

          {detail?.kind === 'gap' && (
            <>
              <div className="axw-detail-sec">레코드</div>
              <div className="axw-detail-value">
                {detail.record.values[Object.keys(detail.record.values)[0]]}
              </div>
              <div className="axw-detail-sec">자동 감지된 공백</div>
              <div className="axw-prov-quote">{detail.record.gap ?? '—'}</div>
              <p className="axw-conflict-hint">
                워크스페이스가 유지 스캔 중 발견한 공백입니다. 인박스의 "C사 회신"을 승인하면 이 공백이 해소됩니다.
              </p>
              <div className="axw-detail-actions">
                <button
                  type="button"
                  className="axw-btn primary"
                  onClick={() => ask('C사 후속 메일 초안을 만들어줘')}
                >
                  후속 메일 초안 부탁
                </button>
                <button type="button" className="axw-btn" onClick={() => setView('inbox')}>
                  인박스에서 처리
                </button>
              </div>
            </>
          )}

          {detail?.kind === 'doc' && (
            <>
              <div className="axw-detail-sec">
                {detail.source.name} · {detail.item.ingested ? '인덱스됨' : '미인덱스 (유입 없이 참조)'}
              </div>
              <div className="axw-detail-value">
                {detail.item.icon} {detail.item.title}
              </div>
              <p className="axw-conflict-hint">
                이 문서를 맥락으로 아래에서 바로 질문할 수 있어요. 인덱스에 없어도 에이전트가 원본을 조회해 답합니다 — 자주 쓰게 되면 가져오기를 권해요.
              </p>
              <div className="axw-detail-actions">
                {!detail.item.ingested && !pulled.has(detail.item.id) && (
                  <button
                    type="button"
                    className="axw-btn"
                    onClick={() => pullIn(detail.item, detail.source)}
                  >
                    인박스로 가져오기
                  </button>
                )}
                <button
                  type="button"
                  className="axw-btn"
                  onClick={() => showToast('원문 미리보기 — 프리뷰 컴포넌트 연결 지점 (데모)')}
                >
                  원문 열기
                </button>
              </div>
            </>
          )}

          {detail?.kind === 'conflict' && (
            <>
              <div className="axw-detail-sec">{CONFLICT.title}</div>
              <div className="axw-conflict-pair">
                <div className="axw-conflict-side newer">
                  <span className="axw-conflict-src">최신 · {CONFLICT.a.sourceTitle}</span>
                  {CONFLICT.a.quote}
                </div>
                <div className="axw-conflict-side">
                  <span className="axw-conflict-src">구버전 · {CONFLICT.b.sourceTitle}</span>
                  {CONFLICT.b.quote}
                </div>
              </div>
              <p className="axw-conflict-hint">{CONFLICT.hint}</p>
              <div className="axw-detail-actions">
                <button
                  type="button"
                  className="axw-btn primary"
                  onClick={() => {
                    grow({ conflicts: -1 });
                    setDetail(null);
                    showToast('충돌 해소 — 구버전 구절에 갱신 메모를 남겼어요');
                  }}
                >
                  최신 기준으로 정리
                </button>
              </div>
            </>
          )}
        </div>

        {/* Ask-in-context drawer */}
        {detail && (
          <div className="axw-ask">
            <div className="axw-detail-sec" style={{ margin: '0 0 6px' }}>
              여기서 묻기 — {contextLabel()}
            </div>
            {askLog.length > 0 && (
              <div className="axw-ask-log">
                {askLog.map((e, i) => (
                  <div key={i}>
                    <div className="axw-ask-q">Q. {e.q}</div>
                    <div className="axw-ask-a">{e.a || '생각 중…'}</div>
                  </div>
                ))}
              </div>
            )}
            <div className="axw-ask-row">
              <input
                className="axw-ask-input"
                placeholder="이 맥락에서 질문…"
                value={askInput}
                onChange={(e) => setAskInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') ask(askInput);
                }}
              />
              <button type="button" className="axw-btn primary" onClick={() => ask(askInput)}>
                묻기
              </button>
            </div>
            <div className="axw-ask-hint">
              대화는 목적지가 아니라 동사 — 답은 이 맥락의 근거를 인용합니다
            </div>
          </div>
        )}
      </aside>

      {/* Upload dialog — Astryx Dialog anatomy: shadow-high + overlay scrim */}
      {uploadOpen && (
        <div className="axw-overlay" onClick={() => setUploadOpen(false)}>
          <div className="axw-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>파일 업로드 · 소스 연결</h3>
            <p className="axw-dialog-sub">
              직접 올린 파일도 같은 파이프를 탑니다 — 인박스에서 승인하면 색인돼요.
            </p>
            <div className="axw-drop" role="button" tabIndex={0} onClick={upload}>
              ☁️ 클릭하거나 파일을 끌어다 놓으세요
              <span className="axw-drop-hint">(데모: 샘플 파일이 인박스로 들어갑니다)</span>
            </div>
            <div className="axw-dialog-footer">
              <button
                type="button"
                className="axw-btn"
                onClick={() => showToast('새 소스 연결(OAuth) 플로우 — 데모 범위 밖')}
              >
                ＋ 새 소스 연결
              </button>
              <button type="button" className="axw-btn primary" onClick={() => setUploadOpen(false)}>
                닫기
              </button>
            </div>
          </div>
        </div>
      )}

      {toast && <div className="axw-toast">{toast}</div>}
    </div>
  );
}
