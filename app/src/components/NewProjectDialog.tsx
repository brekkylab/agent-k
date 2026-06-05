// New project creation dialog. Composes two existing patterns:
//   - InviteDialog: in-component mutation + invalidate + close
//   - NewFolderDialog: client-side name validation + backdrop drag guard
// On success we navigate straight into the created project so the user can
// start working immediately (matches Slack/Notion/Linear post-create UX).

import { useCallback, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { createProject, listProjects } from '@/api/projects';
import { ApiError } from '@/api/client';
import { Icon } from '@/components/Icon';
import { useDialogEscape } from '@/lib/useDialogEscape';

interface NewProjectDialogProps {
  existingNames: string[];
  onClose: () => void;
}

export function NewProjectDialog({ existingNames, onClose }: NewProjectDialogProps) {
  const { t } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');

  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [submitError, setSubmitError] = useState<string | null>(null);

  function validate(raw: string, existing: string[]): string | null {
    const value = raw.trim();
    if (value.length === 0) return null;
    if (value.length > 100) return t('new_project.validation.too_long');
    if (existing.some((e) => e.toLowerCase() === value.toLowerCase())) {
      return t('new_project.validation.duplicate', { name: value });
    }
    return null;
  }

  function messageOf(err: unknown): string {
    if (err instanceof ApiError) {
      if (err.status === 401) return t('new_project.errors.session_expired');
      if (err.status === 409) return t('new_project.errors.name_taken');
      if (err.status === 422 || err.status === 400) return t('new_project.errors.invalid');
      return `${err.status} — ${err.message}`;
    }
    if (err instanceof Error) return err.message;
    return t('new_project.errors.generic');
  }

  const mutation = useMutation({
    mutationFn: () => createProject({
      name: name.trim(),
      description: description.trim() || undefined,
    }),
    onSuccess: async (created) => {
      await queryClient.invalidateQueries({ queryKey: ['projects'] });
      onClose();
      navigate({ to: '/projects/$projectSlug', params: { projectSlug: created.slug } });
    },
    onError: (err) => {
      setSubmitError(messageOf(err));
    },
  });

  const pending = mutation.isPending;
  const trimmed = name.trim();
  const submitDisabled = trimmed.length === 0 || pending;

  useDialogEscape(onClose, { disabled: pending });

  function submit() {
    if (submitDisabled) return;
    const err = validate(name, existingNames);
    if (err) {
      setSubmitError(err);
      return;
    }
    setSubmitError(null);
    mutation.mutate();
  }

  const downOnBackdropRef = useRef(false);

  // Portal escapes any ancestor stacking context (e.g. cw-page-enter's
  // animation transform), guaranteeing the backdrop fills the viewport
  // regardless of where in the tree this dialog is rendered.
  return createPortal(
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
        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>{t('new_project.title')}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 16px', fontSize: 13, lineHeight: 1.55 }}>
          {t('new_project.description')}
        </p>
        <label className="cw-field">
          <span>{t('new_project.name_label')}</span>
          <input
            autoFocus
            value={name}
            onChange={(e) => { setName(e.target.value); if (submitError) setSubmitError(null); }}
            placeholder={t('new_project.name_placeholder')}
            disabled={pending}
            aria-invalid={submitError !== null}
            aria-describedby={submitError ? 'cw-new-project-error' : undefined}
            maxLength={100}
          />
        </label>
        <label className="cw-field">
          <span>{t('new_project.description_label')} <span style={{ color: 'var(--cw-ink-4)', fontWeight: 400 }}>{t('new_project.optional')}</span></span>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t('new_project.description_placeholder')}
            disabled={pending}
            rows={3}
            style={{ resize: 'vertical', minHeight: 72, fontFamily: 'inherit' }}
          />
        </label>
        {submitError && (
          <div id="cw-new-project-error" className="cw-dialog-warn" role="alert">
            <Icon name="x" size={12} /> {submitError}
          </div>
        )}
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 18 }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>{tCommon('actions.cancel')}</button>
          <button type="submit" className="cw-btn-primary" disabled={submitDisabled}>
            {pending ? t('new_project.submitting') : t('new_project.submit')}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}

// Convenience hook so multiple entry points (sidebar, projects page header, …)
// can share state + dialog rendering without each duplicating
// `existingNames` derivation and useState wiring. Returns the imperative
// `open()` and the declarative `dialog` ReactNode to drop into JSX.
export function useNewProjectDialog(): { open: () => void; dialog: ReactNode } {
  const [isOpen, setIsOpen] = useState(false);
  const projects = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  const dialog = isOpen ? (
    <NewProjectDialog
      existingNames={(projects.data ?? []).map((p) => p.name)}
      onClose={close}
    />
  ) : null;
  return { open, dialog };
}
