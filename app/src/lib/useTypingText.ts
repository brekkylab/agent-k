import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

const TYPING_INTERVAL_MS = 32;
const TYPING_CHARS_PER_TICK = 1;

/**
 * Sentinel value used by `transformers.ts` to mark sessions without a
 * backend-assigned title. The hook detects the sentinel→real-title
 * transition to drive the typing animation, and renders the sentinel as
 * the locale-appropriate "untitled" placeholder.
 */
export const FALLBACK_TITLE = '__cw_untitled__';

interface TypingResult {
  text: string;
  typing: boolean;
}

export function useTypingText(target: string): TypingResult {
  const { t } = useTranslation('common');
  const displayTarget = target === FALLBACK_TITLE ? t('placeholders.untitled_session') : target;

  const [text, setText] = useState(displayTarget);
  const [typing, setTyping] = useState(false);
  const prevTargetRef = useRef<string | null>(null);

  useEffect(() => {
    const prev = prevTargetRef.current;
    prevTargetRef.current = target;

    if (prev === null || prev === target || prev !== FALLBACK_TITLE) {
      setText(displayTarget);
      setTyping(false);
      return;
    }

    setText('');
    setTyping(true);
    let i = 0;
    const id = window.setInterval(() => {
      i += TYPING_CHARS_PER_TICK;
      if (i >= displayTarget.length) {
        setText(displayTarget);
        setTyping(false);
        window.clearInterval(id);
      } else {
        setText(displayTarget.slice(0, i));
      }
    }, TYPING_INTERVAL_MS);

    return () => window.clearInterval(id);
  }, [target, displayTarget]);

  return { text, typing };
}
