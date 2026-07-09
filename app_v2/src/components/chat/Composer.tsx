import { useState, type KeyboardEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { stopRun } from '@/api/messages';

interface ComposerProps {
  running: boolean;
  sessionId: string;
  /** Returns true if the send was accepted; false restores the typed text. */
  onSend: (text: string) => Promise<boolean>;
}

export function Composer({ running, sessionId, onSend }: ComposerProps) {
  const { t } = useTranslation('session');
  const [value, setValue] = useState('');
  const canSend = value.trim().length > 0 && !running;

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  }

  async function submit() {
    const text = value.trim();
    if (!text || running) return;
    // Clear optimistically for a snappy input; restore if the send is rejected
    // (e.g. a 409 while the previous run's tail is still archiving).
    setValue('');
    const accepted = await onSend(text);
    if (!accepted) setValue(text);
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
              onClick={() => void submit()}
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
