// Workspace candidate C — Cultivation Canvas.
// A card/workbench translation of "Workspace 키우기·가꾸기".

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  INITIAL_CARDS,
  INITIAL_COLLECTIONS,
  INITIAL_LINKS,
  LEGAL_RISK_QUOTE,
  SEEDED_CONFLICT,
  type CcCard,
  type CcCardSource,
  type CcCollection,
  type CcLink,
} from './data';
import './cultivation-canvas.css';

let seq = 1;
const uid = () => `cc-gen-${seq++}`;

type CollectionEvidenceItem = {
  id: string;
  cardId: string;
  collectionId: string;
  title: string;
  sourceLabel: string;
  fieldLabel: string;
  fieldKey?: string;
  quote: string;
  status: 'pending' | 'accepted';
  returnX: number;
  returnY: number;
};

type SourceFilter = 'all' | CcCardSource;
type EvidenceAction = 'drag' | 'quick' | 'bridge' | 'pile';

const SOURCE_LANES: Array<{ id: SourceFilter; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'drive', label: 'Drive' },
  { id: 'gmail', label: 'Gmail' },
  { id: 'session', label: 'Session' },
  { id: 'upload', label: 'Upload' },
];

const INITIAL_COLLECTION_EVIDENCE: CollectionEvidenceItem[] = [
  {
    id: 'seed-contract-term',
    cardId: 'seed-contract-term',
    collectionId: 'contracts',
    title: '신규 서비스 계약서 2026-07.docx',
    sourceLabel: 'Drive',
    fieldLabel: 'Contracts.term',
    fieldKey: 'term',
    quote: '계약 기간 12개월',
    status: 'accepted',
    returnX: 358,
    returnY: 120,
  },
  {
    id: 'seed-meeting-action',
    cardId: 'seed-meeting-action',
    collectionId: 'meetings',
    title: '회의록 6/30',
    sourceLabel: 'Session',
    fieldLabel: 'Meetings.action',
    fieldKey: 'action',
    quote: '계약서 9조 수정안 법무팀 송부 합의.',
    status: 'accepted',
    returnX: 378,
    returnY: 410,
  },
];

function updateContractsRisk(collections: CcCollection[], quote: string, sourceTitle: string) {
  return collections.map((collection) => {
    if (collection.id !== 'contracts') return collection;
    return {
      ...collection,
      records: collection.records.map((record) => {
        if (record.id !== 'contract-b') return record;
        const { risk: _riskGap, ...remainingGaps } = record.gaps ?? {};
        return {
          ...record,
          status: 'approved' as const,
          values: {
            ...record.values,
            risk: '9조 배상 상한 200%',
          },
          provenance: {
            ...record.provenance,
            risk: {
              sourceTitle,
              quote,
            },
          },
          gaps: Object.keys(remainingGaps).length > 0 ? remainingGaps : undefined,
        };
      }),
    };
  });
}

function sourceLabel(source: CcCard['source']) {
  switch (source) {
    case 'drive':
      return 'Drive';
    case 'gmail':
      return 'Gmail';
    case 'slack':
      return 'Slack';
    case 'session':
      return 'Session';
    case 'upload':
      return 'Upload';
  }
}

function pileClaimFor(card: CcCard) {
  if (card.id === 'contract-draft') return '계약서 조항: 9조 배상 상한 200%';
  if (card.id === 'legal-review') return '법무 의견: 100% 또는 150%로 하향 요청';
  if (card.id === 'quote-mail') return '견적 메일: 월 340만, 기존 320만과 충돌';
  if (card.id === 'meeting-note') return '회의 메모: 수정안 송부가 다음 액션';
  return card.body.split('\n').find((line) => line.trim().length > 14)?.trim() ?? card.body.slice(0, 72);
}

function CardBody({ card }: { card: CcCard }) {
  if (card.kind === 'chart') {
    const max = Math.max(...(card.chartValues ?? [1]));
    return (
      <div className="cc-chart-body">
        <div className="cc-chart-bars" aria-hidden="true">
          {(card.chartValues ?? []).map((value, index) => (
            <span
              key={`${card.id}-${value}-${index}`}
              className="cc-chart-bar"
              style={{ height: `${Math.max(18, (value / max) * 78)}px` }}
            />
          ))}
        </div>
        <p>{card.body}</p>
      </div>
    );
  }

  return <div className="cc-card-body">{card.analyzing ? '분석 중...' : card.body}</div>;
}

function CollectionBed({
  collection,
  evidenceItems,
  riskApproved,
  conflictResolved,
  onApproveEvidence,
  onReturnEvidence,
  onOpenConflict,
}: {
  collection: CcCollection;
  evidenceItems: CollectionEvidenceItem[];
  riskApproved: boolean;
  conflictResolved: boolean;
  onApproveEvidence: (itemId: string) => void;
  onReturnEvidence: (itemId: string) => void;
  onOpenConflict: () => void;
}) {
  const pendingItems = evidenceItems.filter((item) => item.status === 'pending');
  const acceptedItems = evidenceItems.filter((item) => item.status === 'accepted');

  return (
    <section className="cc-collection-bed" data-testid={`collection-bed-${collection.id}`}>
      <div className="cc-bed-header">
        <span className="cc-bed-icon">{collection.icon}</span>
        <div>
          <h2>{collection.name}</h2>
          <p>{collection.description}</p>
        </div>
      </div>

      {collection.type === 'index' ? (
        <div className="cc-index-card">
          <strong>{collection.docCount} docs indexed</strong>
          <span>raw material lands here before it becomes typed evidence</span>
        </div>
      ) : (
        <div className="cc-record-list">
          {collection.records.map((record) => (
            <article key={record.id} className={`cc-record-card is-${record.status}`}>
              <div className="cc-record-top">
                <strong>{record.title}</strong>
                <span>{record.status}</span>
              </div>
              <div className="cc-field-grid">
                {collection.fields.map((field) => {
                  const prov = record.provenance[field.key];
                  const gap = record.gaps?.[field.key];
                  return (
                    <div key={field.key} className={`cc-field${gap ? ' has-gap' : ''}`}>
                      <span className="cc-field-label">{field.label}</span>
                      <strong>{record.values[field.key] ?? '-'}</strong>
                      {prov && <em>{prov.sourceTitle}</em>}
                      {gap && <button type="button" className="cc-gap-chip">{gap}</button>}
                    </div>
                  );
                })}
              </div>
              {record.id === 'contract-b' && riskApproved && (
                <div className="cc-gap-cleared">Gap cleared</div>
              )}
            </article>
          ))}
          <div className="cc-evidence-drawer" data-testid={`collection-intake-${collection.id}`}>
            <div className="cc-evidence-section">
              <div className="cc-evidence-section-title">
                <span>승인 대기</span>
                <strong>{pendingItems.length}</strong>
              </div>
              {pendingItems.length === 0 ? (
                <p className="cc-evidence-empty">새로 들어온 근거 없음</p>
              ) : (
                pendingItems.map((item) => (
                  <article
                    key={item.id}
                    className="cc-evidence-item is-pending"
                    data-testid={`evidence-pending-${item.cardId}`}
                  >
                    <div>
                      <strong>{item.title}</strong>
                      <span>{item.sourceLabel} · {item.fieldLabel}</span>
                    </div>
                    <p>{item.quote}</p>
                    <div className="cc-evidence-actions">
                      <button type="button" onClick={() => onApproveEvidence(item.id)}>승인</button>
                      <button type="button" onClick={() => onReturnEvidence(item.id)}>되돌리기</button>
                    </div>
                  </article>
                ))
              )}
            </div>

            {acceptedItems.length > 0 && (
              <div className="cc-evidence-section">
                <div className="cc-evidence-section-title">
                  <span>이미 심어진 근거</span>
                  <strong>{acceptedItems.length}</strong>
                </div>
                <div className="cc-evidence-accepted-list">
                  {acceptedItems.map((item) => (
                    <div
                      key={item.id}
                      className="cc-evidence-chip"
                      data-testid={`evidence-accepted-${item.cardId}`}
                    >
                      <span>{item.title}</span>
                      <em>{item.fieldLabel}</em>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {collection.id === 'contracts' && !conflictResolved && (
        <div className="cc-conflict-chip" data-testid="conflict-chip">
          <span>충돌: 월 320만 vs 340만</span>
          <button type="button" onClick={onOpenConflict}>충돌 열기</button>
        </div>
      )}
    </section>
  );
}

export function CultivationCanvasWorkspace() {
  const [cards, setCards] = useState<CcCard[]>(INITIAL_CARDS);
  const [collections, setCollections] = useState<CcCollection[]>(INITIAL_COLLECTIONS);
  const [links, setLinks] = useState<CcLink[]>(INITIAL_LINKS);
  const [collectionEvidence, setCollectionEvidence] = useState<CollectionEvidenceItem[]>(INITIAL_COLLECTION_EVIDENCE);
  const [pileCardIds, setPileCardIds] = useState<string[]>([]);
  const [pileProposalCreated, setPileProposalCreated] = useState(false);
  const [lastEvidenceAction, setLastEvidenceAction] = useState<EvidenceAction | null>(null);
  const [libSearch, setLibSearch] = useState('');
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>('all');
  const [collectionsCollapsed, setCollectionsCollapsed] = useState(false);
  const [replayActive, setReplayActive] = useState(false);
  const [replayHidden, setReplayHidden] = useState<Set<string>>(new Set());
  const [activeCardId, setActiveCardId] = useState<string | null>(null);
  const [hoverCollectionId, setHoverCollectionId] = useState<string | null>(null);
  const [dropActive, setDropActive] = useState(false);
  const [conflictOpen, setConflictOpen] = useState(false);
  const [conflictResolved, setConflictResolved] = useState(false);
  const [riskApproved, setRiskApproved] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const dropzoneRef = useRef<HTMLDivElement | null>(null);
  const timers = useRef<number[]>([]);

  const collectedCardIds = useMemo(() => {
    const realCardIds = new Set(cards.map((card) => card.id));
    return new Set(
      collectionEvidence
        .filter((item) => realCardIds.has(item.cardId))
        .map((item) => item.cardId),
    );
  }, [cards, collectionEvidence]);
  const pileCardIdSet = useMemo(() => new Set(pileCardIds), [pileCardIds]);
  const pileCards = useMemo(
    () => pileCardIds.map((id) => cards.find((card) => card.id === id)).filter((card): card is CcCard => Boolean(card)),
    [cards, pileCardIds],
  );
  const visibleCards = useMemo(
    () => cards.filter((card) => !collectedCardIds.has(card.id) && !pileCardIdSet.has(card.id)),
    [cards, collectedCardIds, pileCardIdSet],
  );
  const pendingEvidence = useMemo(
    () => collectionEvidence.filter((item) => item.status === 'pending'),
    [collectionEvidence],
  );
  const orderedCollections = useMemo(() => {
    const order = new Map([
      ['contracts', 0],
      ['knowledge-index', 1],
      ['crm', 2],
      ['meetings', 3],
    ]);
    return [...collections].sort((a, b) => (order.get(a.id) ?? 99) - (order.get(b.id) ?? 99));
  }, [collections]);
  const latestPendingEvidence = pendingEvidence[pendingEvidence.length - 1];
  const placedCards = useMemo(() => visibleCards.filter((card) => card.placed), [visibleCards]);
  const sourceCounts = useMemo(() => {
    const counts = new Map<SourceFilter, number>([['all', visibleCards.length]]);
    for (const lane of SOURCE_LANES) {
      if (lane.id !== 'all') counts.set(lane.id, visibleCards.filter((card) => card.source === lane.id).length);
    }
    return counts;
  }, [visibleCards]);
  const trayCards = useMemo(() => {
    const query = libSearch.trim().toLowerCase();
    return visibleCards.filter((card) => {
      if (card.placed) return false;
      if (sourceFilter !== 'all' && card.source !== sourceFilter) return false;
      if (!query) return true;
      return `${card.title} ${card.origin} ${card.body}`.toLowerCase().includes(query);
    });
  }, [visibleCards, libSearch, sourceFilter]);
  const activeCard = cards.find((card) => card.id === activeCardId);
  const bridgeCard = activeCard?.placed && !collectedCardIds.has(activeCard.id)
    ? activeCard
    : placedCards.find((card) => card.targetCollectionId) ?? placedCards[0];
  const bridgeCollection = collections.find(
    (collection) => collection.id === (latestPendingEvidence?.collectionId ?? bridgeCard?.targetCollectionId ?? 'contracts'),
  );
  const activeCollectionId = hoverCollectionId ?? latestPendingEvidence?.collectionId ?? null;

  function showToast(message: string) {
    setToast(message);
    timers.current.push(window.setTimeout(() => setToast(null), 2400));
  }

  useEffect(() => () => timers.current.forEach((timer) => window.clearTimeout(timer)), []);

  function placeCard(cardId: string, clientX?: number, clientY?: number) {
    const rect = dropzoneRef.current?.getBoundingClientRect();
    const nextX = rect && clientX ? clientX - rect.left - 150 : 280;
    const nextY = rect && clientY ? clientY - rect.top - 90 : 180;
    setCards((prev) =>
      prev.map((card) =>
        card.id === cardId
          ? { ...card, placed: true, x: Math.max(24, nextX), y: Math.max(80, nextY) }
          : card,
      ),
    );
    showToast('자료를 작업대에 올렸어요');
  }

  function moveCard(cardId: string, clientX?: number, clientY?: number) {
    const rect = dropzoneRef.current?.getBoundingClientRect();
    if (!rect || !clientX || !clientY) return;
    setActiveCardId(cardId);
    setCards((prev) =>
      prev.map((card) =>
        card.id === cardId
          ? {
              ...card,
              x: Math.max(18, clientX - rect.left - (card.width ?? 300) / 2),
              y: Math.max(72, clientY - rect.top - 34),
            }
          : card,
      ),
    );
  }

  function nudgeCard(cardId: string) {
    setActiveCardId(cardId);
    setCards((prev) =>
      prev.map((card) =>
        card.id === cardId
          ? { ...card, x: card.x + 96, y: card.y + 12 }
          : card,
      ),
    );
  }

  function returnToTray(cardId: string) {
    setCards((prev) =>
      prev
        .filter((card) => !card.id.startsWith(`ai-bubble-${cardId}`))
        .map((card) => (card.id === cardId ? { ...card, placed: false, x: 0, y: 0 } : card)),
    );
    setLinks((prev) => prev.filter((link) => link.from !== cardId && link.to !== cardId));
    showToast('카드를 Material Tray로 되돌렸어요');
  }

  function addCardToPile(cardId: string) {
    const source = cards.find((card) => card.id === cardId);
    if (!source) return;
    setPileCardIds((prev) => (prev.includes(cardId) ? prev : [...prev, cardId]));
    setActiveCardId(cardId);
    showToast('검토 묶음에 자료를 더했어요');
  }

  function handlePileDrop(event: React.DragEvent<HTMLDivElement>) {
    event.preventDefault();
    event.stopPropagation();
    const cardId =
      event.dataTransfer.getData('application/x-cultivation-move') ||
      event.dataTransfer.getData('application/x-cultivation-card');
    if (cardId) addCardToPile(cardId);
  }

  function returnPileCard(cardId: string) {
    const source = cards.find((card) => card.id === cardId);
    setPileCardIds((prev) => prev.filter((id) => id !== cardId));
    if (source && !source.placed) {
      setCards((prev) =>
        prev.map((card) => (card.id === cardId ? { ...card, placed: true, x: 320, y: 220 } : card)),
      );
    }
    showToast('자료를 검토 묶음에서 작업대로 꺼냈어요');
  }

  function handleDragStart(event: React.DragEvent, cardId: string) {
    event.dataTransfer.setData('application/x-cultivation-card', cardId);
    event.dataTransfer.effectAllowed = 'copy';
  }

  function handleCardMoveStart(event: React.DragEvent, cardId: string) {
    setActiveCardId(cardId);
    event.dataTransfer.setData('application/x-cultivation-move', cardId);
    event.dataTransfer.effectAllowed = 'move';
  }

  function handleDrop(event: React.DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDropActive(false);
    const moveCardId = event.dataTransfer.getData('application/x-cultivation-move');
    if (moveCardId) {
      moveCard(moveCardId, event.clientX, event.clientY);
      return;
    }

    const cardId = event.dataTransfer.getData('application/x-cultivation-card');
    if (cardId) {
      placeCard(cardId, event.clientX, event.clientY);
      return;
    }

    if (event.dataTransfer.files.length > 0) {
      const file = event.dataTransfer.files[0];
      const rect = dropzoneRef.current?.getBoundingClientRect();
      const x = rect ? event.clientX - rect.left - 150 : 240;
      const y = rect ? event.clientY - rect.top - 90 : 220;
      const nextCard: CcCard = {
        id: 'file-drop',
        kind: 'material',
        source: 'upload',
        icon: '📄',
        title: file.name,
        origin: 'Dropped file',
        body: '분석 중...',
        placed: true,
        x: Math.max(24, x),
        y: Math.max(80, y),
        width: 320,
        analyzing: true,
      };
      setCards((prev) => [...prev.filter((card) => card.id !== 'file-drop'), nextCard]);
      window.setTimeout(() => {
        setCards((prev) =>
          prev.map((card) =>
            card.id === 'file-drop'
              ? {
                  ...card,
                  analyzing: false,
                  title: '보안 감사 요약',
                  body:
                    '보안 감사 요약\n\n' +
                    '접근 권한 감사 결과 이상 없음. 퇴직자 계정 정리 완료. 다음 감사는 2026년 하반기 예정.',
                }
              : card,
          ),
        );
      }, 650);
    }
  }

  function startReplay() {
    if (replayActive) return;
    const ids = placedCards.map((card) => card.id);
    setReplayActive(true);
    setReplayHidden(new Set(ids));
    ids.forEach((id, index) => {
      timers.current.push(window.setTimeout(() => {
        setReplayHidden((prev) => {
          const next = new Set(prev);
          next.delete(id);
          return next;
        });
      }, 160 + index * 120));
    });
    timers.current.push(window.setTimeout(() => setReplayActive(false), 900 + ids.length * 120));
  }

  function fieldLabelFor(card: CcCard, collectionId: string) {
    const collection = collections.find((item) => item.id === collectionId);
    const key = card.targetFieldKey ?? (collectionId === 'contracts' ? 'risk' : undefined);
    const field = collection?.fields.find((item) => item.key === key);
    return `${collection?.name ?? 'Collection'}.${field?.key ?? 'record'}`;
  }

  function evidenceQuoteFor(card: CcCard) {
    if (card.id === 'legal-review') return LEGAL_RISK_QUOTE;
    if (card.id === 'contract-draft') {
      return '제9조 손해배상 책임 총액은 최근 3개월 대금의 100분의 200을 상한으로 한다.';
    }
    if (card.id === 'quote-mail') {
      return '최종 견적은 월 340만 원이며, 월간 트래픽 10TB 초과분은 GB당 120원으로 산정됩니다.';
    }
    return card.body.split('\n').find((line) => line.trim().length > 16)?.trim() ?? card.body.slice(0, 90);
  }

  function sendCardToCollection(cardId: string, collectionId: string, action: EvidenceAction = 'drag') {
    const source = cards.find((card) => card.id === cardId);
    if (!source) return;
    const targetCollection = collections.find((collection) => collection.id === collectionId);
    const fieldKey = source.targetFieldKey ?? (collectionId === 'contracts' ? 'risk' : undefined);
    const quote = evidenceQuoteFor(source);
    const evidenceItem: CollectionEvidenceItem = {
      id: uid(),
      cardId,
      collectionId,
      title: source.title,
      sourceLabel: sourceLabel(source.source),
      fieldLabel: fieldLabelFor(source, collectionId),
      fieldKey,
      quote,
      status: 'pending',
      returnX: source.placed ? source.x : 320,
      returnY: source.placed ? source.y : 220,
    };
    setActiveCardId(cardId);
    setHoverCollectionId(collectionId);
    setLastEvidenceAction(action);
    setCollectionEvidence((prev) => [
      ...prev.filter((item) => item.cardId !== cardId || item.status !== 'pending'),
      evidenceItem,
    ]);
    showToast(`${targetCollection?.name ?? 'Collection'} 근거함으로 보냈어요`);
  }

  function handleCollectionDrop(event: React.DragEvent<HTMLDivElement>, collectionId: string) {
    event.preventDefault();
    event.stopPropagation();
    const cardId =
      event.dataTransfer.getData('application/x-cultivation-move') ||
      event.dataTransfer.getData('application/x-cultivation-card');
    if (cardId) sendCardToCollection(cardId, collectionId);
  }

  function createPileProposal() {
    if (pileCards.length < 2) return;
    setPileProposalCreated(true);
    setLastEvidenceAction('pile');
    showToast('여러 자료를 하나의 Contracts.risk 제안으로 묶었어요');
  }

  function approvePileProposal() {
    if (!pileProposalCreated) return;
    const sourceTitles = pileCards.map((card) => card.title);
    setCollectionEvidence((prev) => [
      ...prev.filter((item) => item.cardId !== 'pile-contract-risk'),
      {
        id: uid(),
        cardId: 'pile-contract-risk',
        collectionId: 'contracts',
        title: 'B사 계약 리스크 검토',
        sourceLabel: 'Canvas pile',
        fieldLabel: 'Contracts.risk',
        fieldKey: 'risk',
        quote: `계약서 조항, 법무 의견, 견적 메일을 함께 검토한 복합 근거 3개: ${sourceTitles.join(' / ')}`,
        status: 'accepted',
        returnX: 0,
        returnY: 0,
      },
    ]);
    setCollections((prev) => updateContractsRisk(prev, LEGAL_RISK_QUOTE, '복합 근거 3개 · B사 계약 리스크 검토'));
    setRiskApproved(true);
    setLastEvidenceAction('pile');
    showToast('Contracts.risk에 복합 근거를 심었어요');
  }

  function approveEvidence(itemId: string) {
    const item = collectionEvidence.find((entry) => entry.id === itemId);
    if (!item) return;
    setCollectionEvidence((prev) =>
      prev.map((entry) => (entry.id === itemId ? { ...entry, status: 'accepted' } : entry)),
    );
    if (item.collectionId === 'contracts' && item.fieldKey === 'risk') {
      setCollections((prev) => updateContractsRisk(prev, item.quote, item.title));
      setRiskApproved(true);
    }
    showToast(`${item.fieldLabel}에 근거를 심었어요`);
  }

  function returnEvidence(itemId: string) {
    const item = collectionEvidence.find((entry) => entry.id === itemId);
    if (!item) return;
    setCollectionEvidence((prev) => prev.filter((entry) => entry.id !== itemId));
    setCards((prev) =>
      prev.map((card) =>
        card.id === item.cardId
          ? { ...card, placed: true, x: item.returnX, y: item.returnY }
          : card,
      ),
    );
    setHoverCollectionId(null);
    setActiveCardId(item.cardId);
    showToast('카드를 작업대로 되돌렸어요');
  }

  function resolveConflict() {
    setConflictResolved(true);
    setConflictOpen(false);
    showToast('Conflict resolved');
  }

  useEffect(() => {
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setConflictOpen(false);
      }
    };
    window.addEventListener('keydown', onEscape);
    return () => window.removeEventListener('keydown', onEscape);
  }, []);

  return (
    <div className={`cc-root${collectionsCollapsed ? ' has-collapsed-collections' : ''}`}>
      <aside className="cc-material-tray" data-testid="material-tray">
        <div className="cc-pane-head">
          <div>
            <div className="cc-panel-kicker">Material Tray</div>
            <h1>가꾸기 캔버스</h1>
          </div>
          <button type="button" className="cc-icon-btn" onClick={startReplay}>성장 리플레이</button>
        </div>
        <p className="cc-tray-copy">멀티소스 자료를 작업대에 올리고, 필요한 카드를 collection 근거함으로 보낸 뒤 승인합니다.</p>
        <div className="cc-source-lanes" aria-label="connected material sources">
          {SOURCE_LANES.map((lane) => (
            <button
              key={lane.id}
              type="button"
              className={`cc-source-lane${sourceFilter === lane.id ? ' is-active' : ''}`}
              data-testid={`source-lane-${lane.id}`}
              onClick={() => setSourceFilter(lane.id)}
            >
              <span>{lane.label}</span>
              <strong>{sourceCounts.get(lane.id) ?? 0}</strong>
            </button>
          ))}
        </div>
        <input
          className="cc-search"
          placeholder="자료 검색"
          value={libSearch}
          onChange={(event) => setLibSearch(event.target.value)}
        />
        {replayActive && (
          <div className="cc-replay-status" data-testid="growth-replay-status">
            자료가 workspace에 다시 심어지는 중
          </div>
        )}

        <div className="cc-tray-list">
          {trayCards.map((card) => (
            <article
              key={card.id}
              draggable
              className="cc-tray-item"
              data-testid={`tray-item-${card.id}`}
              onDragStart={(event) => handleDragStart(event, card.id)}
            >
              <span>{card.icon}</span>
              <div className="cc-tray-main">
                <strong>{card.title}</strong>
                <em>{sourceLabel(card.source)} · drag to workbench</em>
              </div>
              <div className="cc-tray-actions">
                <button type="button" onClick={() => placeCard(card.id)}>작업대에 올리기</button>
                <button type="button" onClick={() => sendCardToCollection(card.id, 'contracts', 'quick')}>
                  빠른 심기
                </button>
              </div>
            </article>
          ))}
        </div>
      </aside>

      <main
        ref={dropzoneRef}
        className={`cc-workbench${dropActive ? ' is-drop-active' : ''}`}
        data-testid="canvas-dropzone"
        onDragEnter={(event) => {
          event.preventDefault();
          setDropActive(true);
        }}
        onDragOver={(event) => event.preventDefault()}
        onDragLeave={(event) => {
          if (event.currentTarget === event.target) setDropActive(false);
        }}
        onDrop={handleDrop}
      >
        <div className="cc-workbench-top">
          <div>
            <span className="cc-panel-kicker">Cultivation Canvas</span>
            <h2>Raw Material → Evidence Intake → Record</h2>
          </div>
          <div className="cc-workbench-top-right">
            <div className="cc-flow-strip" aria-label="workspace cultivation flow">
              <span>Raw Material</span>
              <span>Evidence Intake</span>
              <span>Evidence</span>
              <span>Collection Record</span>
            </div>
            <div className={`cc-interaction-guide${activeCollectionId ? ' is-active' : ''}`} data-testid="interaction-guide">
              {latestPendingEvidence
                ? `${lastEvidenceAction === 'quick' ? '빠른 심기 → ' : ''}${bridgeCollection?.name ?? 'Collection'} 근거함에서 승인 대기 중 · 승인 또는 되돌리기`
                : activeCollectionId
                  ? `${bridgeCollection?.name ?? 'Collection'} 근거함으로 들어갑니다`
                  : pileProposalCreated
                    ? '캔버스 pile에서 만든 복합 proposal을 승인하면 record provenance가 더 풍부해집니다'
                    : '단일 근거는 빠른 심기, 여러 자료 판단은 캔버스 pile에서 조합하세요'}
            </div>
          </div>
        </div>

        <div className="cc-collection-bridge" data-testid="collection-bridge">
          <span>{latestPendingEvidence ? '승인 대기' : '근거함으로 보내기 전'}</span>
          <strong>
            {latestPendingEvidence && bridgeCollection
              ? `${latestPendingEvidence.title} → ${bridgeCollection.name} 근거함`
              : bridgeCard && bridgeCollection
              ? `${bridgeCard.title} → ${bridgeCollection.name}`
              : '작업대 카드 → 컬렉션 근거함'}
          </strong>
          <em>
            {latestPendingEvidence
              ? '원본 카드는 작업대에서 빠졌고, 오른쪽 컬렉션 안에서 승인 또는 되돌리기를 선택합니다.'
              : '보내면 원본 카드는 작업대에서 빠지고 컬렉션의 승인 대기 근거로 이동합니다.'}
          </em>
          {bridgeCard && bridgeCollection && (
            <button type="button" onClick={() => sendCardToCollection(bridgeCard.id, bridgeCollection.id, 'bridge')}>
              {bridgeCollection.name} 근거함으로 보내기
            </button>
          )}
        </div>

        <section
          className={`cc-synthesis-pile${pileCards.length > 0 ? ' has-items' : ''}`}
          data-testid="synthesis-pile"
          onDragOver={(event) => event.preventDefault()}
          onDrop={handlePileDrop}
        >
          <div className="cc-pile-head">
            <div>
              <span className="cc-panel-kicker">Canvas-only work</span>
              <h3>B사 계약 리스크 검토</h3>
            </div>
            <strong>{pileCards.length}/3 sources</strong>
          </div>
          <p className="cc-pile-copy">
            캔버스에서만 여러 source를 한 판단 단위로 모아 단일 근거보다 풍부한 proposal을 만듭니다.
          </p>
          <div className="cc-pile-items">
            {pileCards.length === 0 ? (
              <div className="cc-pile-empty">계약서, 견적 메일, 법무 의견서를 끌어오거나 카드의 묶음 버튼을 누르세요.</div>
            ) : (
              pileCards.map((card) => (
                <article key={card.id} className="cc-pile-mini-card" data-testid={`pile-item-${card.id}`}>
                  <span>{card.icon}</span>
                  <div>
                    <strong>{card.title}</strong>
                    <em>{pileClaimFor(card)}</em>
                  </div>
                  <button type="button" onClick={() => returnPileCard(card.id)}>꺼내기</button>
                </article>
              ))
            )}
          </div>
          <button
            type="button"
            className="cc-pile-primary"
            disabled={pileCards.length < 2}
            onClick={createPileProposal}
          >
            {pileCards.length}개 자료로 제안 만들기
          </button>
          {pileProposalCreated && (
            <article className="cc-synthesis-proposal" data-testid="synthesis-proposal">
              <span>Contracts.risk 복합 제안</span>
              <strong>계약서 조항 + 법무 의견 + 견적 메일을 함께 반영</strong>
              <p>계약서 조항은 200% 상한, 법무 의견은 100% 또는 150% 하향 요청, 견적 메일은 월 340만 비용 충돌을 제공합니다.</p>
              <button type="button" onClick={approvePileProposal}>복합 근거 승인</button>
            </article>
          )}
        </section>

        <svg className="cc-link-layer" aria-hidden="true">
          {links.map((link) => {
            const from = cards.find((card) => card.id === link.from);
            const to = cards.find((card) => card.id === link.to);
            if (!from?.placed || !to?.placed) return null;
            return (
              <line
                key={link.id}
                className={`cc-link is-${link.kind}`}
                x1={from.x + (from.width ?? 300)}
                y1={from.y + 56}
                x2={to.x}
                y2={to.y + 56}
              />
            );
          })}
        </svg>

        {placedCards.map((card, index) => (
          <article
            key={card.id}
            className={`cc-card is-${card.kind}${replayHidden.has(card.id) ? ' is-replay-hidden' : ''}`}
            data-testid={card.id.startsWith('ai-bubble-') ? card.id : `card-${card.id}`}
            style={{
              left: card.x,
              top: card.y,
              width: card.width ?? 300,
              zIndex: activeCardId === card.id ? 32 : card.id.startsWith('ai-bubble-') ? 18 : 6 + index,
            }}
            onMouseDown={() => setActiveCardId(card.id)}
          >
            <div
              className="cc-card-header"
              data-testid="card-drag-handle"
              draggable
              onDragStart={(event) => handleCardMoveStart(event, card.id)}
            >
              <span>{card.icon}</span>
              <div className="cc-card-title">
                <strong>{card.title}</strong>
                <em>{card.origin}</em>
              </div>
              {card.targetCollectionId && (
                <span className="cc-card-target">
                  → {card.targetCollectionId}.{card.targetFieldKey ?? 'record'}
                </span>
              )}
              {!card.id.startsWith('ai-bubble-') && (
                <div className="cc-card-actions">
                  <button
                    type="button"
                    className="cc-card-move"
                    aria-label="카드 오른쪽으로 이동"
                    onClick={(event) => {
                      event.stopPropagation();
                      nudgeCard(card.id);
                    }}
                  >
                    이동
                  </button>
                  <button
                    type="button"
                    className="cc-card-pile"
                    aria-label="검토 묶음에 추가"
                    onClick={(event) => {
                      event.stopPropagation();
                      addCardToPile(card.id);
                    }}
                  >
                    묶음
                  </button>
                  <button
                    type="button"
                    className="cc-card-remove"
                    aria-label="작업대에서 빼기"
                    onClick={(event) => {
                      event.stopPropagation();
                      returnToTray(card.id);
                    }}
                  >
                    빼기
                  </button>
                </div>
              )}
            </div>
            <CardBody card={card} />
          </article>
        ))}

        {dropActive && (
          <div className="cc-file-drop-overlay" data-testid="file-drop-overlay">
            <strong>파일을 놓으면 workspace 재료로 분석합니다</strong>
          </div>
        )}
      </main>

      <aside className={`cc-collections${collectionsCollapsed ? ' is-collapsed' : ''}`} data-testid="collection-panel">
        <div className="cc-collections-head">
          <div>
            <div className="cc-panel-kicker">Collection Beds</div>
            <strong>컬렉션 타깃</strong>
          </div>
          <button
            type="button"
            className="cc-icon-btn"
            onClick={() => setCollectionsCollapsed((value) => !value)}
            aria-label={collectionsCollapsed ? '컬렉션 패널 펼치기' : '컬렉션 패널 접기'}
          >
            {collectionsCollapsed ? '열기' : '접기'}
          </button>
        </div>
        {!collectionsCollapsed && orderedCollections.map((collection) => (
          <div
            key={collection.id}
            data-testid={`collection-target-${collection.id}`}
            className={`cc-collection-target${activeCollectionId === collection.id ? ' is-targeted' : ''}${hoverCollectionId === collection.id ? ' is-drop-target' : ''}`}
            onDragEnter={(event) => {
              event.preventDefault();
              setHoverCollectionId(collection.id);
            }}
            onDragOver={(event) => {
              event.preventDefault();
              setHoverCollectionId(collection.id);
            }}
            onDragLeave={(event) => {
              if (event.currentTarget === event.target) setHoverCollectionId(null);
            }}
            onDrop={(event) => handleCollectionDrop(event, collection.id)}
          >
            <CollectionBed
              collection={collection}
              evidenceItems={collectionEvidence.filter((item) => item.collectionId === collection.id)}
              riskApproved={riskApproved}
              conflictResolved={conflictResolved}
              onApproveEvidence={approveEvidence}
              onReturnEvidence={returnEvidence}
              onOpenConflict={() => setConflictOpen(true)}
            />
          </div>
        ))}
        {collectionsCollapsed && (
          <div className="cc-collapsed-targets" aria-hidden="true">
            {orderedCollections.map((collection) => <span key={collection.id}>{collection.icon}</span>)}
          </div>
        )}
      </aside>

      {conflictOpen && (
        <div className="cc-modal-backdrop" onClick={() => setConflictOpen(false)}>
          <section className="cc-conflict-panel" data-testid="conflict-panel" onClick={(e) => e.stopPropagation()}>
            <span className="cc-panel-kicker">Conflict</span>
            <h2>{SEEDED_CONFLICT.fieldLabel}</h2>
            <div className="cc-conflict-quotes">
              <blockquote>
                <strong>{SEEDED_CONFLICT.latestSource}</strong>
                <p>{SEEDED_CONFLICT.latestQuote}</p>
              </blockquote>
              <blockquote>
                <strong>{SEEDED_CONFLICT.olderSource}</strong>
                <p>{SEEDED_CONFLICT.olderQuote}</p>
              </blockquote>
            </div>
            <button type="button" onClick={resolveConflict}>최신 근거 채택</button>
          </section>
        </div>
      )}

      {toast && <div className="cc-toast">{toast}</div>}
    </div>
  );
}
