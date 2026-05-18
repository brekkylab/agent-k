// Modal for creating a new folder at the current Files location.
// Replaces the previous window.prompt() with an in-design dialog that
// surfaces duplicate-name and invalid-character validation inline.

import { useEffect, useRef, useState } from 'react';
import { Icon } from './Icon';

interface NewFolderDialogProps {
  existingNames: string[];
  pending: boolean;
  onConfirm: (name: string) => void;
  onClose: () => void;
}

function validate(raw: string, existing: string[]): string | null {
  const name = raw.trim();
  if (name.length === 0) return null;
  if (/[\\/]/.test(name)) return '폴더 이름에 / 또는 \\ 문자는 사용할 수 없습니다.';
  if (/^\.+$/.test(name)) return '. 만으로 이루어진 이름은 사용할 수 없습니다.';
  if (name.startsWith('.')) return '. 으로 시작하는 이름은 숨김으로 처리되어 보이지 않습니다.';
  if (existing.some((e) => e.toLowerCase() === name.toLowerCase())) {
    return `"${name}" 이름의 폴더가 이미 있어요.`;
  }
  return null;
}

export function NewFolderDialog({ existingNames, pending, onConfirm, onClose }: NewFolderDialogProps) {
  const [name, setName] = useState('');
  // Validation is only revealed *after* the user attempts to submit. Mid-typing
  // is silent so the form doesn't feel like it's nagging.
  const [submitError, setSubmitError] = useState<string | null>(null);
  const trimmed = name.trim();
  const submitDisabled = trimmed.length === 0 || pending;

  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape' && !pending) onClose(); }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, pending]);

  function handleChange(value: string) {
    setName(value);
    // Editing wipes the prior submit feedback so the next attempt starts clean.
    if (submitError) setSubmitError(null);
  }

  function submit() {
    if (submitDisabled) return;
    const err = validate(name, existingNames);
    if (err) {
      setSubmitError(err);
      return;
    }
    onConfirm(trimmed);
  }

  // Track where mousedown started so a drag that ends on the backdrop doesn't
  // close the dialog. Without this, selecting text inside the input and
  // releasing outside dismisses the dialog (click fires on the common ancestor).
  const downOnBackdropRef = useRef(false);

  return (
    <div
      className="cw-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const wasDownOnBackdrop = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (!wasDownOnBackdrop) return;
        if (e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <form className="cw-dialog" onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label="close">
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>새 폴더</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 16px', fontSize: 13, lineHeight: 1.55 }}>
          현재 위치에 새 폴더를 만듭니다.
        </p>
        <label className="cw-field">
          <span>이름</span>
          <input
            autoFocus
            value={name}
            onChange={(e) => handleChange(e.target.value)}
            placeholder="예: Drafts"
            disabled={pending}
            aria-invalid={submitError !== null}
            aria-describedby={submitError ? 'cw-new-folder-error' : undefined}
          />
        </label>
        {submitError && (
          <div id="cw-new-folder-error" className="cw-dialog-warn" role="alert">
            <Icon name="x" size={12} /> {submitError}
          </div>
        )}
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 18 }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>취소</button>
          <button type="submit" className="cw-btn-primary" disabled={submitDisabled}>
            {pending ? '생성 중…' : '만들기'}
          </button>
        </div>
      </form>
    </div>
  );
}
