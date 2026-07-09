// Home composer surface for the `/` route. Ported (slimmed) from the original
// Cowork app's ProjectHomeComposer: a roomier multiline textarea with an
// agent-type picker + optional model input in the footer, plus a send button.
// The @mention/command/attachment/view-transition machinery from the original
// is intentionally left out — app_v2's home is a single-user "new chat" surface.

import { useEffect, useRef, type KeyboardEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import type { AgentType } from '@/api/types';

export interface ProjectHomeComposerSubmission {
  text: string;
}

const AGENT_TYPES: AgentType[] = ['coworker', 'deep_research'];

interface ProjectHomeComposerProps {
  value: string;
  onChange: (next: string) => void;
  onSubmit: (submission: ProjectHomeComposerSubmission) => void | Promise<void>;
  agentType: AgentType;
  onAgentTypeChange: (next: AgentType) => void;
  model: string;
  onModelChange: (next: string) => void;
  disabled?: boolean;
  // While the send is being processed (session creation / stream start): show a
  // spinner on the send button to indicate "sending".
  pending?: boolean;
  placeholder?: string;
  // Inline error rendered below the composer (session-create / send failure).
  error?: string | null;
}

const MAX_TEXTAREA_HEIGHT = 200;

export function ProjectHomeComposer({
  value,
  onChange,
  onSubmit,
  agentType,
  onAgentTypeChange,
  model,
  onModelChange,
  disabled = false,
  pending = false,
  placeholder,
  error,
}: ProjectHomeComposerProps) {
  const { t } = useTranslation('session');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const canSubmit = value.trim().length > 0 && !disabled;

  // Auto-grow: reset to auto first so removing lines also shrinks the box. Keep
  // overflow hidden until we actually hit the cap — otherwise a sub-pixel
  // rounding of scrollHeight vs clientHeight flashes a phantom scrollbar.
  useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    const next = Math.min(ta.scrollHeight, MAX_TEXTAREA_HEIGHT);
    ta.style.height = `${next}px`;
    ta.style.overflowY = ta.scrollHeight > MAX_TEXTAREA_HEIGHT ? 'auto' : 'hidden';
  }, [value]);

  const submit = () => {
    if (canSubmit) void onSubmit({ text: value.trim() });
  };

  // Enter sends; Shift+Enter inserts a newline. isComposing guards Korean/IME
  // composition so confirming a character with Enter doesn't fire a send.
  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      submit();
    }
  };

  return (
    <form
      className="cw-home-composer"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <div className="cw-home-composer-box">
        <textarea
          ref={textareaRef}
          className="cw-home-composer-input"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder ?? t('home.placeholder')}
          disabled={disabled}
          rows={3}
        />
        <div className="cw-home-composer-actions">
          <label className="cw-home-agent-pick">
            <span className="cw-home-agent-pick-label">{t('home.agent_label')}</span>
            <select
              className="cw-home-agent-select"
              value={agentType}
              onChange={(event) => onAgentTypeChange(event.target.value as AgentType)}
              disabled={disabled}
              aria-label={t('home.agent_label')}
            >
              {AGENT_TYPES.map((a) => (
                <option key={a} value={a}>
                  {t(`home.agent.${a}`)}
                </option>
              ))}
            </select>
          </label>
          <input
            className="cw-home-model-input"
            value={model}
            onChange={(event) => onModelChange(event.target.value)}
            placeholder={t('home.model_placeholder')}
            disabled={disabled}
            aria-label={t('home.model_label')}
          />
          <span className="cw-home-composer-actions-spacer" />
          <button
            type="submit"
            className="cw-send-button"
            aria-label={t('home.send')}
            disabled={!canSubmit || pending}
          >
            {pending ? <span className="cw-send-spinner" aria-hidden /> : <Icon name="send" size={13} />}
          </button>
        </div>
      </div>
      {error ? (
        <small className="is-blocked">{error}</small>
      ) : (
        <small>{t('home.hint')}</small>
      )}
    </form>
  );
}
