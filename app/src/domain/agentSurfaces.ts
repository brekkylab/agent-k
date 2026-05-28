export type AgentId = 'coworker' | 'speedwagon' | 'deep-research' | 'buddy';

export type AgentSurfaceIcon = 'zap' | 'search' | 'analysis' | 'brainstorm';

export interface SuggestedPrompt {
  label: string;
  seedText: string;
}

export interface AgentSurface {
  id: AgentId;
  label: string;
  icon: AgentSurfaceIcon;
  // Composer placeholder when this agent is active.
  placeholder: string;
  // Dynamic greeting shown on the home surface.
  greeting: string;
  // Agent-specific suggested prompts (3 per agent).
  prompts: SuggestedPrompt[];
}

export const AGENT_SURFACES: readonly AgentSurface[] = [
  {
    id: 'coworker',
    label: 'Coworker',
    icon: 'zap',
    placeholder: '실행할 작업을 적어줘…',
    greeting: 'Coworker가 실행을 도와줘요',
    prompts: [
      { label: '이번 주 진행 요약', seedText: '이번 주 진행 상황을 정리해서 보고서 초안 만들어줘' },
      { label: '회의 메모 정리', seedText: '오늘 회의록을 액션 아이템 중심으로 정리해줘' },
      { label: '이메일 초안', seedText: '이번 결정을 팀에 공유할 이메일 초안 써줘' },
    ],
  },
  {
    id: 'speedwagon',
    label: 'Speedwagon',
    icon: 'search',
    placeholder: '무엇이 궁금해?',
    greeting: 'Speedwagon이 파일에서 답을 찾아줘요',
    prompts: [
      { label: '프로젝트 파일 요약', seedText: '프로젝트 파일들의 핵심 내용을 요약해줘' },
      { label: '특정 사실 찾기', seedText: '문서들에서 매출 관련 수치를 모두 찾아줘' },
      { label: '문서 비교', seedText: '최근 두 보고서의 차이를 비교해줘' },
    ],
  },
  {
    id: 'deep-research',
    label: 'Deep Research',
    icon: 'analysis',
    placeholder: '무엇을 조사할까?',
    greeting: 'Deep Research가 깊이 파고들어요',
    prompts: [
      { label: '시장 동향 조사', seedText: 'KlientCo가 있는 시장의 최근 동향을 조사해줘' },
      { label: '경쟁사 분석', seedText: '주요 경쟁사 3곳의 전략을 비교 분석해줘' },
      { label: '선행 연구 정리', seedText: '관련 분야 선행 연구를 정리해줘' },
    ],
  },
  {
    id: 'buddy',
    label: 'Buddy',
    icon: 'brainstorm',
    placeholder: '편하게 말 걸어줘…',
    greeting: 'Buddy와 자유롭게 대화해요',
    prompts: [
      { label: '아이디어 발산', seedText: 'Q3 캠페인 아이디어 10개만 빠르게 던져줘' },
      { label: '용어 풀이', seedText: '이 도메인 용어들을 쉽게 설명해줘' },
      { label: '번역', seedText: '이 문장을 자연스러운 한국어로 번역해줘' },
    ],
  },
] as const;

export const DEFAULT_AGENT_ID: AgentId = AGENT_SURFACES[0].id;

export function getAgentSurface(id: string | undefined): AgentSurface {
  return AGENT_SURFACES.find((agent) => agent.id === id) ?? AGENT_SURFACES[0];
}
