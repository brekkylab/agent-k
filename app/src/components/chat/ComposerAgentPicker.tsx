// Agent / mode selector for the project-home composer, shown as a segmented tab
// row above the input (Perplexity Search/Research style). This picks WHICH agent
// surface drives the conversation — a different axis from the LLM model picker.
//
// Mock for now: the selection is not wired to create-session / dispatch yet
// (the backend's CreateSessionRequest only takes project_id, with
// deny_unknown_fields). The IDs below are the real intended agent surfaces so the
// seam is concrete — a follow-up PR routes ComposerSubmission.agentHint here.

import { Icon, type IconName } from '@/components/Icon';

export type AgentId = 'coworker' | 'speedwagon' | 'deep-research' | 'buddy';

interface AgentOption {
  id: AgentId;
  label: string;
  icon: IconName;
  // One-line role, shown as a subtitle under the active tab.
  blurb: string;
  // Composer placeholder when this agent is active, so the surface reacts to the choice.
  placeholder: string;
}

export const AGENT_OPTIONS: readonly AgentOption[] = [
  {
    id: 'coworker',
    label: 'Coworker',
    icon: 'zap',
    blurb: '작업 실행 에이전트',
    placeholder: '실행할 작업을 적어줘…',
  },
  {
    id: 'speedwagon',
    label: 'Speedwagon',
    icon: 'search',
    blurb: '지식 기반 질의 응답',
    placeholder: '무엇이 궁금해?',
  },
  {
    id: 'deep-research',
    label: 'Deep Research',
    icon: 'analysis',
    blurb: '심층 조사·리서치',
    placeholder: '무엇을 조사할까?',
  },
  {
    id: 'buddy',
    label: 'Buddy',
    icon: 'brainstorm',
    blurb: '브레인스토밍·번역 등 채팅',
    placeholder: '편하게 말 걸어줘…',
  },
] as const;

export const DEFAULT_AGENT_ID: AgentId = 'coworker';

export function getAgentOption(id: AgentId): AgentOption {
  return AGENT_OPTIONS.find((a) => a.id === id) ?? AGENT_OPTIONS[0];
}

export function ComposerAgentPicker({
  value,
  onChange,
}: {
  value: AgentId;
  onChange: (id: AgentId) => void;
}) {
  const active = getAgentOption(value);
  return (
    <div className="cw-agent-picker">
      <div role="tablist" aria-label="에이전트 선택 (미리보기)" className="cw-agent-tabs">
        {AGENT_OPTIONS.map((agent) => {
          const isActive = agent.id === value;
          return (
            <button
              key={agent.id}
              type="button"
              role="tab"
              aria-selected={isActive}
              className={`cw-agent-tab${isActive ? ' is-active' : ''}`}
              onClick={() => onChange(agent.id)}
            >
              <Icon name={agent.icon} size={15} />
              <span>{agent.label}</span>
            </button>
          );
        })}
      </div>
      <p className="cw-agent-blurb">{active.blurb}</p>
    </div>
  );
}
