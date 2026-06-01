import { useRef, useState } from 'react';
import { Icon } from './Icon';
import type { BackendDirent } from '@/api/backend-types';
import { nameOf } from '@/domain/files';
import { useDialogEscape } from '@/lib/useDialogEscape';

interface RenameDialogProps {
  entry: BackendDirent;
  existingNames: string[];
  pending: boolean;
  onConfirm: (newFullName: string) => void;
  onClose: () => void;
}

function extOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot) : '';
}

function validate(raw: string, originalName: string, existing: string[]): string | null {
  const name = raw.trim();
  if (name.length === 0) return null;
  if (/[/\\]/.test(name)) return '이름에 / 또는 \\ 문자는 사용할 수 없습니다.';
  if (/^\.+$/.test(name)) return '. 만으로 이루어진 이름은 사용할 수 없습니다.';
  if (name === originalName) return '현재 이름과 동일합니다.';
  if (existing.some((e) => e.toLowerCase() === name.toLowerCase())) {
    return `"${name}" 이름이 이미 있습니다.`;
  }
  return null;
}

type Step = 'input' | 'confirm-ext';

export function RenameDialog({ entry, existingNames, pending, onConfirm, onClose }: RenameDialogProps) {
  const originalName = nameOf(entry);
  const originalExt = entry.kind === 'file' ? extOf(originalName) : '';

  const [value, setValue] = useState(originalName);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [step, setStep] = useState<Step>('input');

  const trimmed = value.trim();
  const submitDisabled = trimmed.length === 0 || pending;

  // The rename dialog has a two-step flow: ext-change confirmation rolls back
  // to the input step before fully closing. Both transitions are dispatched
  // from a single onClose-shaped callback so the modal-stack hook can drive
  // them uniformly.
  useDialogEscape(
    () => {
      if (step === 'confirm-ext') setStep('input');
      else onClose();
    },
    { disabled: pending },
  );

  function handleChange(v: string) {
    setValue(v);
    if (submitError) setSubmitError(null);
  }

  function submit() {
    if (submitDisabled) return;
    const err = validate(value, originalName, existingNames.filter((n) => n !== originalName));
    if (err) { setSubmitError(err); return; }

    // Warn only for files with changed extension.
    const newExt = entry.kind === 'file' ? extOf(trimmed) : '';
    if (entry.kind === 'file' && newExt.toLowerCase() !== originalExt.toLowerCase()) {
      setStep('confirm-ext');
      return;
    }
    onConfirm(trimmed);
  }

  const downOnBackdropRef = useRef(false);

  return (
    <div
      className="cw-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const wasDown = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (wasDown && e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <form className="cw-dialog" onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label="close">
          <Icon name="x" />
        </button>

        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>이름 변경</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 16px', fontSize: 13, lineHeight: 1.55 }}>
          현재 이름: <strong style={{ color: 'var(--cw-ink-2)' }}>{originalName}</strong>
        </p>

        {step === 'input' ? (
          <>
            <label className="cw-field">
              <span>새 이름</span>
              <input
                autoFocus
                value={value}
                onChange={(e) => handleChange(e.target.value)}
                disabled={pending}
                aria-invalid={submitError !== null}
                aria-describedby={submitError ? 'cw-rename-error' : undefined}
              />
            </label>
            {submitError && (
              <div id="cw-rename-error" className="cw-dialog-warn" role="alert">
                <Icon name="x" size={12} /> {submitError}
              </div>
            )}
            <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 18 }}>
              <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>취소</button>
              <button type="submit" className="cw-btn-primary" disabled={submitDisabled}>
                {pending ? '변경 중…' : '변경'}
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="cw-dialog-warn" role="alert" style={{ marginBottom: 16 }}>
              <Icon name="x" size={12} />
              {' '}확장자가 <strong>{originalExt || '(없음)'}</strong>에서{' '}
              <strong>{extOf(trimmed) || '(없음)'}</strong>으로 바뀝니다.
              파일이 제대로 열리지 않을 수 있습니다.
            </div>
            <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
              <button type="button" className="cw-btn-secondary" onClick={() => setStep('input')} disabled={pending}>
                돌아가기
              </button>
              <button
                type="button"
                className="cw-btn-primary"
                disabled={pending}
                onClick={() => onConfirm(trimmed)}
              >
                {pending ? '변경 중…' : '확장자 포함하여 변경'}
              </button>
            </div>
          </>
        )}
      </form>
    </div>
  );
}
