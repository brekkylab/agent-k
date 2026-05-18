// Invite a user to a project by username. The list-pick pattern from the
// wireframe needs an admin-level user directory API which backend-v2 doesn't
// expose for regular owners — so we follow Slack/Discord's username-entry
// pattern instead. Backend resolves username → user_id and 404s if unknown.

import { useEffect, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { addMember } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { ApiError } from '@/api/client';

interface InviteDialogProps {
  projectId: string;
  onClose: () => void;
}

export function InviteDialog({ projectId, onClose }: InviteDialogProps) {
  const queryClient = useQueryClient();
  const [username, setUsername] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape' && !mutation.isPending) onClose(); }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onClose]);

  const mutation = useMutation({
    mutationFn: (name: string) => addMember(projectId, name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['members', projectId] });
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
      setError('username을 입력해 주세요.');
      return;
    }
    mutation.mutate(cleaned);
  }

  const pending = mutation.isPending;

  return (
    <div className="cw-dialog-backdrop" role="dialog" aria-modal="true" onClick={(e) => { if (e.target === e.currentTarget && !pending) onClose(); }}>
      <div className="cw-dialog">
        <button className="cw-close" onClick={onClose} aria-label="close" disabled={pending}>
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 8px', fontSize: 18, letterSpacing: '-0.015em' }}>멤버 초대</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 4px', fontSize: 13, lineHeight: 1.6 }}>
          초대할 사용자의 username을 입력하세요. 가입된 사용자만 추가할 수 있습니다.
        </p>
        <p style={{ color: 'var(--cw-ink-4)', margin: '0 0 14px', fontSize: 11, fontFamily: 'var(--cw-font-mono)' }}>
          POST /projects/{projectId.slice(0, 8)}…/members
        </p>
        <form onSubmit={submit}>
          <div className="cw-field">
            <label>Username</label>
            <input
              value={username}
              onChange={(e) => { setUsername(e.target.value); setError(null); }}
              placeholder="username"
              autoFocus
              disabled={pending}
              autoComplete="off"
            />
          </div>
          {error && (
            <div className="cw-live-login-error" style={{ marginBottom: 12 }}>{error}</div>
          )}
          <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 14 }}>
            <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>취소</button>
            <button type="submit" className="cw-btn-primary" disabled={pending || !username.trim()}>
              {pending ? '초대 중…' : '초대'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function messageOf(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 404) return '해당 username을 가진 사용자를 찾을 수 없습니다.';
    if (err.status === 409) return '이미 이 프로젝트의 멤버입니다.';
    if (err.status === 403) return '멤버를 초대할 권한이 없습니다 (project owner만 가능).';
    return `${err.status} — ${err.message}`;
  }
  if (err instanceof Error) return err.message;
  return '초대에 실패했습니다.';
}
