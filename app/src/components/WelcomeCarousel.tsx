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
  // To use a real demo video, drop app/public/welcome/<file>.mp4 and specify its path;
  // it then plays automatically instead of the mockup. e.g. video: '/welcome/routing.mp4'
  video?: string;
  poster?: string;
}

// "Make a lot, then trim" — start rich. You can pare down by removing or merging slides later.
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

  // WCAG 2.2.2 (Pause, Stop, Hide) — auto-advance stops if any one of the following three
  // is on. hover is excluded by user requirement ("the progress bar must not stop on hover").
  //  · userPaused: explicit pause/play toggle
  //  · focusedWithin: a keyboard user moves focus inside the carousel (a different scenario from mouse hover)
  //  · pageHidden: the tab is hidden in the background
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

  // activeIndex is derived from the scroll position — swipe/auto-advance flows in only one direction.
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

  // page hidden tracking — when the tab is in the background, prevents auto-advance from
  // making meaningless changes for users who aren't looking.
  useEffect(() => {
    const onVis = () => setPageHidden(document.hidden);
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  }, []);

  const isPaused = userPaused || focusedWithin || pageHidden;

  // auto-advance — when the slide changes, restart the next countdown. Only while isPaused is false.
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

  // when focus moves around within the container(arrow → dot, etc.), don't treat it as a blur.
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
        // Ignore focus caused by a mouse click. Only keyboard(Tab) entry is a pause trigger —
        // the "examining the carousel" scenario is limited to keyboard users, and this avoids
        // the false positive of the pause button gaining focus right after a mouse click.
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
                {/* The key forces the fill element to remount so the CSS animation restarts.
                   When the active slide changes or the isPaused toggle is released, the
                   auto-advance setTimeout resets to a fresh 7 seconds, and the progress bar
                   must also start from 0 at the same moment so the two stay in sync. */}
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

        {/* WCAG 2.2.2 (Pause, Stop, Hide) — content that auto-updates for more than 5 seconds
           needs an explicit pause control. Under reduced motion, auto-advance is already
           off, so the toggle is hidden. */}
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
          {/* The composer that expands after selection — completes the "what happens after you pick" story */}
          <div className="cw-mock-composer">
            <span className="cw-mock-composer-chip">검색</span>
            <span className="cw-mock-composer-text">무엇을 도와드릴까요?</span>
          </div>
        </>
      )}

      {kind === 'knowledge' && (
        <div className="cw-mock-knowledge">
          {/* Files uploaded to the project — placed at the top to first signal "these sources back the answer" */}
          <div className="cw-mock-files">
            {KNOWLEDGE_FILES.map((f, i) => (
              <span className="cw-mock-file" data-i={i} key={f.name}>
                <Icon name={f.icon} size={13} />
                {f.name}
              </span>
            ))}
          </div>
          {/* User question → answer + source. The core value of RAG is that the source is visible */}
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
          {/* Result of "converse together" mode — a single line someone on the team sent to the AI */}
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
          {/* Two trigger types(cron / webhook) — shows at a glance "how automation starts" */}
          <div className="cw-mock-trigger-row">
            <span className="cw-mock-trigger" data-kind="cron">
              <Icon name="calendar" size={13} /> 매일 09:00
            </span>
            <span className="cw-mock-trigger" data-kind="webhook">
              <Icon name="zap" size={13} /> Webhook
            </span>
          </div>
          {/* Auto-executed runs — visualizes the results that triggers accumulate over time */}
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
              {/* "A snippet of example for this step" — gives the last slide the same
                 concreteness as 03/04. At narrow widths it is safely truncated with an ellipsis. */}
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

// Knowledge(RAG) mock — two uploaded file types and the source cited in the answer. To convey
// the essence of "answering with sources," a small source chip is emphasized next to the answer line.
const KNOWLEDGE_FILES: { name: string; icon: IconName }[] = [
  { name: 'Q3-report.pdf', icon: 'file-pdf' },
  { name: 'market.docx',   icon: 'file-text' },
];

// Automation mock — the top is "running now," and further down are past runs.
// Since this visualizes "the results that triggers accumulate," the time labels are relative.
type RunStatus = 'running' | 'success';
const AUTOMATION_RUNS: { time: string; label: string; status: RunStatus }[] = [
  { time: '방금',       label: '데일리 리포트', status: 'running' },
  { time: '어제 09:00', label: '데일리 리포트', status: 'success' },
  { time: '그제 09:00', label: '데일리 리포트', status: 'success' },
];
