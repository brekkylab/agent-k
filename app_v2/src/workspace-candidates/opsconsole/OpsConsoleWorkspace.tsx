// Workspace candidate F — ops console archetype ("Glean/Dust 운영 콘솔").
// Philosophy (docs/workspace-cultivation-direction.md) expressed as ops-console idioms:
//   - 자료→근거 (material→evidence): evidence rate score + extract action
//   - 관계 (relationship): doc detail panel with related doc links
//   - deposit loop: session artifacts as 1st-class source
//   - 충돌 (conflict): conflict card type in the verify inbox

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  computeEvidenceRate,
  computeTrust,
  deriveVerifyQueue,
  findDoc,
  INITIAL_SOURCES,
  SPARKLINE_SEED,
  urgencyLabel,
  type FConflict,
  type FDoc,
  type FDocQueueItem,
  type FQueueItem,
  type FSource,
} from './data';
import './ops.css';

// ---------- types ----------

interface Toast {
  id: number;
  msg: string;
}

interface PopoverState {
  docId: string;
  sourceId: string;
}

interface ConnectDialogState {
  open: boolean;
  selected: number | null;
}

// ---------- constants ----------

const CONNECT_OPTIONS = [
  { icon: '📓', name: 'Notion', id: 'notion' },
  { icon: '🔀', name: 'Linear', id: 'linear' },
  { icon: '🐙', name: 'GitHub', id: 'github' },
];

const STREAM_DOC_TITLES: Record<string, string[]> = {
  notion: ['회의록 데이터베이스.db', '제품 로드맵.page', 'OKR 트래커.page'],
  linear: ['백로그 이슈 목록', 'Q3 마일스톤 사이클', '버그 트래킹'],
  github: ['README.md', 'CHANGELOG.md', 'API 레퍼런스.md'],
};

let toastCounter = 0;

// ---------- small helpers ----------

function BadgeCell({
  doc,
  sourceId,
  onPopover,
}: {
  doc: FDoc;
  sourceId: string;
  onPopover: (state: PopoverState | null) => void;
}) {
  const tooltipText =
    doc.verify === 'verified' && doc.verifiedAt
      ? `누가: 나 · 언제: ${doc.verifiedAt}`
      : doc.verify === 'deprecated'
        ? '구식 처리됨'
        : '미검증';

  return (
    <div className="fw-badge-wrap">
      <button
        type="button"
        className={`fw-badge fw-badge-${doc.verify}`}
        onClick={(e) => {
          e.stopPropagation();
          onPopover({ docId: doc.id, sourceId });
        }}
        title={tooltipText}
      >
        {doc.verify === 'verified' && '✅ 검증됨'}
        {doc.verify === 'unverified' && '❔ 미검증'}
        {doc.verify === 'deprecated' && '구식'}
      </button>
      <span className="fw-badge-tooltip">{tooltipText}</span>
    </div>
  );
}

function IndexCell({
  doc,
  onRetry,
}: {
  doc: FDoc;
  onRetry: (docId: string) => void;
}) {
  if (doc.indexState === 'done') {
    return <span className="fw-index-cell fw-index-done">✓ 완료</span>;
  }
  if (doc.indexState === 'failed') {
    return (
      <span className="fw-index-cell fw-index-failed">
        ✗ 실패&nbsp;
        <button
          type="button"
          className="fw-btn fw-btn-sm"
          onClick={(e) => { e.stopPropagation(); onRetry(doc.id); }}
        >
          재시도
        </button>
      </span>
    );
  }
  if (doc.indexState === 'indexing') {
    return (
      <span className="fw-index-cell">
        <span className="fw-spinner" />
        <span className="fw-index-bar-wrap">
          <span
            className="fw-index-bar"
            style={{ '--pct': doc.indexPct ?? 0 } as React.CSSProperties}
          />
        </span>
        <span>{Math.round((doc.indexPct ?? 0) * 100)}%</span>
      </span>
    );
  }
  return <span className="fw-index-cell fw-index-pending">⏱ 대기</span>;
}

// ---------- doc detail panel — Transformation 1 ----------

function DocDetailPanel({
  doc,
  source,
  allSources,
  onClose,
  onJumpToDoc,
  onVerify,
  onDeprecate,
  onExtract,
  extracting,
}: {
  doc: FDoc;
  source: FSource;
  allSources: FSource[];
  onClose: () => void;
  onJumpToDoc: (docId: string) => void;
  onVerify: (docId: string) => void;
  onDeprecate: (docId: string) => void;
  onExtract: (docId: string) => void;
  extracting: boolean;
}) {
  const hasEvidence = doc.evidence && doc.evidence.length > 0;
  const relatedDocs = (doc.related ?? [])
    .map((id) => findDoc(allSources, id))
    .filter((r): r is NonNullable<typeof r> => r !== null);

  return (
    <div className="fw-detail-panel">
      <div className="fw-panel-header">
        <span style={{ fontSize: 16 }}>{doc.icon}</span>
        <span className="fw-panel-title">{doc.title}</span>
        <button type="button" className="fw-panel-close" onClick={onClose} aria-label="닫기">
          ×
        </button>
      </div>
      <div className="fw-panel-scroll">
        {/* body preview */}
        {doc.body && (
          <div>
            <div className="fw-panel-section-label">본문 미리보기</div>
            <div className="fw-panel-body">{doc.body}</div>
          </div>
        )}

        {/* meta */}
        <div>
          <div className="fw-panel-section-label">메타</div>
          <div className="fw-panel-meta-row">
            <span className="fw-panel-meta-key">소스</span>
            <span className="fw-panel-meta-val">{source.icon} {source.name}</span>
          </div>
          <div className="fw-panel-meta-row">
            <span className="fw-panel-meta-key">인덱싱</span>
            <span className="fw-panel-meta-val">{doc.indexState}</span>
          </div>
          <div className="fw-panel-meta-row">
            <span className="fw-panel-meta-key">검증 상태</span>
            <span className="fw-panel-meta-val">{doc.verify}</span>
          </div>
          {doc.verifiedAt && (
            <div className="fw-panel-meta-row">
              <span className="fw-panel-meta-key">검증일</span>
              <span className="fw-panel-meta-val">{doc.verifiedAt}</span>
            </div>
          )}
          {doc.expiresAt && (
            <div className="fw-panel-meta-row">
              <span className="fw-panel-meta-key">만료</span>
              <span className="fw-panel-meta-val">{doc.expiresAt}</span>
            </div>
          )}
        </div>

        {/* evidence — Transformation 2 */}
        <div>
          <div className="fw-panel-section-label">이 문서에서 추출된 근거</div>
          {hasEvidence ? (
            (doc.evidence ?? []).map((ev, i) => (
              <div key={i} className="fw-evidence-item">
                <div className="fw-evidence-path">
                  {ev.collection} › {ev.field}
                </div>
                <div className="fw-evidence-value">{ev.value}</div>
              </div>
            ))
          ) : (
            <>
              <div className="fw-no-evidence">근거화된 항목이 없습니다.</div>
              {extracting ? (
                <span className="fw-extract-spinner">
                  <span className="fw-spinner" />
                  컬렉션으로 추출 중…
                </span>
              ) : (
                <button
                  type="button"
                  className="fw-btn fw-btn-extract fw-btn-sm"
                  onClick={() => onExtract(doc.id)}
                  disabled={doc.indexState !== 'done'}
                >
                  컬렉션으로 추출
                </button>
              )}
            </>
          )}
        </div>

        {/* related docs */}
        {relatedDocs.length > 0 && (
          <div>
            <div className="fw-panel-section-label">관련 문서</div>
            <div className="fw-related-list">
              {relatedDocs.map(({ doc: rd, source: rs }) => (
                <button
                  type="button"
                  key={rd.id}
                  className="fw-related-item"
                  onClick={() => onJumpToDoc(rd.id)}
                >
                  <span style={{ fontSize: 13 }}>{rd.icon}</span>
                  <span style={{ flex: 1, textAlign: 'left', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {rd.title}
                  </span>
                  <span style={{ fontSize: 11, color: 'var(--cw-ink-muted)', flexShrink: 0 }}>
                    {rs.name}
                  </span>
                </button>
              ))}
            </div>
          </div>
        )}

        {/* usage timeline */}
        {doc.usage && doc.usage.length > 0 && (
          <div>
            <div className="fw-panel-section-label">참조 타임라인</div>
            <div className="fw-usage-list">
              {doc.usage.map((u, i) => (
                <div key={i} className="fw-usage-item">
                  <span className="fw-usage-when">{u.when}</span>
                  <span className="fw-usage-what">{u.what}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* verify actions */}
        <div style={{ display: 'flex', gap: 8, paddingTop: 4 }}>
          <button
            type="button"
            className="fw-btn fw-btn-verify fw-btn-sm"
            onClick={() => onVerify(doc.id)}
          >
            ✅ 검증
          </button>
          <button
            type="button"
            className="fw-btn fw-btn-deprecate fw-btn-sm"
            onClick={() => onDeprecate(doc.id)}
          >
            구식 처리
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------- conflict card — Transformation 3 ----------

function ConflictCard({
  conflict,
  onAdoptLatest,
  onKeepBoth,
}: {
  conflict: FConflict;
  onAdoptLatest: () => void;
  onKeepBoth: () => void;
}) {
  return (
    <div className="fw-conflict-card">
      <div className="fw-conflict-label">⚠ 충돌 감지</div>
      <div className="fw-conflict-title">{conflict.title}</div>
      <div className="fw-conflict-quotes">
        <div className="fw-conflict-quote">
          <div className="fw-conflict-source">{conflict.docA.sourceTitle}</div>
          {conflict.docA.quote}
        </div>
        <div className="fw-conflict-quote">
          <div className="fw-conflict-source">{conflict.docB.sourceTitle}</div>
          {conflict.docB.quote}
        </div>
      </div>
      <div className="fw-conflict-hint">💡 {conflict.hint}</div>
      <div className="fw-queue-actions">
        <button type="button" className="fw-btn fw-btn-verify" onClick={onAdoptLatest}>
          최신 채택
        </button>
        <button type="button" className="fw-btn fw-btn-deprecate" onClick={onKeepBoth}>
          둘 다 유지
        </button>
      </div>
    </div>
  );
}

// ---------- main component ----------

export function OpsConsoleWorkspace() {
  const [sources, setSources] = useState<FSource[]>(INITIAL_SOURCES);
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);
  const [selectedDocId, setSelectedDocId] = useState<string | null>(null);
  const [extractingDocId, setExtractingDocId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [popover, setPopover] = useState<PopoverState | null>(null);
  const [sparkline, setSparkline] = useState<number[]>(SPARKLINE_SEED);
  const [trustPopClass, setTrustPopClass] = useState('');
  const [evidencePopClass, setEvidencePopClass] = useState('');
  const [showConflict, setShowConflict] = useState(true);
  const [connectDialog, setConnectDialog] = useState<ConnectDialogState>({
    open: false,
    selected: null,
  });
  const [syncingSourceId, setSyncingSourceId] = useState<string | null>(null);

  const timerRefs = useRef<ReturnType<typeof setTimeout>[]>([]);
  const addTimer = useCallback((id: ReturnType<typeof setTimeout>) => {
    timerRefs.current.push(id);
  }, []);

  useEffect(() => {
    const ids = timerRefs.current;
    return () => { ids.forEach(clearTimeout); };
  }, []);

  // ---------- derived state ----------

  const trust = computeTrust(sources);
  const evidenceRate = computeEvidenceRate(sources);
  const queue = deriveVerifyQueue(sources, showConflict);
  const panelResult = selectedDocId ? findDoc(sources, selectedDocId) : null;
  const is3Col = panelResult !== null;

  // ---------- toast ----------

  const showToast = useCallback((msg: string) => {
    const id = ++toastCounter;
    setToasts((prev) => [...prev, { id, msg }]);
    const t = setTimeout(() => {
      setToasts((prev) => prev.filter((x) => x.id !== id));
    }, 2800);
    addTimer(t);
  }, [addTimer]);

  // ---------- pop animations ----------

  const popTrust = useCallback(() => {
    setTrustPopClass('fw-trust-pop');
    const t = setTimeout(() => setTrustPopClass(''), 400);
    addTimer(t);
  }, [addTimer]);

  const popEvidence = useCallback(() => {
    setEvidencePopClass('fw-score-pop');
    const t = setTimeout(() => setEvidencePopClass(''), 400);
    addTimer(t);
  }, [addTimer]);

  // ---------- verify / deprecate ----------

  const applyVerify = useCallback(
    (docId: string, action: 'verified' | 'deprecated') => {
      setSources((prev) =>
        prev.map((s) => ({
          ...s,
          docs: s.docs.map((d) =>
            d.id === docId
              ? { ...d, verify: action, verifiedAt: action === 'verified' ? '방금' : d.verifiedAt }
              : d,
          ),
        })),
      );
      setPopover(null);
      popTrust();
      if (selectedDocId === docId) setSelectedDocId(null);
      showToast(action === 'verified' ? '✓ 검증 완료' : '✓ 구식 처리 완료');
      setSparkline((prev) => {
        const next = [...prev];
        next[6] = Math.min(100, Math.max(0, next[6] + (action === 'verified' ? 3 : -2)));
        return next;
      });
    },
    [popTrust, selectedDocId, showToast],
  );

  // ---------- extract evidence — Transformation 2 ----------

  const handleExtract = useCallback(
    (docId: string) => {
      // decide extracted evidence outside setState to stay StrictMode-pure
      const evidenceMap: Record<string, Array<{ collection: string; field: string; value: string }>> = {
        'doc-d2': [{ collection: '계약', field: '주의 조항', value: '9조 배상 상한 200%' }],
        'doc-sess2': [{ collection: '계약', field: '수정안', value: '9조 상한 100% 수정안 초안' }],
        'doc-g2': [{ collection: 'CRM', field: '마지막 접점', value: '7/07 데모 일정 회신' }],
        'doc-j1': [{ collection: '릴리스', field: '성능 지표', value: '결제 API 316ms' }],
        'doc-sess1': [{ collection: 'CRM', field: '견적 근거', value: '트래픽 전망 산출 스크립트' }],
      };

      setExtractingDocId(docId);
      const t = setTimeout(() => {
        const ev = evidenceMap[docId] ?? [{ collection: '지식 인덱스', field: '핵심 내용', value: docId }];
        setSources((prev) =>
          prev.map((s) => ({
            ...s,
            docs: s.docs.map((d) =>
              d.id === docId ? { ...d, evidence: ev } : d,
            ),
          })),
        );
        setExtractingDocId(null);
        popEvidence();
        showToast('✓ 근거 추출 완료');
      }, 1200);
      addTimer(t);
    },
    [addTimer, popEvidence, showToast],
  );

  // ---------- conflict actions — Transformation 3 ----------

  const handleAdoptLatest = useCallback(() => {
    // Deprecate the outdated 340-quote doc (doc-g1). Aligned with B/C: 320 re-adjustment is latest.
    setSources((prev) =>
      prev.map((s) => ({
        ...s,
        docs: s.docs.map((d) =>
          d.id === 'doc-g1' ? { ...d, verify: 'deprecated' as const } : d,
        ),
      })),
    );
    setShowConflict(false);
    popTrust();
    showToast('✓ 최신 문서 기준으로 채택 완료');
  }, [popTrust, showToast]);

  const handleKeepBoth = useCallback(() => {
    setShowConflict(false);
    showToast('알림: 두 문서를 모두 유지합니다');
  }, [showToast]);

  // ---------- keyboard shortcuts ----------

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement).isContentEditable) return;
      if (selectedSourceId !== null) return;

      const firstDocItem = queue.find((q): q is FDocQueueItem => q.kind === 'doc');
      if (e.key === '1' && firstDocItem) {
        applyVerify(firstDocItem.id, 'verified');
      } else if (e.key === '2' && firstDocItem) {
        applyVerify(firstDocItem.id, 'deprecated');
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [applyVerify, queue, selectedSourceId]);

  // ---------- doc retry ----------

  const handleRetry = useCallback(
    (docId: string) => {
      setSources((prev) =>
        prev.map((s) => ({
          ...s,
          docs: s.docs.map((d) =>
            d.id === docId ? { ...d, indexState: 'indexing' as const, indexPct: 0.1 } : d,
          ),
        })),
      );
      const steps = [0.3, 0.6, 0.85, 1.0];
      steps.forEach((pct, i) => {
        const t = setTimeout(() => {
          if (pct < 1.0) {
            setSources((prev) =>
              prev.map((s) => ({
                ...s,
                docs: s.docs.map((d) => (d.id === docId ? { ...d, indexPct: pct } : d)),
              })),
            );
          } else {
            setSources((prev) =>
              prev.map((s) => ({
                ...s,
                docs: s.docs.map((d) =>
                  d.id === docId ? { ...d, indexState: 'done' as const, indexPct: undefined } : d,
                ),
              })),
            );
            showToast('✓ 인덱싱 완료');
          }
        }, (i + 1) * 500);
        addTimer(t);
      });
    },
    [addTimer, showToast],
  );

  // ---------- sync ----------

  const handleSync = useCallback(
    (sourceId: string) => {
      if (syncingSourceId) return;
      setSyncingSourceId(sourceId);
      const t = setTimeout(() => {
        let flippedId: string | null = null;
        setSources((prev) =>
          prev.map((s) => {
            if (s.id !== sourceId) return s;
            let flipped = false;
            return {
              ...s,
              lastSync: '방금',
              docs: s.docs.map((d) => {
                if (!flipped && d.indexState !== 'done' && d.indexState !== 'failed') {
                  flipped = true;
                  flippedId = d.id;
                  return { ...d, indexState: 'indexing' as const, indexPct: 0.5 };
                }
                return d;
              }),
            };
          }),
        );
        setSyncingSourceId(null);
        const capturedId = flippedId;
        const t2 = setTimeout(() => {
          if (!capturedId) return;
          setSources((prev) =>
            prev.map((s) =>
              s.id === sourceId
                ? {
                    ...s,
                    docs: s.docs.map((d) =>
                      d.id === capturedId
                        ? { ...d, indexState: 'done' as const, indexPct: undefined }
                        : d,
                    ),
                  }
                : s,
            ),
          );
          showToast('✓ 동기화 완료');
        }, 600);
        addTimer(t2);
      }, 1200);
      addTimer(t);
    },
    [addTimer, showToast, syncingSourceId],
  );

  // ---------- connect source ----------

  const handleConnectConfirm = useCallback(() => {
    const optIdx = connectDialog.selected;
    if (optIdx === null) return;
    const opt = CONNECT_OPTIONS[optIdx];
    setConnectDialog({ open: false, selected: null });

    const newSourceId = `src-${opt.id}-${Date.now()}`;
    const pendingDocs: FDoc[] = (STREAM_DOC_TITLES[opt.id] ?? []).map((title, i) => ({
      id: `nd-${opt.id}-${i}`,
      icon: '📄' as const,
      title,
      indexState: 'pending' as const,
      verify: 'unverified' as const,
    }));

    setSources((prev) => [
      ...prev,
      { id: newSourceId, icon: opt.icon, name: opt.name, status: 'live' as const, lastSync: '방금', docs: pendingDocs },
    ]);
    showToast(`✓ ${opt.name} 연결됨`);

    pendingDocs.forEach((doc, i) => {
      const t1 = setTimeout(() => {
        setSources((prev) =>
          prev.map((s) =>
            s.id === newSourceId
              ? { ...s, docs: s.docs.map((d) => d.id === doc.id ? { ...d, indexState: 'indexing' as const, indexPct: 0.3 } : d) }
              : s,
          ),
        );
        const t2 = setTimeout(() => {
          setSources((prev) =>
            prev.map((s) =>
              s.id === newSourceId
                ? { ...s, docs: s.docs.map((d) => d.id === doc.id ? { ...d, indexPct: 0.85 } : d) }
                : s,
            ),
          );
          const t3 = setTimeout(() => {
            setSources((prev) =>
              prev.map((s) =>
                s.id === newSourceId
                  ? { ...s, docs: s.docs.map((d) => d.id === doc.id ? { ...d, indexState: 'done' as const, indexPct: undefined } : d) }
                  : s,
              ),
            );
          }, 500);
          addTimer(t3);
        }, 400);
        addTimer(t2);
      }, i * 700 + 300);
      addTimer(t1);
    });
  }, [addTimer, connectDialog.selected, showToast]);

  // ---------- search ----------

  const searchQuery = search.trim().toLowerCase();
  const allDocs = sources.flatMap((s) =>
    s.docs.map((d) => ({ ...d, sourceName: s.name, sourceId: s.id })),
  );
  const searchResults = searchQuery
    ? allDocs.filter((d) => d.title.toLowerCase().includes(searchQuery))
    : [];
  const searchGroups: Record<string, typeof searchResults> = {};
  for (const r of searchResults) {
    if (!searchGroups[r.sourceName]) searchGroups[r.sourceName] = [];
    searchGroups[r.sourceName].push(r);
  }

  const selectedSource = selectedSourceId
    ? sources.find((s) => s.id === selectedSourceId)
    : null;

  // ---------- render ----------

  return (
    <div
      className={`fw-root${is3Col ? ' fw-root-3col' : ''}`}
      onClick={() => setPopover(null)}
    >
      {/* ── left rail ── */}
      <aside className="fw-rail" onClick={(e) => e.stopPropagation()}>
        {/* trust + evidence rate — Transformation 2 */}
        <div className="fw-trust-block">
          <div className="fw-score-row">
            <div className="fw-score-block">
              <div className="fw-score-label">신뢰도</div>
              <div className="fw-trust-score">
                <span className={`fw-score-num ${trustPopClass}`}>{trust}</span>
                <span className="fw-score-pct">%</span>
              </div>
            </div>
            <div className="fw-score-block">
              <div className="fw-score-label">근거화율</div>
              <div className="fw-trust-score">
                <span className={`fw-score-num ${evidencePopClass}`}>{evidenceRate}</span>
                <span className="fw-score-pct">%</span>
              </div>
            </div>
          </div>
          <div className="fw-trust-caption">검증되고 최신인 재료 비율</div>
          <div className="fw-sparkline">
            {sparkline.map((val, i) => {
              const maxVal = Math.max(...sparkline, 100);
              return (
                <div
                  key={i}
                  className={`fw-spark-bar${i === 6 ? ' fw-spark-today' : ''}`}
                  style={{ height: `${Math.round((val / maxVal) * 100)}%` }}
                />
              );
            })}
          </div>
        </div>

        <div className="fw-rail-section-label">소스</div>
        <div className="fw-source-list">
          {sources.map((s) => (
            <div
              key={s.id}
              className={`fw-source-row${selectedSourceId === s.id ? ' fw-source-active' : ''}`}
              onClick={() => setSelectedSourceId((prev) => (prev === s.id ? null : s.id))}
            >
              <span className={`fw-source-dot fw-dot-${s.status}`} />
              <span className="fw-source-icon">{s.icon}</span>
              <span className="fw-source-meta">
                <div className="fw-source-name">{s.name}</div>
                <div className="fw-source-sync">{s.lastSync}</div>
              </span>
              <span className="fw-source-count">{s.docs.length}개</span>
            </div>
          ))}
        </div>

        <button
          type="button"
          className="fw-add-source"
          onClick={() => setConnectDialog({ open: true, selected: null })}
        >
          ＋ 소스 연결
        </button>
      </aside>

      {/* ── main panel ── */}
      <div className="fw-main" onClick={() => setPopover(null)}>
        <div className="fw-search-bar">
          <input
            className="fw-search-input"
            type="text"
            placeholder="모든 소스에서 검색…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <div className="fw-content" onClick={(e) => e.stopPropagation()}>
          {searchQuery ? (
            <>
              {Object.keys(searchGroups).length === 0 && (
                <div className="fw-empty">검색 결과 없음</div>
              )}
              {Object.entries(searchGroups).map(([srcName, docs]) => (
                <div key={srcName} className="fw-search-group">
                  <div className="fw-search-group-label">{srcName}</div>
                  {docs.map((d) => (
                    <div
                      key={d.id}
                      className="fw-search-result-row"
                      style={{ cursor: 'pointer' }}
                      onClick={() => setSelectedDocId(d.id)}
                    >
                      <span className="fw-doc-icon">{d.icon}</span>
                      <span className="fw-search-result-title">{d.title}</span>
                      <BadgeCell doc={d} sourceId={d.sourceId} onPopover={setPopover} />
                    </div>
                  ))}
                </div>
              ))}
            </>
          ) : selectedSource ? (
            <SourceDetailView
              source={selectedSource}
              isSyncing={syncingSourceId === selectedSource.id}
              onSync={handleSync}
              onRetry={handleRetry}
              popover={popover}
              onPopover={setPopover}
              onVerify={(docId) => applyVerify(docId, 'verified')}
              onDeprecate={(docId) => applyVerify(docId, 'deprecated')}
              selectedDocId={selectedDocId}
              onSelectDoc={setSelectedDocId}
            />
          ) : (
            <InboxView
              queue={queue}
              onVerify={(docId) => applyVerify(docId, 'verified')}
              onDeprecate={(docId) => applyVerify(docId, 'deprecated')}
              onAdoptLatest={handleAdoptLatest}
              onKeepBoth={handleKeepBoth}
              onSelectDoc={setSelectedDocId}
            />
          )}
        </div>
      </div>

      {/* ── doc detail panel — Transformation 1 ── */}
      {panelResult && (
        <DocDetailPanel
          doc={panelResult.doc}
          source={panelResult.source}
          allSources={sources}
          onClose={() => setSelectedDocId(null)}
          onJumpToDoc={setSelectedDocId}
          onVerify={(docId) => applyVerify(docId, 'verified')}
          onDeprecate={(docId) => applyVerify(docId, 'deprecated')}
          onExtract={handleExtract}
          extracting={extractingDocId === panelResult.doc.id}
        />
      )}

      {/* ── connect dialog ── */}
      {connectDialog.open && (
        <div
          className="fw-dialog-overlay"
          onClick={() => setConnectDialog({ open: false, selected: null })}
        >
          <div className="fw-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="fw-dialog-title">소스 연결</div>
            <div className="fw-dialog-options">
              {CONNECT_OPTIONS.map((opt, i) => (
                <div
                  key={opt.id}
                  className={`fw-dialog-option${connectDialog.selected === i ? ' fw-option-selected' : ''}`}
                  onClick={() => setConnectDialog((prev) => ({ ...prev, selected: i }))}
                >
                  <span className="fw-dialog-option-icon">{opt.icon}</span>
                  <span>{opt.name}</span>
                </div>
              ))}
            </div>
            <div className="fw-dialog-actions">
              <button
                type="button"
                className="fw-btn"
                onClick={() => setConnectDialog({ open: false, selected: null })}
              >
                취소
              </button>
              <button
                type="button"
                className="fw-btn fw-btn-primary"
                disabled={connectDialog.selected === null}
                onClick={handleConnectConfirm}
              >
                OAuth 연결
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ── toasts ── */}
      {toasts.map((t) => (
        <div key={t.id} className="fw-toast">{t.msg}</div>
      ))}
    </div>
  );
}

// ---------- inbox sub-component ----------

function InboxView({
  queue,
  onVerify,
  onDeprecate,
  onAdoptLatest,
  onKeepBoth,
  onSelectDoc,
}: {
  queue: FQueueItem[];
  onVerify: (docId: string) => void;
  onDeprecate: (docId: string) => void;
  onAdoptLatest: () => void;
  onKeepBoth: () => void;
  onSelectDoc: (docId: string) => void;
}) {
  return (
    <>
      <div className="fw-inbox-title">검증 인박스</div>
      <div className="fw-keyboard-hint">
        최상단 문서 항목: <kbd className="fw-kbd">1</kbd> 여전히 유효&nbsp;
        <kbd className="fw-kbd">2</kbd> 구식 처리
      </div>
      {queue.length === 0 ? (
        <div className="fw-empty">검증 대기 항목 없음 — 신뢰도 양호!</div>
      ) : (
        <div className="fw-queue-list">
          {queue.map((item) => {
            if (item.kind === 'conflict') {
              return (
                <ConflictCard
                  key={item.id}
                  conflict={item}
                  onAdoptLatest={onAdoptLatest}
                  onKeepBoth={onKeepBoth}
                />
              );
            }
            return (
              <div
                key={item.id}
                className="fw-queue-card"
                style={{ cursor: 'pointer' }}
                onClick={() => onSelectDoc(item.id)}
              >
                <div className="fw-queue-card-header">
                  <span className="fw-queue-icon">{item.icon}</span>
                  <span className="fw-queue-title">{item.title}</span>
                </div>
                <div className="fw-queue-urgent">{urgencyLabel(item, item.sourceName)}</div>
                <div className="fw-queue-actions" onClick={(e) => e.stopPropagation()}>
                  <button
                    type="button"
                    className="fw-btn fw-btn-verify"
                    onClick={() => onVerify(item.id)}
                  >
                    여전히 유효 ✓
                  </button>
                  <button
                    type="button"
                    className="fw-btn fw-btn-deprecate"
                    onClick={() => onDeprecate(item.id)}
                  >
                    구식 처리
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}

// ---------- source detail sub-component ----------

function SourceDetailView({
  source,
  isSyncing,
  onSync,
  onRetry,
  popover,
  onPopover,
  onVerify,
  onDeprecate,
  selectedDocId,
  onSelectDoc,
}: {
  source: FSource;
  isSyncing: boolean;
  onSync: (id: string) => void;
  onRetry: (docId: string) => void;
  popover: PopoverState | null;
  onPopover: (state: PopoverState | null) => void;
  onVerify: (docId: string) => void;
  onDeprecate: (docId: string) => void;
  selectedDocId: string | null;
  onSelectDoc: (docId: string) => void;
}) {
  return (
    <>
      <div className="fw-detail-header">
        <span className="fw-detail-icon">{source.icon}</span>
        <div className="fw-detail-meta">
          <div className="fw-detail-name">{source.name}</div>
          <div className="fw-detail-sync">● 동기화 {source.lastSync}</div>
        </div>
        <div className="fw-sync-btn-wrap">
          {isSyncing && <span className="fw-spinner" />}
          <button
            type="button"
            className="fw-btn fw-btn-sm"
            disabled={isSyncing}
            onClick={() => onSync(source.id)}
          >
            {isSyncing ? '동기화 중…' : '지금 동기화'}
          </button>
        </div>
      </div>

      <table className="fw-doc-table">
        <thead>
          <tr>
            <th>문서</th>
            <th>인덱싱</th>
            <th>검증</th>
            <th>마지막 사용</th>
          </tr>
        </thead>
        <tbody>
          {source.docs.map((doc) => {
            const isPopoverOpen =
              popover?.docId === doc.id && popover?.sourceId === source.id;
            const isSelected = selectedDocId === doc.id;
            return (
              <tr
                key={doc.id}
                className="fw-doc-row"
                style={{
                  background: isSelected ? 'var(--cw-accent-soft)' : undefined,
                  cursor: 'pointer',
                }}
                onClick={() => onSelectDoc(doc.id)}
              >
                <td>
                  <div className="fw-doc-title-cell">
                    <span className="fw-doc-icon">{doc.icon}</span>
                    <span
                      className={`fw-doc-name${doc.verify === 'deprecated' ? ' fw-deprecated-text' : ''}`}
                    >
                      {doc.title}
                    </span>
                  </div>
                </td>
                <td onClick={(e) => e.stopPropagation()}>
                  <IndexCell doc={doc} onRetry={onRetry} />
                </td>
                <td onClick={(e) => e.stopPropagation()}>
                  <div className="fw-popover-anchor">
                    <BadgeCell doc={doc} sourceId={source.id} onPopover={onPopover} />
                    {isPopoverOpen && (
                      <div className="fw-popover" onClick={(e) => e.stopPropagation()}>
                        <div className="fw-popover-label">검증 상태</div>
                        {doc.expiresAt && (
                          <div className="fw-popover-expiry">만료: {doc.expiresAt}</div>
                        )}
                        <div className="fw-popover-actions">
                          <button
                            type="button"
                            className="fw-popover-btn"
                            onClick={() => { onVerify(doc.id); onPopover(null); }}
                          >
                            ✅ 검증
                          </button>
                          <button
                            type="button"
                            className="fw-popover-btn"
                            onClick={() => { onDeprecate(doc.id); onPopover(null); }}
                          >
                            구식 처리
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                </td>
                <td style={{ color: 'var(--cw-ink-muted)', fontSize: '12px' }}>
                  {doc.lastUsed ?? '—'}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}
