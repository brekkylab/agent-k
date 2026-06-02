// Agent / mode selector for the project-home composer, shown as a segmented tab
// row attached to the top of the composer box. Picks WHICH agent surface drives
// the conversation — a different axis from the LLM model picker.
//
// The selected surface id IS the `agent_type` sent to POST /sessions (no
// mapping): it selects the recommended model chain and drives agent dispatch.

import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import { AGENT_SURFACES, type AgentId } from '@/domain/agentSurfaces';

// Renders only the tab row — the wrapping container lives in the home route
// so the picker and composer share one bordered box.
//
// `standalone` rounds all four corners of the active-tab indicator (a pill); the
// default leaves the bottom square so it sits flush on the composer box.
export function ComposerAgentPicker({
  value,
  onChange,
  standalone = false,
}: {
  value: AgentId;
  onChange: (id: AgentId) => void;
  standalone?: boolean;
}) {
  const { t } = useTranslation('automation');
  const tabsRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState({ left: 0, width: 0 });

  useLayoutEffect(() => {
    const tabs = tabsRef.current;
    if (!tabs) return;

    const activeTab = tabs.querySelector<HTMLButtonElement>(`button[data-agent="${value}"]`);
    if (!activeTab) return;

    const updateIndicator = () => {
      const tabsRect = tabs.getBoundingClientRect();
      const tabRect = activeTab.getBoundingClientRect();
      setIndicator({
        left: tabRect.left - tabsRect.left,
        width: tabRect.width,
      });
    };

    updateIndicator();

    const resizeObserver = new ResizeObserver(updateIndicator);
    resizeObserver.observe(tabs);
    resizeObserver.observe(activeTab);
    window.addEventListener('resize', updateIndicator);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener('resize', updateIndicator);
    };
  }, [value]);

  const indicatorStyle = {
    '--cw-agent-indicator-left': `${indicator.left}px`,
    '--cw-agent-indicator-width': `${indicator.width}px`,
  } as CSSProperties;

  return (
    <div
      ref={tabsRef}
      role="group"
      aria-label={t('agent_picker.group_label')}
      className={`cw-agent-tabs${standalone ? ' is-standalone' : ''}`}
    >
      <span
        key={value}
        aria-hidden
        className="cw-agent-tab-indicator"
        data-agent={value}
        style={indicatorStyle}
      />
      {AGENT_SURFACES.map((agent) => {
        const isActive = agent.id === value;
        return (
          <button
            key={agent.id}
            type="button"
            aria-pressed={isActive}
            data-agent={agent.id}
            className={`cw-agent-tab${isActive ? ' is-active' : ''}`}
            onClick={() => onChange(agent.id)}
          >
            <Icon name={agent.icon} size={15} />
            <span>{agent.label}</span>
          </button>
        );
      })}
    </div>
  );
}
