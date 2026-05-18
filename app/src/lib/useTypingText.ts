import { useEffect, useRef, useState } from 'react';

const TYPING_INTERVAL_MS = 32;
const TYPING_CHARS_PER_TICK = 1;
const FALLBACK_TITLE = '새 대화';

interface TypingResult {
  text: string;
  typing: boolean;
}

export function useTypingText(target: string): TypingResult {
  const [text, setText] = useState(target);
  const [typing, setTyping] = useState(false);
  const prevTargetRef = useRef<string | null>(null);

  useEffect(() => {
    const prev = prevTargetRef.current;
    prevTargetRef.current = target;

    if (prev === null || prev === target || prev !== FALLBACK_TITLE) {
      setText(target);
      setTyping(false);
      return;
    }

    setText('');
    setTyping(true);
    let i = 0;
    const id = window.setInterval(() => {
      i += TYPING_CHARS_PER_TICK;
      if (i >= target.length) {
        setText(target);
        setTyping(false);
        window.clearInterval(id);
      } else {
        setText(target.slice(0, i));
      }
    }, TYPING_INTERVAL_MS);

    return () => window.clearInterval(id);
  }, [target]);

  return { text, typing };
}
