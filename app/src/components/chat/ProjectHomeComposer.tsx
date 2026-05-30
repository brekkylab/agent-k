// Project-home composer preview surface. Separate from the session composer because
// this surface owns agent/model picker UI, suggested-prompt handoff, and a roomier
// multiline textarea.

import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Icon } from '@/components/Icon';

export interface ProjectHomeComposerSubmission {
  text: string;
  // future: agentHint?: string;
  // future: promptId?: string;
}

interface ProjectHomeComposerProps {
  value: string;
  onChange: (next: string) => void;
  onSubmit: (submission: ProjectHomeComposerSubmission) => void | Promise<void>;
  disabled?: boolean;
  // 전송 처리 중(세션 생성/스트림 시작 대기): send 버튼에 스피너를 띄워 "보내는 중"을 알림.
  pending?: boolean;
  placeholder?: string;
  // 파일 추가 진입점 (placeholder — PR #114 의 attachment tray 와 연결 예정).
  onAttachClick?: () => void;
  // Home-only model picker slot. Session composer owns its own compact controls.
  modelPicker?: ReactNode;
  // focus 요청 신호. 값이 바뀔 때마다 input 에 focus (boolean 이 아니라 nonce 라
  // 같은 요청이 반복돼도 매번 focus 가 걸린다). 0/undefined 면 focus 안 함.
  focusSignal?: number;
}

const DEFAULT_PLACEHOLDER = 'Message Cowork and the team…';
const DEFAULT_HINT = 'Enter to send · Shift+Enter for newline · Reference files with @filename';
const MAX_TEXTAREA_HEIGHT = 200;

export function ProjectHomeComposer({
  value,
  onChange,
  onSubmit,
  disabled = false,
  pending = false,
  placeholder = DEFAULT_PLACEHOLDER,
  onAttachClick,
  modelPicker,
  focusSignal,
}: ProjectHomeComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const canSubmit = value.trim().length > 0 && !disabled;

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
          placeholder={placeholder}
          disabled={disabled}
          rows={3}
        />
        <div className="cw-home-composer-actions">
          {modelPicker}
          <span className="cw-home-composer-actions-spacer" />
          {attachButton}
          {sendButton}
        </div>
      </div>
      <small>{DEFAULT_HINT}</small>
    </form>
  );
}
