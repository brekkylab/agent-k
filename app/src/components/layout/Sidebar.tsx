// Sidebar — markup follows app-live exactly: brand lockup, project nav, active
// project block (Home/Files/Skills/Schedule/Members/Settings), Sessions rail,
// sidebar-user. Only the routing API is swapped to TanStack Router.

import { useState } from 'react';
import { useNavigate, useRouterState } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import logoMark from '@/assets/logo-mark.svg';
import { listProjects } from '@/api/projects';
import { createSession, deleteSession, listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { Avatar, IconPocket, SectionLabel } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/components/Toast';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { canAdministerSession } from '@/lib/permissions';
import { ApiError } from '@/api/client';
import type { Session } from '@/domain/types';

export function Sidebar() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const projectsQuery = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  const urlProjectId = useActiveProjectId();
  // /projects URL에서도 첫 프로젝트의 sub-nav를 보여주기 위해 default fallback.
  const activeProjectId = urlProjectId ?? projectsQuery.data?.[0]?.id ?? null;
  const activeProject = (projectsQuery.data ?? []).find((p) => p.id === activeProjectId);

  const sessionsQuery = useQuery({
    queryKey: ['sessions', activeProjectId],
    queryFn: () => listSessions(activeProjectId!),
    enabled: Boolean(activeProjectId),
  });

  const activeSessionId = useActiveSessionId();
  const activeRoute = useActiveRouteKey();

  const createSessionMutation = useMutation({
    mutationFn: (projectId: string) => createSession(projectId),
    onSuccess: async (session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', session.projectId] });
      showToast('새 세션이 만들어졌습니다');
      navigate({
        to: '/projects/$projectId/sessions/$sessionId',
        params: { projectId: session.projectId, sessionId: session.id },
      });
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(`세션 생성 실패: ${msg}`);
    },
  });

  const [pendingDelete, setPendingDelete] = useState<Session | null>(null);
  const deleteMutation = useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId),
    onSuccess: async (_, deletedId) => {
      const deletedProjectId = pendingDelete?.projectId ?? activeProjectId;
      if (deletedProjectId) {
        await queryClient.invalidateQueries({ queryKey: ['sessions', deletedProjectId] });
      }
      await queryClient.invalidateQueries({ queryKey: ['session', deletedId] });
      // 지금 보고 있던 세션이 사라졌다면 project home으로 돌려보냄.
      if (activeSessionId === deletedId && activeProjectId) {
        navigate({ to: '/projects/$projectId', params: { projectId: activeProjectId } });
      }
      showToast('세션이 삭제되었습니다');
      setPendingDelete(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError
        ? (err.status === 403 ? '삭제 권한이 없습니다 (creator 또는 project owner만 가능)' : err.message)
        : err instanceof Error ? err.message : 'delete failed';
      showToast(`세션 삭제 실패: ${msg}`);
    },
  });

  function openProject(id: string) {
    navigate({ to: '/projects/$projectId', params: { projectId: id } });
  }

  function openSession(projectId: string, sessionId: string) {
    navigate({ to: '/projects/$projectId/sessions/$sessionId', params: { projectId, sessionId } });
  }

  return (
    <aside className="cw-sidebar-app">
      <button
        className="cw-brand-lockup"
        onClick={() => navigate({ to: '/projects' })}
        aria-label="Cowork projects"
      >
        <img src={logoMark} alt="" />
        <strong>Cowork</strong>
      </button>

      <SectionLabel>PROJECTS</SectionLabel>
      <nav className="cw-nav-list">
        {(projectsQuery.data ?? []).map((item) => (
          <button
            key={item.id}
            className={`cw-nav-row cw-project-nav-row ${item.id === activeProjectId ? 'is-active' : ''}`}
            onClick={() => openProject(item.id)}
          >
            <span className="cw-project-swatch" />
            <span>{item.name}</span>
          </button>
        ))}
        <button
          className="cw-nav-row is-ghost"
          onClick={() => showToast('새 프로젝트 생성 다이얼로그는 곧 추가됩니다')}
        >
          <span className="cw-project-swatch is-plus">+</span>
          <span>새 Project</span>
        </button>
      </nav>

      {activeProject && (
        <div className="cw-project-block">
          <div className="cw-project-name">{activeProject.name}</div>
          <button
            className={`cw-nav-row ${activeRoute === 'project' ? 'is-active' : ''}`}
            onClick={() => openProject(activeProject.id)}
          >
            <IconPocket tone="home" icon="home" /> Home
          </button>
          <button
            className={`cw-nav-row ${activeRoute === 'files' ? 'is-active' : ''}`}
            onClick={() => navigate({ to: '/projects/$projectId/files', params: { projectId: activeProject.id } })}
          >
            <IconPocket tone="files" icon="folder-open" /> Files
          </button>
          <button
            className={`cw-nav-row ${activeRoute === 'skills' ? 'is-active' : ''}`}
            onClick={() => navigate({ to: '/projects/$projectId/skills', params: { projectId: activeProject.id } })}
          >
            <IconPocket tone="skills" icon="zap" /> Skills
          </button>
          <button
            className={`cw-nav-row ${activeRoute === 'schedule' ? 'is-active' : ''}`}
            onClick={() => navigate({ to: '/projects/$projectId/schedule', params: { projectId: activeProject.id } })}
          >
            <IconPocket tone="schedule" icon="calendar" /> Schedule
          </button>
          <button
            className={`cw-nav-row ${activeRoute === 'members' ? 'is-active' : ''}`}
            onClick={() => navigate({ to: '/projects/$projectId/members', params: { projectId: activeProject.id } })}
          >
            <IconPocket tone="members" icon="users" /> Members
          </button>
          <button
            className={`cw-nav-row ${activeRoute === 'settings' ? 'is-active' : ''}`}
            onClick={() => navigate({ to: '/projects/$projectId/settings', params: { projectId: activeProject.id } })}
          >
            <IconPocket tone="settings" icon="settings" /> Settings
          </button>
        </div>
      )}

      {activeProject && (
        <>
          <SectionLabel>Sessions</SectionLabel>
          <div className="cw-session-rail">
            {(sessionsQuery.data ?? []).map((session) => {
              const canDelete = canAdministerSession(session, activeProject, currentUser);
              return (
                <div
                  key={session.id}
                  className={`cw-session-row ${session.id === activeSessionId ? 'is-active' : ''}`}
                  onClick={() => openSession(activeProject.id, session.id)}
                  role="button"
                  tabIndex={0}
                  style={{ cursor: 'pointer' }}
                >
                  <IconPocket tone="trust" icon="message-square" compact />
                  <span>{session.title}</span>
                  {session.isAutoAppend && <span className="auto-dot">●</span>}
                  {canDelete && (
                    <span style={{ marginLeft: 'auto' }}>
                      <SessionCardMenu onDelete={() => setPendingDelete(session)} />
                    </span>
                  )}
                </div>
              );
            })}
            <button
              className="cw-session-row is-new"
              onClick={() => activeProject && createSessionMutation.mutate(activeProject.id)}
              disabled={createSessionMutation.isPending}
            >
              <IconPocket tone="add" icon="plus" compact />
              <span>{createSessionMutation.isPending ? '생성 중…' : '새 session'}</span>
            </button>
          </div>
        </>
      )}

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

      {currentUser && (
        <div className="cw-sidebar-user">
          <Avatar user={currentUser} />
          <div className="cw-sidebar-user-meta">
            <b>{currentUser.name.split(' ')[0]}</b>
          </div>
          <button
            aria-label="logout"
            onClick={() => { useAuthStore.getState().reset(); window.location.href = '/login'; }}
            style={{ border: 0, background: 'transparent', padding: 0, color: 'var(--cw-ink-3)', cursor: 'pointer' }}
          >
            <Icon name="more" />
          </button>
        </div>
      )}
    </aside>
  );
}

function useActiveProjectId(): string | null {
  return useParamFromMatches('projectId');
}

function useActiveSessionId(): string | null {
  return useParamFromMatches('sessionId');
}

function useParamFromMatches(key: string): string | null {
  const state = useRouterState();
  for (const match of state.matches) {
    const params = match.params as Record<string, string | undefined>;
    if (params[key]) return params[key]!;
  }
  return null;
}

function useActiveRouteKey(): 'project' | 'files' | 'skills' | 'schedule' | 'members' | 'settings' | 'session' | 'projects' {
  const state = useRouterState();
  const path = state.location.pathname;
  if (path.includes('/sessions/')) return 'session';
  if (path.endsWith('/files')) return 'files';
  if (path.endsWith('/skills')) return 'skills';
  if (path.endsWith('/schedule')) return 'schedule';
  if (path.endsWith('/members')) return 'members';
  if (path.endsWith('/settings')) return 'settings';
  if (path.match(/\/projects\/[^/]+$/)) return 'project';
  return 'projects';
}
