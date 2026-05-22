// Sidebar — header(brand) + 단일 스크롤 본문(PROJECTS section, active project
// menu, Sessions section) + user-area 푸터. 폭은 사용자가 오른쪽 가장자리
// resizer로 직접 조절(layout store에 persist). PROJECTS/Sessions는 sticky
// SectionHeader를 통해 펼치고 접을 수 있고, 접힘 상태에서도 active 항목과
// (Sessions 한정) unread 항목은 유지된다.

import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate, useRouterState } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import logoMark from '@/assets/logo-mark.svg';
import { listProjects } from '@/api/projects';
import { createSession, deleteSession, listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { Avatar, IconPocket } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import {
  getSidebarModeForDrag,
  getSidebarModeWhileResizing,
  isSidebarRevealHoldPoint,
  shouldCloseSidebarRevealOnNavigation,
  shouldRevealSidebarAfterDrag,
  useLayoutStore,
  SIDEBAR_REVEAL_WIDTH,
} from '@/stores/layout';
import { useToastStore } from '@/components/Toast';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useNewProjectDialog } from '@/components/NewProjectDialog';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { canAdministerSession } from '@/lib/permissions';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';
import type { Session } from '@/domain/types';

function SidebarResizer({ setRevealed }: { setRevealed: (revealed: boolean) => void }) {
  const setSidebarMode = useLayoutStore((s) => s.setSidebarMode);
  const setExpandedWidth = useLayoutStore((s) => s.setExpandedWidth);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      const startX = e.clientX;
      const { sidebarMode, expandedWidth } = useLayoutStore.getState();
      // When hidden, the floating overlay shows at REVEAL_WIDTH, so the user's hand
      // is already at that x-coordinate — start from there so dragging is 1:1.
      const hiddenAtStart = sidebarMode === 'hidden';
      const startW = sidebarMode === 'hidden' ? SIDEBAR_REVEAL_WIDTH : expandedWidth;
      let lastW = startW;
      let lastClientX = e.clientX;
      let lastClientY = e.clientY;
      document.body.classList.add('is-resizing-sidebar');

      function onMove(ev: PointerEvent) {
        const w = startW + (ev.clientX - startX);
        const modeAfterRelease = getSidebarModeForDrag(w, hiddenAtStart);
        const modeWhileResizing = getSidebarModeWhileResizing(modeAfterRelease);
        lastW = w;
        lastClientX = ev.clientX;
        lastClientY = ev.clientY;
        setRevealed(false);
        if (modeWhileResizing === 'hidden') {
          setSidebarMode('hidden');
        } else {
          setSidebarMode('expanded');
          setExpandedWidth(w);  // store clamps to [MIN, MAX]
        }
      }
      function onUp() {
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        document.body.classList.remove('is-resizing-sidebar');
        const modeAfterRelease = getSidebarModeForDrag(lastW, hiddenAtStart);
        setSidebarMode(modeAfterRelease);
        setRevealed(
          shouldRevealSidebarAfterDrag(
            modeAfterRelease,
            lastClientX,
            lastClientY,
            window.innerHeight,
          ),
        );
      }
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
    },
    [setRevealed, setSidebarMode, setExpandedWidth],
  );

  return (
    <div
      className="cw-sidebar-resizer"
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation="vertical"
      aria-label="사이드바 폭 조절"
    />
  );
}

function SectionHeader({
  label,
  expanded,
  onToggle,
  onAdd,
  addLabel,
  addDisabled,
}: {
  label: string;
  expanded: boolean;
  onToggle: () => void;
  onAdd?: () => void;
  addLabel?: string;
  addDisabled?: boolean;
}) {
  return (
    <div className="cw-section-header">
      <button
        type="button"
        className="cw-section-toggle"
        onClick={onToggle}
        aria-expanded={expanded}
        aria-controls={`section-${label}`}
      >
        <Icon
          name="chevron-right"
          size={12}
          className={`cw-section-chevron ${expanded ? 'is-open' : ''}`}
        />
        <span>{label}</span>
      </button>
      {onAdd && (
        <button
          type="button"
          className="cw-section-add"
          onClick={(e) => { e.stopPropagation(); onAdd(); }}
          disabled={addDisabled}
          aria-label={addLabel ?? `${label} 추가`}
          title={addLabel ?? `${label} 추가`}
        >
          <Icon name="plus" size={14} />
        </button>
      )}
    </div>
  );
}

export function Sidebar() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);
  const sidebarMode = useLayoutStore((s) => s.sidebarMode);
  const projectsExpanded = useLayoutStore((s) => s.projectsExpanded);
  const sessionsExpanded = useLayoutStore((s) => s.sessionsExpanded);
  const toggleProjects = useLayoutStore((s) => s.toggleProjects);
  const toggleSessions = useLayoutStore((s) => s.toggleSessions);
  const [revealed, setRevealed] = useState(false);
  // Grace delay before closing on mouse-leave so brief excursions outside the
  // floating sidebar don't snap it shut. ~250ms feels intentional but not sticky.
  const closeTimerRef = useRef<number | null>(null);
  const shouldHoldReveal = useCallback((clientX: number, clientY: number) => (
    isSidebarRevealHoldPoint(clientX, clientY, window.innerHeight)
  ), []);
  const cancelClose = useCallback(() => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);
  const openReveal = useCallback(() => {
    cancelClose();
    setRevealed(true);
  }, [cancelClose]);
  const scheduleClose = useCallback((point?: { clientX: number; clientY: number }) => {
    cancelClose();
    if (point && shouldHoldReveal(point.clientX, point.clientY)) return;
    closeTimerRef.current = window.setTimeout(() => setRevealed(false), 250);
  }, [cancelClose, shouldHoldReveal]);
  useEffect(() => cancelClose, [cancelClose]);

  useEffect(() => {
    if (sidebarMode !== 'hidden' || !revealed) return;

    function onPointerMove(ev: PointerEvent) {
      if (shouldHoldReveal(ev.clientX, ev.clientY)) {
        cancelClose();
      } else {
        scheduleClose();
      }
    }

    window.addEventListener('pointermove', onPointerMove);
    return () => window.removeEventListener('pointermove', onPointerMove);
  }, [cancelClose, revealed, scheduleClose, shouldHoldReveal, sidebarMode]);

  const projectsQuery = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  // URL에 projectId가 있을 때만 활성 프로젝트로 인정한다 — /projects 같은 곳에서는
  // sub-nav(Home/Files/.../Sessions)가 보이지 않아야 사용자 멘탈 모델과 일치.
  const activeProjectId = useActiveProjectId();
  const activeProject = (projectsQuery.data ?? []).find((p) => p.id === activeProjectId);

  const sessionsQuery = useQuery({
    queryKey: ['sessions', activeProjectId],
    queryFn: () => listSessions(activeProjectId!),
    enabled: Boolean(activeProjectId),
  });

  const activeSessionId = useActiveSessionId();
  const activeRoute = useActiveRouteKey();

  // Close on navigation for mobile drawers, but keep desktop hidden/reveal open
  // so sidebar clicks don't immediately tuck it away.
  useEffect(() => {
    if (shouldCloseSidebarRevealOnNavigation(sidebarMode, window.innerWidth)) {
      setRevealed(false);
    }
  }, [activeProjectId, activeSessionId, activeRoute, sidebarMode]);

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
  const projectCreator = useNewProjectDialog();
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
    <>
      {sidebarMode === 'hidden' && (
        <div
          className="cw-sidebar-edge-trigger"
          onPointerEnter={openReveal}
          aria-hidden="true"
        />
      )}
      <button
        type="button"
        className="cw-sidebar-hamburger"
        onClick={() => setRevealed((v) => !v)}
        aria-label={revealed ? '사이드바 닫기' : '사이드바 열기'}
        aria-expanded={revealed}
      >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" width={18} height={18} aria-hidden="true">
          <line x1="4" x2="20" y1="6" y2="6" /><line x1="4" x2="20" y1="12" y2="12" /><line x1="4" x2="20" y1="18" y2="18" />
        </svg>
      </button>
      {revealed && (
        <div
          className="cw-sidebar-backdrop"
          onClick={() => setRevealed(false)}
          aria-hidden="true"
        />
      )}
      <aside
        className={`cw-sidebar-app${revealed ? ' is-revealed' : ''}`}
        onPointerEnter={() => { if (sidebarMode === 'hidden') openReveal(); }}
        onPointerLeave={(e) => {
          if (sidebarMode === 'hidden') scheduleClose({ clientX: e.clientX, clientY: e.clientY });
        }}
      >
      <div className="cw-sidebar-header">
        <button
          className="cw-brand-lockup"
          onClick={() => navigate({ to: '/projects' })}
          aria-label="Cowork projects"
        >
          <img src={logoMark} alt="" />
          <strong>Cowork</strong>
        </button>
      </div>

      <div className="cw-sidebar-scroll cw-scroll-quiet">
        <SectionHeader
          label="PROJECTS"
          expanded={projectsExpanded}
          onToggle={toggleProjects}
          onAdd={projectCreator.open}
          addLabel="새 Project"
        />
        <nav className="cw-projects-list" data-expanded={projectsExpanded ? 'true' : 'false'}>
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
        </nav>

        {activeProject && (
          <div className="cw-project-block">
            <div className="cw-project-name">{activeProject.name}</div>
            <button
              className={`cw-nav-row ${activeRoute === 'project' ? 'is-active' : ''}`}
              onClick={() => openProject(activeProject.id)}
            >
              <IconPocket tone="home" icon="home" /> <span>Home</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'files' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectId/files', params: { projectId: activeProject.id } })}
            >
              <IconPocket tone="files" icon="folder-open" /> <span>Files</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'skills' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectId/skills', params: { projectId: activeProject.id } })}
            >
              <IconPocket tone="skills" icon="zap" /> <span>Skills</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'schedule' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectId/schedule', params: { projectId: activeProject.id } })}
            >
              <IconPocket tone="schedule" icon="calendar" /> <span>Schedule</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'members' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectId/members', params: { projectId: activeProject.id } })}
            >
              <IconPocket tone="members" icon="users" /> <span>Members</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'settings' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectId/settings', params: { projectId: activeProject.id } })}
            >
              <IconPocket tone="settings" icon="settings" /> <span>Settings</span>
            </button>
          </div>
        )}

        {activeProject && (
          <>
            <SectionHeader
              label="Sessions"
              expanded={sessionsExpanded}
              onToggle={toggleSessions}
              onAdd={() => createSessionMutation.mutate(activeProject.id)}
              addLabel={createSessionMutation.isPending ? '세션 생성 중…' : '새 Session'}
              addDisabled={createSessionMutation.isPending}
            />
            <div className="cw-sessions-list" data-expanded={sessionsExpanded ? 'true' : 'false'}>
              {(sessionsQuery.data ?? []).map((session) => {
                const canDelete = canAdministerSession(session, activeProject, currentUser);
                return (
                  <div
                    key={session.id}
                    className={[
                      'cw-session-row',
                      session.id === activeSessionId ? 'is-active' : '',
                      session.unreadCount > 0 ? 'is-unread' : '',
                    ].filter(Boolean).join(' ')}
                    onClick={() => openSession(activeProject.id, session.id)}
                    role="button"
                    tabIndex={0}
                    style={{ cursor: 'pointer' }}
                  >
                    {session.unreadCount > 0 ? (
                      <span className="cw-unread-badge" aria-label={`unread ${session.unreadCount}`}>
                        <span className="n">{session.unreadCount}</span>
                      </span>
                    ) : (
                      <IconPocket tone="trust" icon="message-square" compact />
                    )}
                    <SessionTitleText title={session.title} />
                    {session.isAutoAppend && <span className="auto-dot">●</span>}
                    {canDelete && (
                      <span className="cw-session-menu-wrap">
                        <SessionCardMenu onDelete={() => setPendingDelete(session)} />
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>

      {currentUser && (
        <div className="cw-sidebar-user-area">
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
        </div>
      )}

      <SidebarResizer setRevealed={setRevealed} />

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

      {projectCreator.dialog}
      </aside>
    </>
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
