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
  // Fallback attach entry point when file handling isn't wired (no onAddFiles).
  onAttachClick?: () => void;
  // Local files staged for upload (uploaded to the new session's inputs/ on submit).
  files?: File[];
  onAddFiles?: (files: File[]) => void;
  onRemoveFile?: (index: number) => void;
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
  files = [],
  onAddFiles,
  onRemoveFile,
  modelPicker,
  focusSignal,
}: ProjectHomeComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
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

  const canAttach = Boolean(onAddFiles || onAttachClick);
  const attachButton = canAttach && (
    <button
      type="button"
      className="cw-attach-button"
      onClick={() => (onAddFiles ? fileInputRef.current?.click() : onAttachClick?.())}
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
        {files.length > 0 && (
          <div className="cw-attach-tray">
            {files.map((file, i) => (
              <AttachmentChip
                key={`${file.name}-${i}`}
                filename={file.name}
                status="staged"
                onRemove={() => onRemoveFile?.(i)}
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
        {onAddFiles && (
          <input
            ref={fileInputRef}
            type="file"
            multiple
            style={{ display: 'none' }}
            onChange={(e) => { onAddFiles(Array.from(e.target.files ?? [])); e.target.value = ''; }}
          />
        )}
        <div className="cw-home-composer-actions">
          {modelPicker}
          <span className="cw-home-composer-actions-spacer" />
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
