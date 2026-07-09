import { useState, type KeyboardEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { stopRun } from '@/api/messages';

interface ComposerProps {
  running: boolean;
  sessionId: string;
  onSend: (text: string) => void;
}

export function Composer({ running, sessionId, onSend }: ComposerProps) {
  const { t } = useTranslation('session');
  const [value, setValue] = useState('');
  const canSend = value.trim().length > 0 && !running;

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  function submit() {
    const text = value.trim();
    if (!text || running) return;
    setValue('');
    onSend(text);
  }

  function handleStop() {
    void stopRun(sessionId);
  }

  return (
    <div className="cw-composer">
      <div className="cw-composer-box">
        <textarea
          placeholder={t('ui.composer_placeholder')}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          rows={1}
        />
        <div className="cw-composer-actions">
          {running ? (
            <button
              type="button"
              className="cw-composer-stop"
              aria-label={t('ui.stop_generation')}
              onClick={handleStop}
            >
              Stop
            </button>
          ) : (
            <button
              type="button"
              className="cw-composer-send"
              aria-label={t('ui.send_aria')}
              onClick={submit}
              disabled={!canSend}
            >
              Send
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
