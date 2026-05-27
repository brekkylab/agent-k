// Shared chat composer. Extracted from the inline form that used to live in the
// session page so the project home ("new conversation" surface) and the session
// page render the exact same input. onSubmit takes an envelope object so future
// dispatch metadata (agentHint / promptId from suggested-prompt chips) can be added
// without touching every call site.
//
// Two sizes intentionally diverge:
//   - 'pill'  (default, session page): a tight single-row rounded pill matching PR #114.
//   - 'large' (home "new conversation"): a roomier box — textarea on top, a bottom
//     toolbar row (model picker on the left, attach + send on the right) so the home
//     surface doesn't feel cramped with the model picker crammed inline.

import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Icon } from '@/components/Icon';

export interface ComposerSubmission {
  text: string;
  // future: agentHint?: string;
  // future: promptId?: string;
}

interface SessionComposerProps {
  value: string;
  onChange: (next: string) => void;
  onSubmit: (submission: ComposerSubmission) => void | Promise<void>;
  disabled?: boolean;
  // 전송 처리 중(세션 생성/스트림 시작 대기): send 버튼에 스피너를 띄워 "보내는 중"을 알림.
  pending?: boolean;
  placeholder?: string;
  // 파일 추가 진입점 (placeholder — PR #114 의 attachment tray 와 연결 예정).
  onAttachClick?: () => void;
  footerHint?: ReactNode;
  // composer 아래 suggested prompts 슬롯.
  belowSlot?: ReactNode;
  // model picker 슬롯. pill 에선 send 앞 inline, large 에선 하단 toolbar 좌측.
  actionsSlot?: ReactNode;
  // 'pill'(세션, 기본) | 'large'(home). 레이아웃과 textarea 기본 높이가 달라진다.
  size?: 'pill' | 'large';
  // focus 요청 신호. 값이 바뀔 때마다 input 에 focus (boolean 이 아니라 nonce 라
  // 같은 요청이 반복돼도 매번 focus 가 걸린다). 0/undefined 면 focus 안 함.
  focusSignal?: number;
}

const DEFAULT_PLACEHOLDER = 'Message Cowork and the team…';
const DEFAULT_HINT = 'Enter to send · Shift+Enter for newline · Reference files with @filename';
const MAX_TEXTAREA_HEIGHT = 200;

export function SessionComposer({
  value,
  onChange,
  onSubmit,
  disabled = false,
  pending = false,
  placeholder = DEFAULT_PLACEHOLDER,
  onAttachClick,
  footerHint = DEFAULT_HINT,
  belowSlot,
  actionsSlot,
  size = 'pill',
  focusSignal,
}: SessionComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const canSubmit = value.trim().length > 0 && !disabled;
  const isLarge = size === 'large';

  useEffect(() => {
    if (focusSignal) textareaRef.current?.focus();
  }, [focusSignal]);

  // Auto-grow: reset to auto first so removing lines also shrinks the box. Keep
  // overflow hidden until we actually hit the cap — otherwise a sub-pixel rounding
  // of scrollHeight vs clientHeight makes `overflow-y: auto` flash a phantom scrollbar.
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

  const attachButton = onAttachClick && (
    <button
      type="button"
      className="cw-attach-button"
      onClick={onAttachClick}
      disabled={disabled}
      aria-label="파일 추가"
      title="파일 추가"
    >
      <Icon name="paperclip" size={17} />
    </button>
  );

  const sendButton = (
    <button type="submit" className="cw-send-button" aria-label="Send" disabled={!canSubmit || pending}>
      {pending ? <span className="cw-send-spinner" aria-hidden /> : <Icon name="send" size={12} />}
    </button>
  );

  return (
    <form
      className={`cw-composer${isLarge ? ' cw-composer--large' : ''}`}
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <div className={`cw-composer-box${isLarge ? ' cw-composer-box--large' : ''}`}>
        <textarea
          ref={textareaRef}
          className="cw-composer-input"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          disabled={disabled}
          rows={1}
        />
        {isLarge ? (
          <div className="cw-composer-actions">
            {actionsSlot}
            <span className="cw-composer-actions-spacer" />
            {attachButton}
            {sendButton}
          </div>
        ) : (
          <>
            {actionsSlot}
            {attachButton}
            {sendButton}
          </>
        )}
      </div>
      {footerHint && <small>{footerHint}</small>}
      {belowSlot}
    </form>
  );
}
