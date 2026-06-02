import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FALLBACK_TITLE } from './sessionConstants';

const TYPING_INTERVAL_MS = 32;
const TYPING_CHARS_PER_TICK = 1;

export { FALLBACK_TITLE } from './sessionConstants';

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
  // Keep the latest displayTarget reachable inside the effect without making
  // it a dependency — otherwise a locale switch on an untitled session would
  // re-fire the typing animation just to swap the placeholder copy.
  const displayTargetRef = useRef(displayTarget);
  displayTargetRef.current = displayTarget;

  useEffect(() => {
    const prev = prevTargetRef.current;
    prevTargetRef.current = target;
    const current = displayTargetRef.current;

    if (prev === null || prev === target || prev !== FALLBACK_TITLE) {
      setText(current);
      setTyping(false);
      return;
    }

    setText('');
    setTyping(true);
    let i = 0;
    const id = window.setInterval(() => {
      i += TYPING_CHARS_PER_TICK;
      if (i >= current.length) {
        setText(current);
        setTyping(false);
        window.clearInterval(id);
      } else {
        setText(current.slice(0, i));
      }
    }, TYPING_INTERVAL_MS);

    return () => window.clearInterval(id);
  }, [target]);

  // A pure language switch (target unchanged, displayTarget changed) updates
  // the rendered text in place without restarting the typing animation.
  useEffect(() => {
    if (target !== FALLBACK_TITLE) return;
    setText(displayTarget);
  }, [displayTarget, target]);

  return { text, typing };
}
