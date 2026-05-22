import { useTypingText } from '@/lib/useTypingText';

export function SessionTitleText({ title }: { title: string }) {
  const { text, typing } = useTypingText(title);
  return (
    <span className={['cw-session-title', typing ? 'cw-typing-caret' : ''].filter(Boolean).join(' ')}>
      {text}
    </span>
  );
}
