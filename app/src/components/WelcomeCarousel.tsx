import { useCallback, useEffect, useRef, useState } from 'react';
import { Icon, type IconName } from '@/components/Icon';

type MockKind = 'agent' | 'knowledge' | 'artifact' | 'share' | 'automation' | 'quickstart';

interface Slide {
  key: string;
  eyebrow: string;
  title: string;
  blurb: string;
  detail: string;
  mock: MockKind;
  // 실제 시연 영상을 넣으려면 app/public/welcome/<파일>.mp4 를 두고 경로를 적으면
  // 목업 대신 자동으로 재생됩니다. 예: video: '/welcome/routing.mp4'
  video?: string;
  poster?: string;
}

// "많이 만든 다음 덜어내기" — 우선 풍부하게. 슬라이드를 빼거나 합치며 줄이면 됩니다.
const SLIDES: Slide[] = [
  {
    key: 'agent',
    eyebrow: '01 · 에이전트',
    title: '원하는 에이전트로',
    blurb: '새 세션을 시작할 때 에이전트를 고릅니다.',
    detail: '작업의 성격에 맞는 에이전트를 선택해 요청을 보냅니다. 빠른 검색, 코드 실행, 멀티 페이지 심층 조사까지. 골라 둔 에이전트가 그 대화를 책임지고 처리합니다.',
    mock: 'agent',
  },
  {
    key: 'knowledge',
    eyebrow: '02 · 자료',
    title: '내 문서로 답하기',
    blurb: '파일을 올려두면 출처와 함께 답합니다.',
    detail: 'PDF·문서·코드를 프로젝트에 올려두면 에이전트가 그 자료를 근거로 답합니다. 답변에는 어느 파일에서 가져왔는지 출처가 함께 표시되어 그대로 검토할 수 있습니다.',
    mock: 'knowledge',
  },
  {
    key: 'artifact',
    eyebrow: '03 · 산출물',
    title: '결과물을 바로 손에',
    blurb: '보고서·코드·문서를 바로 받습니다.',
    detail: '에이전트가 생성한 산출물을 내려받거나, 클릭 한 번으로 팀 공유 폴더로 옮겨 함께 이어갑니다.',
    mock: 'artifact',
  },
  {
    key: 'share',
    eyebrow: '04 · 협업',
    title: '함께 대화하는 세션',
    blurb: '공개 범위를 골라 함께 묻습니다.',
    detail: '비공개·읽기 공유·함께 대화 모드로 팀원이 한 세션에서 같은 AI와 실시간 협업합니다. 같은 자료, 같은 답을 함께 검토합니다.',
    mock: 'share',
  },
  {
    key: 'automation',
    eyebrow: '05 · 자동화',
    title: '사람 없이도 알아서',
    blurb: '스케줄이나 웹훅으로 자동 실행됩니다.',
    detail: '매일 아침 리포트, 외부 알람이 트리거하는 분석처럼 반복되거나 이벤트에 반응해야 하는 작업을 등록해 두면 결과만 쌓입니다. 자동화 세션은 일반 대화와 분리되어 따로 관리됩니다.',
    mock: 'automation',
  },
  {
    key: 'quickstart',
    eyebrow: '시작하기',
    title: '3단계로 시작하세요',
    blurb: '프로젝트 → 대화 → 공유.',
    detail: '워크스페이스를 만들고, 원하는 작업을 입력하면 에이전트가 처리합니다. 공유 모드로 팀과 이어 다듬으세요.',
    mock: 'quickstart',
  },
];

const AUTO_ADVANCE_MS = 7000;

function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined'
    && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

export function WelcomeCarousel() {
  const trackRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const activeRef = useRef(0);
  activeRef.current = active;
  const reduced = prefersReducedMotion();

  // WCAG 2.2.2 (Pause, Stop, Hide) — 자동 전환은 다음 세 가지 중 하나라도
  // 켜지면 멈춘다. hover는 사용자 요구로 제외 ("호버 시 진행바가 멈추면 안 됨").
  //  · userPaused: 명시적 pause/play 토글
  //  · focusedWithin: 키보드 사용자가 carousel 내부로 focus 진입 (mouse hover와 다른 시나리오)
  //  · pageHidden: 탭이 background로 숨겨짐
  const [userPaused, setUserPaused] = useState(false);
  const [focusedWithin, setFocusedWithin] = useState(false);
  const [pageHidden, setPageHidden] = useState(
    typeof document !== 'undefined' ? document.hidden : false,
  );

  const scrollToIndex = useCallback((i: number) => {
    const track = trackRef.current;
    if (!track) return;
    track.scrollTo({ left: i * track.clientWidth, behavior: reduced ? 'auto' : 'smooth' });
  }, [reduced]);

  // activeIndex는 스크롤 위치에서 파생 — 스와이프/자동전환이 한 방향으로만 흐른다.
  useEffect(() => {
    const track = trackRef.current;
    if (!track) return;
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const i = Math.round(track.scrollLeft / track.clientWidth);
        if (i !== activeRef.current) setActive(i);
      });
    };
    track.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      track.removeEventListener('scroll', onScroll);
      cancelAnimationFrame(frame);
    };
  }, []);

  // page hidden 추적 — 탭이 background에 있으면 자동 전환이 시각 없는 사용자에게
  // 의미 없는 변화를 만들지 않게 한다.
  useEffect(() => {
    const onVis = () => setPageHidden(document.hidden);
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  }, []);

  const isPaused = userPaused || focusedWithin || pageHidden;

  // 자동 전환 — 슬라이드가 바뀌면 다음 카운트를 새로 시작. isPaused가 false인 동안에만.
  useEffect(() => {
    if (reduced || isPaused) return;
    const t = window.setTimeout(() => {
      scrollToIndex((activeRef.current + 1) % SLIDES.length);
    }, AUTO_ADVANCE_MS);
    return () => window.clearTimeout(t);
  }, [reduced, isPaused, scrollToIndex, active]);

  const go = useCallback((i: number) => {
    scrollToIndex((i + SLIDES.length) % SLIDES.length);
  }, [scrollToIndex]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowRight') { e.preventDefault(); go(active + 1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); go(active - 1); }
  };

  // focus가 container 안에서 옮겨다닐 때(arrow → dot 등)는 blur로 보지 않는다.
  const onBlurCapture = (e: React.FocusEvent) => {
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    setFocusedWithin(false);
  };

  return (
    <div
      className="cw-welcome-carousel"
      role="region"
      aria-roledescription="carousel"
      aria-label="제품 소개"
      tabIndex={0}
      onKeyDown={onKeyDown}
      onFocus={(e) => {
        // 마우스 클릭으로 인한 focus는 무시. 키보드(Tab) 진입만 pause trigger —
        // 사용자가 carousel을 "살펴보는" 시나리오는 키보드 사용자 한정이고, pause
        // 버튼을 마우스로 누른 직후 그 button이 focus되는 false positive를 피한다.
        if ((e.target as HTMLElement).matches(':focus-visible')) {
          setFocusedWithin(true);
        }
      }}
      onBlur={onBlurCapture}
    >
      <div className="cw-welcome-track cw-scroll-quiet" ref={trackRef}>
        {SLIDES.map((s, i) => (
          <section
            key={s.key}
            className="cw-welcome-slide"
            aria-roledescription="slide"
            aria-label={`${i + 1} / ${SLIDES.length} — ${s.title}`}
            aria-hidden={i !== active}
          >
            <div className="cw-welcome-media">
              {s.video ? (
                <video
                  className="cw-welcome-video"
                  src={s.video}
                  poster={s.poster}
                  autoPlay
                  muted
                  loop
                  playsInline
                />
              ) : (
                <SlideMock kind={s.mock} active={i === active} />
              )}
            </div>
            <div className="cw-welcome-copy">
              <span className="cw-welcome-eyebrow">{s.eyebrow}</span>
              <h1 className="cw-welcome-headline">{s.title}</h1>
              <p className="cw-welcome-blurb">{s.blurb}</p>
              <p className="cw-welcome-detail">{s.detail}</p>
            </div>
          </section>
        ))}
      </div>

      <div className="cw-welcome-controls">
        <button
          type="button"
          className="cw-welcome-arrow"
          aria-label="이전"
          onClick={() => go(active - 1)}
        >
          <Icon name="chevron-left" size={18} />
        </button>

        <div className="cw-welcome-dots" role="tablist" aria-label="슬라이드 선택">
          {SLIDES.map((s, i) => (
            <button
              key={s.key}
              type="button"
              role="tab"
              aria-selected={i === active}
              aria-label={`${i + 1}번 슬라이드: ${s.title}`}
              className="cw-welcome-dot"
              data-active={i === active}
              onClick={() => go(i)}
            >
              <span className="cw-welcome-dot-track">
                {/* key로 fill element를 강제 remount해 CSS animation을 새로
                   시작시킨다. active 슬라이드가 바뀌거나 isPaused 토글이
                   풀리면 자동 전환 setTimeout이 새 7초로 reset되는데, 진행바도
                   같은 시점에 0부터 시작해야 둘이 동기화된다. */}
                <span
                  key={`${active}-${isPaused ? 'paused' : 'running'}`}
                  className="cw-welcome-dot-fill"
                  data-run={i === active && !reduced && !isPaused}
                  style={{ animationDuration: `${AUTO_ADVANCE_MS}ms` }}
                />
              </span>
            </button>
          ))}
        </div>

        <button
          type="button"
          className="cw-welcome-arrow"
          aria-label="다음"
          onClick={() => go(active + 1)}
        >
          <Icon name="chevron-right" size={18} />
        </button>

        {/* WCAG 2.2.2 (Pause, Stop, Hide) — 5초 이상 자동 업데이트되는
           콘텐츠는 명시적 pause 컨트롤이 필요. reduced motion일 때는
           자동 전환 자체가 꺼져 있어 토글을 숨긴다. */}
        {!reduced && (
          <button
            type="button"
            className="cw-welcome-pause"
            aria-label={userPaused ? '자동 전환 다시 시작' : '자동 전환 멈추기'}
            aria-pressed={userPaused}
            onClick={() => setUserPaused((v) => !v)}
          >
            <Icon name={userPaused ? 'play' : 'pause'} size={14} />
          </button>
        )}
      </div>
    </div>
  );
}

function SlideMock({ kind, active }: { kind: MockKind; active: boolean }) {
  return (
    <div className={`cw-mock cw-mock-${kind}`} data-active={active} aria-hidden="true">
      {kind === 'agent' && (
        <>
          <div className="cw-mock-agent-list">
            {AGENTS.map((a, i) => (
              <div
                className="cw-mock-agent-row"
                data-i={i}
                data-on={a.selected ? 'true' : undefined}
                key={a.name}
              >
                <span className="cw-mock-agent-icon"><Icon name={a.icon} size={14} /></span>
                <span className="cw-mock-agent-name">{a.name}</span>
                {a.selected && (
                  <span className="cw-mock-agent-check"><Icon name="check" size={13} /></span>
                )}
              </div>
            ))}
          </div>
          {/* 선택 후 펼쳐지는 composer — "고른 다음 무엇이 일어나는지" 이야기 완성 */}
          <div className="cw-mock-composer">
            <span className="cw-mock-composer-chip">검색</span>
            <span className="cw-mock-composer-text">무엇을 도와드릴까요?</span>
          </div>
        </>
      )}

      {kind === 'knowledge' && (
        <div className="cw-mock-knowledge">
          {/* 프로젝트에 업로드된 파일들 — 가장 위에서 "이 자료들이 답의 근거"임을 먼저 알린다 */}
          <div className="cw-mock-files">
            {KNOWLEDGE_FILES.map((f, i) => (
              <span className="cw-mock-file" data-i={i} key={f.name}>
                <Icon name={f.icon} size={13} />
                {f.name}
              </span>
            ))}
          </div>
          {/* 사용자 질문 → 답변 + 출처. RAG의 핵심 가치는 출처가 보이는 것 */}
          <div className="cw-mock-question">Q3 매출 알려줘</div>
          <div className="cw-mock-answer">
            <span className="cw-mock-answer-text">전년 동기 대비 12% 성장했습니다.</span>
            <span className="cw-mock-source">
              <Icon name="file-text" size={11} /> Q3-report.pdf · 12p
            </span>
          </div>
        </div>
      )}

      {kind === 'artifact' && (
        <div className="cw-mock-artifact">
          <div className="cw-mock-doc">
            <span className="cw-mock-doc-line" />
            <span className="cw-mock-doc-line" />
            <span className="cw-mock-doc-line" />
            <span className="cw-mock-doc-line" />
          </div>
          <span className="cw-mock-share-badge">
            <Icon name="users" size={14} /> 팀 공유
          </span>
        </div>
      )}

      {kind === 'share' && (
        <div className="cw-mock-share">
          <div className="cw-mock-avatars">
            <span className="cw-mock-av" data-i="0">O</span>
            <span className="cw-mock-av" data-i="1">M</span>
            <span className="cw-mock-av" data-i="2">O</span>
          </div>
          {/* "함께 대화" 모드의 결과 — 팀에서 누군가 AI에게 보낸 메시지 한 줄 */}
          <div className="cw-mock-chat-bubble">오늘 미팅 정리해줘</div>
          <div className="cw-mock-modes">
            <span className="cw-mock-mode">비공개</span>
            <span className="cw-mock-mode">읽기 공유</span>
            <span className="cw-mock-mode is-on">함께 대화</span>
          </div>
        </div>
      )}

      {kind === 'automation' && (
        <div className="cw-mock-automation-inner">
          {/* 트리거 두 종(cron / webhook) — 자동화가 "어떻게 시작되는지" 한눈에 */}
          <div className="cw-mock-trigger-row">
            <span className="cw-mock-trigger" data-kind="cron">
              <Icon name="calendar" size={13} /> 매일 09:00
            </span>
            <span className="cw-mock-trigger" data-kind="webhook">
              <Icon name="zap" size={13} /> Webhook
            </span>
          </div>
          {/* 자동 실행된 run들 — 트리거가 누적해 만들어내는 결과를 시각화 */}
          <div className="cw-mock-runs">
            {AUTOMATION_RUNS.map((r, i) => (
              <div className="cw-mock-run" data-i={i} key={`${r.time}-${i}`}>
                <span className="cw-mock-run-dot" data-status={r.status} />
                <span className="cw-mock-run-time">{r.time}</span>
                <span className="cw-mock-run-label">{r.label}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {kind === 'quickstart' && (
        <ol className="cw-mock-steps">
          {QUICKSTART.map((q) => (
            <li key={q.step}>
              <span className="cw-mock-step-num">{q.step}</span>
              <span className="cw-mock-step-icon"><Icon name={q.icon} size={13} /></span>
              <span className="cw-mock-step-title">{q.title}</span>
              {/* "이 단계에 들어갈 예시 한 토막" — 03/04와 같은 구체성을 마지막
                 슬라이드에도 부여. 좁은 폭에서는 ellipsis로 안전하게 잘린다. */}
              <span className="cw-mock-step-preview">{q.preview}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

const QUICKSTART: { step: string; title: string; icon: IconName; preview: string }[] = [
  { step: '1', title: '프로젝트 만들기', icon: 'folder', preview: 'Cowork Q3' },
  { step: '2', title: '대화 시작',      icon: 'send',   preview: '오늘 미팅 정리' },
  { step: '3', title: '팀과 공유',      icon: 'users',  preview: 'olive · mira' },
];

const AGENTS: { name: string; icon: IconName; selected?: boolean }[] = [
  { name: '분석',     icon: 'analysis' },
  { name: '검색',     icon: 'search', selected: true },
  { name: '심층 조사', icon: 'sparkles' },
  { name: '실행',     icon: 'zap' },
];

// 자료(RAG) mock — 업로드된 파일 두 종과, 답변에 인용된 출처. "출처와 함께 답한다"는
// 본질을 보여주기 위해 답변 라인 옆에 작은 source chip을 강조한다.
const KNOWLEDGE_FILES: { name: string; icon: IconName }[] = [
  { name: 'Q3-report.pdf', icon: 'file-pdf' },
  { name: 'market.docx',   icon: 'file-text' },
];

// 자동화 mock — 가장 위가 "지금 실행 중", 아래로 갈수록 과거 실행.
// "트리거가 누적해 만들어내는 결과"의 시각화이므로 시간 라벨은 상대값으로.
type RunStatus = 'running' | 'success';
const AUTOMATION_RUNS: { time: string; label: string; status: RunStatus }[] = [
  { time: '방금',       label: '데일리 리포트', status: 'running' },
  { time: '어제 09:00', label: '데일리 리포트', status: 'success' },
  { time: '그제 09:00', label: '데일리 리포트', status: 'success' },
];
