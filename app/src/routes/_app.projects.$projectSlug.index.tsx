// Project Home — markup mirrors app-live ProjectHome.

import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
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
import { shortSessionId } from '@/lib/sessionId';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';
import { loadNs } from '@/i18n/loader';
import type { Session } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectSlug/')({
  // SessionCardMenu uses `session`. `project`/`common` come from parents.
  loader: () => loadNs('session'),
  component: ProjectHome,
});

function ProjectHome() {
  const { projectSlug } = Route.useParams();
  const { t } = useTranslation('project');
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const sessions = useQuery({ queryKey: ['sessions', projectSlug], queryFn: () => listSessions(projectSlug) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });
  // Dirents are scope-based and keyed by the resolved project UUID (not the slug).
  const resolvedProjectId = project.data?.id;
  const files = useQuery({
    queryKey: ['dirents', 'shared', resolvedProjectId, project.data?.name ?? ''],
    queryFn: () =>
      listDirents({ kind: 'shared', projectId: resolvedProjectId! }, project.data?.name ?? 'project'),
    enabled: Boolean(project.data),
  });

  const newSessionMutation = useMutation({
    mutationFn: () => createSession(projectSlug),
    onSuccess: async (session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
      showToast(t('toast.session_created'));
      navigate({ to: '/projects/$projectSlug/sessions/$sessionPrefix', params: { projectSlug, sessionPrefix: shortSessionId(session.id) } });
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(t('toast.session_create_failed', { message: msg }));
    },
  });

  const [pendingDelete, setPendingDelete] = useState<Session | null>(null);
  const deleteMutation = useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
      await queryClient.invalidateQueries({ queryKey: ['session', pendingDelete?.id] });
      showToast(t('toast.session_deleted'));
      setPendingDelete(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError
        ? (err.status === 403 ? t('toast.no_delete_permission') : err.message)
        : err instanceof Error ? err.message : 'delete failed';
      showToast(t('toast.session_delete_failed', { message: msg }));
    },
  });

  const memberList = members.data ?? [];
  const fileList = (files.data ?? []).filter((f) => f.type !== 'folder');
  const sessionList = (sessions.data ?? []).filter((s) => s.origin === 'user');

  return (
    <section className="cw-page cw-page-enter">
      <div className="cw-project-hero">
        <div>
          <h1>{project.data?.name ?? '...'}</h1>
          <p>{project.data?.description || t('subtitle_fallback')}</p>
        </div>
        <div className="cw-hero-actions">
          <AvatarStack users={memberList} />
          <button className="cw-btn-primary" onClick={() => newSessionMutation.mutate()} disabled={newSessionMutation.isPending}>
            <Icon name="plus" /> {newSessionMutation.isPending ? t('creating') : t('new_session')}
          </button>
        </div>
      </div>

      <div className="cw-project-summary">
        <InfoRow icon="folder-open" title={t('info.files_title', { count: fileList.length })} meta={t('info.files_meta')}>
          {t('info.files_body')}
        </InfoRow>
        <InfoRow icon="users" title={t('info.members_title', { count: memberList.length })} meta={t('info.members_meta')}>
          {t('info.members_body')}
        </InfoRow>
      </div>

      <div className="cw-section-title">
        <SectionLabel>{t('sessions_section_label', { count: sessionList.length })}</SectionLabel>
        <button onClick={() => navigate({ to: '/projects/$projectSlug/schedule', params: { projectSlug } })}>
          {t('schedule_auto')}
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
                to: '/projects/$projectSlug/sessions/$sessionPrefix',
                params: { projectSlug, sessionPrefix: shortSessionId(session.id) },
              })}
              onRequestDelete={() => setPendingDelete(session)}
            />
          ))}
        </div>
      ) : (
        <EmptyState
          title={t('empty.title')}
          body={t('empty.body')}
          action={t('empty.action')}
          onAction={() => newSessionMutation.mutate()}
          chip="+"
        />
      )}

      <SectionLabel>{t('activity.section_label')}</SectionLabel>
      <div className="cw-activity-list">
        <ActivityRow title={t('activity.sync_title')} date={t('activity.sync_when')}>
          {t('activity.sync_body')}
        </ActivityRow>
      </div>

      {pendingDelete && (
        <ConfirmDialog
          title={t('delete_session.title')}
          body={t('delete_session.body', { title: pendingDelete.title })}
          confirmLabel={t('delete_session.confirm')}
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
