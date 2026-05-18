// Project Home — markup mirrors app-live ProjectHome.

import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getProject, listMembers } from '@/api/projects';
import { createSession, deleteSession, listSessions } from '@/api/sessions';
import { listDirents } from '@/api/dirents';
import { Icon } from '@/components/Icon';
import { ActivityRow, AvatarStack, EmptyState, InfoRow, IntentIcon, SectionLabel, SharePill } from '@/components/uiPrimitives';
import { timeAgo } from '@/lib/timeAgo';
import { useToastStore } from '@/components/Toast';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { useAuthStore } from '@/stores/auth';
import { canAdministerSession } from '@/lib/permissions';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';
import type { Session } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectId/')({
  component: ProjectHome,
});

function ProjectHome() {
  const { projectId } = Route.useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const sessions = useQuery({ queryKey: ['sessions', projectId], queryFn: () => listSessions(projectId) });
  const members = useQuery({ queryKey: ['members', projectId], queryFn: () => listMembers(projectId) });
  const files = useQuery({
    queryKey: ['dirents', projectId, project.data?.name ?? ''],
    queryFn: () => listDirents(projectId, project.data?.name ?? 'project'),
    enabled: Boolean(project.data),
  });

  const newSessionMutation = useMutation({
    mutationFn: () => createSession(projectId),
    onSuccess: async (session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      showToast('새 세션이 만들어졌습니다');
      navigate({ to: '/projects/$projectId/sessions/$sessionId', params: { projectId, sessionId: session.id } });
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(`세션 생성 실패: ${msg}`);
    },
  });

  const [pendingDelete, setPendingDelete] = useState<Session | null>(null);
  const deleteMutation = useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      await queryClient.invalidateQueries({ queryKey: ['session', pendingDelete?.id] });
      showToast(`세션이 삭제되었습니다`);
      setPendingDelete(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError
        ? (err.status === 403 ? '삭제 권한이 없습니다 (creator 또는 project owner만 가능)' : err.message)
        : err instanceof Error ? err.message : 'delete failed';
      showToast(`세션 삭제 실패: ${msg}`);
    },
  });

  const memberList = members.data ?? [];
  const fileList = (files.data ?? []).filter((f) => f.type !== 'folder');
  const sessionList = sessions.data ?? [];

  return (
    <section className="cw-page cw-page-enter">
      <div className="cw-project-hero">
        <div>
          <h1>{project.data?.name ?? '...'}</h1>
          <p>{project.data?.description || '세션을 시작해 에이전트와 대화하세요.'}</p>
        </div>
        <div className="cw-hero-actions">
          <AvatarStack users={memberList} />
          <button className="cw-btn-primary" onClick={() => newSessionMutation.mutate()} disabled={newSessionMutation.isPending}>
            <Icon name="plus" /> {newSessionMutation.isPending ? '생성 중…' : 'New session'}
          </button>
        </div>
      </div>

      <div className="cw-project-summary">
        <InfoRow icon="folder-open" title={`${fileList.length} files`} meta="ground truth">
          Project Files can be selected, pinned, and cited in sessions.
        </InfoRow>
        <InfoRow icon="users" title={`${memberList.length} members`} meta="access">
          Shared sessions show who can talk with Cowork.
        </InfoRow>
      </div>

      <div className="cw-section-title">
        <SectionLabel>Sessions · {sessionList.length} visible to you</SectionLabel>
        <button onClick={() => navigate({ to: '/projects/$projectId/schedule', params: { projectId } })}>
          schedule 자동 발화
        </button>
      </div>

      {sessionList.length ? (
        <div className="cw-session-grid">
          {sessionList.map((session) => (
            <SessionCard
              key={session.id}
              session={session}
              canDelete={canAdministerSession(session, project.data, currentUser)}
              onOpen={() => navigate({
                to: '/projects/$projectId/sessions/$sessionId',
                params: { projectId, sessionId: session.id },
              })}
              onRequestDelete={() => setPendingDelete(session)}
            />
          ))}
        </div>
      ) : (
        <EmptyState
          title="No sessions yet"
          body="Start a session to make this project self-serve."
          action="New session"
          onAction={() => newSessionMutation.mutate()}
          chip="+"
        />
      )}

      <SectionLabel>Activity</SectionLabel>
      <div className="cw-activity-list">
        <ActivityRow title="프로젝트 동기화됨" date="방금 전">
          세션과 파일이 실시간으로 동기화되어 표시됩니다.
        </ActivityRow>
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
    </section>
  );
}

function SessionCard({
  session,
  canDelete,
  onOpen,
  onRequestDelete,
}: {
  session: Session;
  canDelete: boolean;
  onOpen: () => void;
  onRequestDelete: () => void;
}) {
  const isUnread = session.unreadCount > 0;
  const timeLabel = session.lastMessageAt ? timeAgo(session.lastMessageAt) : null;

  return (
    <div
      className={`cw-session-card${isUnread ? ' is-unread' : ''}`}
      onClick={onOpen}
      role="button"
      tabIndex={0}
      style={{ cursor: 'pointer' }}
    >
      <div className="cw-session-card-head">
        <span className="cw-session-card-title">
          <IntentIcon intent={session.intent} force />
          <SessionTitleText title={session.title} />
        </span>
        <span className="cw-session-right">
          {isUnread && (
            <span className="cw-unread-badge" aria-label={`unread ${session.unreadCount}`}>
              <span className="dot" />
              <span className="n">{session.unreadCount}</span>
            </span>
          )}
          <SharePill mode={session.shareMode} compact />
          {canDelete && <SessionCardMenu onDelete={onRequestDelete} />}
        </span>
      </div>
      {session.lastMessageSnippet && (
        <p className="cw-session-last">{session.lastMessageSnippet}</p>
      )}
      <div className="cw-session-card-footer">
        {timeLabel && <span className="cw-card-time">{timeLabel}</span>}
      </div>
    </div>
  );
}
