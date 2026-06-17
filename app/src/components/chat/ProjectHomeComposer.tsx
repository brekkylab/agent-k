// Project-home composer preview surface. Separate from the session composer because
// this surface owns agent/model picker UI, suggested-prompt handoff, and a roomier
// multiline textarea.

import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Icon } from '@/components/Icon';
import { AttachmentChip } from '@/components/AttachmentChip';

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
  // While the send is being processed (waiting for session creation / stream start): show a spinner on the send button to indicate "sending".
  pending?: boolean;
  // 현재 모델 선택이 실행 불가(가용 provider 없음)일 때 send 를 막고 힌트를 표시.
  sendBlocked?: boolean;
  sendBlockedHint?: string;
  placeholder?: string;
  // Entry point for adding files (placeholder — to be wired up to PR #114's attachment tray).
  onAttachClick?: () => void;
  // Shared (server) files staged for attach, picked from the shared-file dialog.
  // Referenced by global path — already on the server, so no upload on send.
  sharedFiles?: { globalPath: string; filename: string }[];
  // Opens the shared-file picker dialog. Omitted (button hidden) until the project resolves.
  onPickShared?: () => void;
  onRemoveShared?: (index: number) => void;
  // Home-only model picker slot. Session composer owns its own compact controls.
  modelPicker?: ReactNode;
  // Focus request signal. Focuses the input whenever the value changes (it's a nonce, not a
  // boolean, so focus fires every time even if the same request repeats). 0/undefined means no focus.
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
  sendBlocked = false,
  sendBlockedHint,
  placeholder = DEFAULT_PLACEHOLDER,
  onAttachClick,
  sharedFiles = [],
  onPickShared,
  onRemoveShared,
  modelPicker,
  focusSignal,
}: ProjectHomeComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const canSubmit = value.trim().length > 0 && !disabled && !sendBlocked;

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

  const sharedButton = onPickShared && (
    <button
      type="button"
      className="cw-attach-button"
      onClick={onPickShared}
      disabled={disabled}
      aria-label="공유 파일 첨부"
      title="공유 파일 첨부"
    >
      <Icon name="folder" size={17} />
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
        {sharedFiles.length > 0 && (
          <div className="cw-attach-tray" style={{ display: 'flex', flexWrap: 'wrap', gap: 6, padding: '0 0 6px' }}>
            {sharedFiles.map((item, i) => (
              <AttachmentChip
                key={`s-${item.globalPath}`}
                filename={item.filename}
                status="uploaded"
                shared
                onRemove={() => onRemoveShared?.(i)}
              />
            ))}
          </div>
        )}
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
          {sharedButton}
          {attachButton}
          {sendButton}
        </div>
      </div>
      <small className={sendBlocked ? 'is-blocked' : undefined}>
        {sendBlocked && sendBlockedHint ? sendBlockedHint : DEFAULT_HINT}
      </small>
    </form>
  );
}
