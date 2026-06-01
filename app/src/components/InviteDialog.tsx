// Invite a user to a project by username. The list-pick pattern from the
// wireframe needs an admin-level user directory API which backend-v2 doesn't
// expose for regular owners — so we follow Slack/Discord's username-entry
// pattern instead. Backend resolves username → user_id and 404s if unknown.

import { useEffect, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { addMember } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { ApiError } from '@/api/client';

interface InviteDialogProps {
  projectRef: string;
  onClose: () => void;
}

export function InviteDialog({ projectRef, onClose }: InviteDialogProps) {
  const { t } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');


  const queryClient = useQueryClient();
  const [username, setUsername] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape' && !mutation.isPending) onClose(); }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onClose]);

  function messageOf(err: unknown): string {
    if (err instanceof ApiError) {
      if (err.status === 404) return t('invite.errors.not_found');
      if (err.status === 409) return t('invite.errors.already_member');
      if (err.status === 403) return t('invite.errors.forbidden');
      return `${err.status} — ${err.message}`;
    }
    if (err instanceof Error) return err.message;
    return t('invite.errors.generic');
  }

  const mutation = useMutation({
    mutationFn: (name: string) => addMember(projectRef, name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['members', projectRef] });
      onClose();
    },
    onError: (err) => {
      setError(messageOf(err));
    },
  });

  function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const cleaned = username.trim();
    if (!cleaned) {
      setError(t('invite.validation.username_required'));
      return;
    }
    mutation.mutate(cleaned);
  }

  const pending = mutation.isPending;

  return (
    <div className="cw-dialog-backdrop" role="dialog" aria-modal="true" onClick={(e) => { if (e.target === e.currentTarget && !pending) onClose(); }}>
      <div className="cw-dialog">
        <button className="cw-close" onClick={onClose} aria-label={tCommon('actions.close')} disabled={pending}>
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 8px', fontSize: 18, letterSpacing: '-0.015em' }}>{t('invite.title')}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 4px', fontSize: 13, lineHeight: 1.6 }}>
          {t('invite.description')}
        </p>
        <p style={{ color: 'var(--cw-ink-4)', margin: '0 0 14px', fontSize: 11, fontFamily: 'var(--cw-font-mono)' }}>
          POST /projects/{projectRef}/members
        </p>
        <form onSubmit={submit}>
          <div className="cw-field">
            <label>Username</label>
            <input
              value={username}
              onChange={(e) => { setUsername(e.target.value); setError(null); }}
              placeholder={t('invite.username_placeholder')}
              autoFocus
              disabled={pending}
              autoComplete="off"
            />
          </div>
          {error && (
            <div className="cw-live-login-error" style={{ marginBottom: 12 }}>{error}</div>
          )}
          <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 14 }}>
            <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>{tCommon('actions.cancel')}</button>
            <button type="submit" className="cw-btn-primary" disabled={pending || !username.trim()}>
              {pending ? t('invite.submitting') : t('invite.submit')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
