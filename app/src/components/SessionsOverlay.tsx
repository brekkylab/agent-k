// "View all" sessions overlay — a wide modal floating over the main area, opened
// from the sidebar SESSIONS header. Shows every session of a project as a card
// grid. Self-contained: takes only projectSlug and handles its own data + delete.

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProject } from '@/api/projects';
import { listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { EmptyState } from '@/components/uiPrimitives';
import { SessionCard } from '@/components/SessionCard';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useAuthStore } from '@/stores/auth';
import { canAdministerSession } from '@/lib/permissions';
import { shortSessionId } from '@/lib/sessionId';
import { useSessionDelete } from '@/lib/useSessionDelete';
import type { Session } from '@/domain/types';

export function SessionsOverlay({ projectSlug, onClose }: { projectSlug: string; onClose: () => void }) {
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const sessions = useQuery({ queryKey: ['sessions', projectSlug], queryFn: () => listSessions(projectSlug) });

  // Move focus to the close button on mount so keyboard users can dismiss immediately.
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  // Close on Escape.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const [pendingDelete, setPendingDelete] = useState<Session | null>(null);
  const deleteMutation = useSessionDelete(projectSlug, { onDeleted: () => setPendingDelete(null) });

  const sessionList = (sessions.data ?? []).filter((s) => s.origin === 'user');

  // Render via portal to document.body so `position: fixed` is relative to the
  // viewport — not the sidebar (whose transform would otherwise trap it).
  return createPortal(
    <>
      <div className="cw-overlay-backdrop" onClick={onClose} aria-hidden="true" />
      <div className="cw-sessions-overlay" role="dialog" aria-modal="true" aria-label="All sessions">
        <div className="cw-overlay-head">
          <h2>Sessions · {sessionList.length}</h2>
          <button ref={closeButtonRef} type="button" className="cw-overlay-close" onClick={onClose} aria-label="닫기">
            <Icon name="x" size={18} />
          </button>
        </div>
        <div className="cw-overlay-body cw-scroll-quiet">
          {sessionList.length ? (
            <div className="cw-session-grid">
              {sessionList.map((session) => (
                <SessionCard
                  key={session.id}
                  session={session}
                  canDelete={canAdministerSession(session, project.data, currentUser)}
                  onOpen={() => {
                    navigate({
                      to: '/projects/$projectSlug/sessions/$sessionPrefix',
                      params: { projectSlug, sessionPrefix: shortSessionId(session.id) },
                    });
                    onClose();
                  }}
                  onRequestDelete={() => setPendingDelete(session)}
                />
              ))}
            </div>
          ) : (
            <EmptyState title="No sessions yet" body="새 대화를 시작하면 여기에 쌓입니다." chip="+" />
          )}
        </div>
      </div>
      {pendingDelete && (
        <ConfirmDialog
          title="세션을 삭제하시겠어요?"
          body={`"${pendingDelete.title}"의 모든 메시지와 sandbox 자원이 함께 정리됩니다. 이 작업은 되돌릴 수 없습니다.`}
          confirmLabel="삭제"
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate(pendingDelete.id)}
          onClose={() => setPendingDelete(null)}
        />
      )}
    </>,
    document.body,
  );
}
