// Modal for creating a new folder at the current Files location.
// Replaces the previous window.prompt() with an in-design dialog that
// surfaces duplicate-name and invalid-character validation inline.

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';

interface NewFolderDialogProps {
  existingNames: string[];
  pending: boolean;
  onConfirm: (name: string) => void;
  onClose: () => void;
}

export function NewFolderDialog({ existingNames, pending, onConfirm, onClose }: NewFolderDialogProps) {
  const { t } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');

  const [name, setName] = useState('');
  // Validation is only revealed *after* the user attempts to submit. Mid-typing
  // is silent so the form doesn't feel like it's nagging.
  const [submitError, setSubmitError] = useState<string | null>(null);
  const trimmed = name.trim();
  const submitDisabled = trimmed.length === 0 || pending;

  function validate(raw: string, existing: string[]): string | null {
    const value = raw.trim();
    if (value.length === 0) return null;
    if (/[\\/]/.test(value)) return t('new_folder.validation.slash_blocked');
    if (/^\.+$/.test(value)) return t('new_folder.validation.dots_only');
    if (value.startsWith('.')) return t('new_folder.validation.starts_with_dot');
    if (existing.some((e) => e.toLowerCase() === value.toLowerCase())) {
      return t('new_folder.validation.duplicate', { name: value });
    }
    return null;
  }

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
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label={tCommon('actions.close')}>
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>{t('new_folder.title')}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 16px', fontSize: 13, lineHeight: 1.55 }}>
          {t('new_folder.description')}
        </p>
        <label className="cw-field">
          <span>{t('new_folder.name_label')}</span>
          <input
            autoFocus
            value={name}
            onChange={(e) => handleChange(e.target.value)}
            placeholder={t('new_folder.name_placeholder')}
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
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>{tCommon('actions.cancel')}</button>
          <button type="submit" className="cw-btn-primary" disabled={submitDisabled}>
            {pending ? t('new_folder.submitting') : t('new_folder.submit')}
          </button>
        </div>
      </form>
    </div>
  );
}
