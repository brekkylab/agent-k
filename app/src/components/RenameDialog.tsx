import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import type { BackendDirent } from '@/api/backend-types';
import { nameOf } from '@/domain/files';

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

type Step = 'input' | 'confirm-ext';

export function RenameDialog({ entry, existingNames, pending, onConfirm, onClose }: RenameDialogProps) {
  const { t } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');

  const originalName = nameOf(entry);
  const originalExt = entry.kind === 'file' ? extOf(originalName) : '';

  const [value, setValue] = useState(originalName);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [step, setStep] = useState<Step>('input');

  const trimmed = value.trim();
  const submitDisabled = trimmed.length === 0 || pending;

  function validate(raw: string, existing: string[]): string | null {
    const name = raw.trim();
    if (name.length === 0) return null;
    if (/[/\\]/.test(name)) return t('rename.validation.slash_blocked');
    if (/^\.+$/.test(name)) return t('rename.validation.dots_only');
    if (name === originalName) return t('rename.validation.same_as_current');
    if (existing.some((e) => e.toLowerCase() === name.toLowerCase())) {
      return t('rename.validation.duplicate', { name });
    }
    return null;
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape' && !pending) {
        if (step === 'confirm-ext') { setStep('input'); }
        else { onClose(); }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, pending, step]);

  function handleChange(v: string) {
    setValue(v);
    if (submitError) setSubmitError(null);
  }

  function submit() {
    if (submitDisabled) return;
    const err = validate(value, existingNames.filter((n) => n !== originalName));
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
  const fromExt = originalExt || t('rename.extension_unknown');
  const toExt = extOf(trimmed) || t('rename.extension_unknown');

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
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label={tCommon('actions.close')}>
          <Icon name="x" />
        </button>

        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>{t('rename.title')}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 16px', fontSize: 13, lineHeight: 1.55 }}>
          {t('rename.current_name')}: <strong style={{ color: 'var(--cw-ink-2)' }}>{originalName}</strong>
        </p>

        {step === 'input' ? (
          <>
            <label className="cw-field">
              <span>{t('rename.new_name')}</span>
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
              <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>{tCommon('actions.cancel')}</button>
              <button type="submit" className="cw-btn-primary" disabled={submitDisabled}>
                {pending ? t('rename.submitting') : t('rename.submit')}
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="cw-dialog-warn" role="alert" style={{ marginBottom: 16 }}>
              <Icon name="x" size={12} />
              {' '}{t('rename.extension_warning', { from: fromExt, to: toExt })}
            </div>
            <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
              <button type="button" className="cw-btn-secondary" onClick={() => setStep('input')} disabled={pending}>
                {t('rename.back')}
              </button>
              <button
                type="button"
                className="cw-btn-primary"
                disabled={pending}
                onClick={() => onConfirm(trimmed)}
              >
                {pending ? t('rename.submitting') : t('rename.force_submit')}
              </button>
            </div>
          </>
        )}
      </form>
    </div>
  );
}
