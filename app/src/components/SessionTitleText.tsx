import { useTypingText } from '@/lib/useTypingText';

export function SessionTitleText({ title }: { title: string }) {
  const { text, typing } = useTypingText(title);
  return (
    <span className={typing ? 'cw-typing-caret' : undefined}>
      {text}
    </span>
  );
}
