import { useCallback, useEffect, useRef, useState } from 'react';
import { Icon, type IconName } from '@/components/Icon';

type MockKind = 'agent' | 'artifact' | 'share' | 'automation' | 'quickstart';

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
    title: '원하는 에이전트에 맡기기',
    blurb: '새 세션을 시작할 때 에이전트를 고릅니다.',
    detail: '작업의 성격에 맞는 에이전트를 선택해 요청을 보냅니다. 골라 둔 에이전트가 그 대화를 책임지고 처리합니다.',
    mock: 'agent',
  },
  {
    key: 'artifact',
    eyebrow: '02 · 산출물',
    title: '결과물을 바로 손에',
    blurb: '보고서·코드·문서를 만들어 드립니다.',
    detail: '에이전트가 생성한 산출물을 내려받거나, 클릭 한 번으로 팀 공유 폴더로 옮겨 함께 이어갑니다.',
    mock: 'artifact',
  },
  {
    key: 'share',
    eyebrow: '03 · 협업',
    title: '팀이 함께 대화하는 세션',
    blurb: '공개 범위를 골라 함께 묻습니다.',
    detail: '비공개·읽기 공유·함께 대화 모드로 팀원이 같은 세션에서 같은 AI와 실시간 협력합니다. 문서를 올리면 출처와 함께 답합니다.',
    mock: 'share',
  },
  {
    key: 'automation',
    eyebrow: '04 · 자동화',
    title: '사람 없이 알아서 돌게',
    blurb: '스케줄·웹훅으로 에이전트가 정기·이벤트 실행됩니다.',
    detail: '매일 아침 리포트, 외부 알람이 트리거하는 분석처럼 반복되거나 사건에 반응해야 하는 작업을 등록해 두면 결과만 쌓입니다. 자동화 세션은 일반 대화와 분리되어 따로 관리됩니다.',
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

  // 자동 전환 — 슬라이드가 바뀌면(자동·수동 모두) 다음 5초 카운트를 새로 시작.
  // pause 로직 없음: hover/focus와 무관하게 항상 진행한다.
  useEffect(() => {
    if (reduced) return;
    const t = window.setTimeout(() => {
      scrollToIndex((activeRef.current + 1) % SLIDES.length);
    }, AUTO_ADVANCE_MS);
    return () => window.clearTimeout(t);
  }, [reduced, scrollToIndex, active]);

  const go = useCallback((i: number) => {
    scrollToIndex((i + SLIDES.length) % SLIDES.length);
  }, [scrollToIndex]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowRight') { e.preventDefault(); go(active + 1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); go(active - 1); }
  };

  return (
    <div
      className="cw-welcome-carousel"
      role="region"
      aria-roledescription="carousel"
      aria-label="제품 소개"
      tabIndex={0}
      onKeyDown={onKeyDown}
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
                <span
                  className="cw-welcome-dot-fill"
                  data-run={i === active && !reduced}
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
              <span>{q.title}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

const QUICKSTART: { step: string; title: string; icon: IconName }[] = [
  { step: '1', title: '프로젝트 만들기', icon: 'folder' },
  { step: '2', title: '대화 시작', icon: 'send' },
  { step: '3', title: '팀과 공유', icon: 'users' },
];

const AGENTS: { name: string; icon: IconName; selected?: boolean }[] = [
  { name: '분석', icon: 'analysis' },
  { name: '검색', icon: 'search', selected: true },
  { name: '실행', icon: 'zap' },
];

// 자동화 mock — 가장 위가 "지금 실행 중", 아래로 갈수록 과거 실행.
// "트리거가 누적해 만들어내는 결과"의 시각화이므로 시간 라벨은 상대값으로.
type RunStatus = 'running' | 'success';
const AUTOMATION_RUNS: { time: string; label: string; status: RunStatus }[] = [
  { time: '방금',       label: '데일리 리포트', status: 'running' },
  { time: '어제 09:00', label: '데일리 리포트', status: 'success' },
  { time: '그제 09:00', label: '데일리 리포트', status: 'success' },
];
