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
  isSidebarRevealHoldPoint,
  useLayoutStore,
  SIDEBAR_MOBILE_BREAKPOINT,
} from '@/stores/layout';
import { useToastStore } from '@/components/Toast';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useNewProjectDialog } from '@/components/NewProjectDialog';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { canAdministerSession } from '@/lib/permissions';
import { shortSessionId } from '@/lib/sessionId';
import { useDuplicateSession } from '@/lib/useDuplicateSession';
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
      // Resizer is only mounted in expanded mode (CSS hides it when hidden),
      // so we always start from the persisted expandedWidth — no hidden branch.
      const startW = useLayoutStore.getState().expandedWidth;
      let lastClientX = e.clientX;
      document.body.classList.add('is-resizing-sidebar');

      function onMove(ev: PointerEvent) {
        lastClientX = ev.clientX;
        const w = startW + (ev.clientX - startX);
        const currentMode = useLayoutStore.getState().sidebarMode;
        if (currentMode === 'hidden') {
          // Option B in hidden: drag adjusts the floating panel width only —
          // it must NOT auto-pin back to expanded. The width persists, so when
          // the user later expands via the hamburger / header button, the pin
          // lands at exactly the width they dragged to.
          setExpandedWidth(w);
          return;
        }
        const nextMode = getSidebarModeForDrag(w);
        if (nextMode === 'hidden') {
          // drag-to-hide: keep the panel revealed so the user sees it float
          // (CSS will apply the floating inset style for this case).
          setSidebarMode('hidden');
          setRevealed(true);
        } else {
          setExpandedWidth(w);  // store clamps to [MIN, MAX]
        }
      }
      function onUp() {
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        document.body.classList.remove('is-resizing-sidebar');
        // If drag ended in hidden, decide whether the floating reveal sticks based
        // on where the cursor was released (within the floating panel + buffer).
        if (useLayoutStore.getState().sidebarMode === 'hidden') {
          // Read the latest expandedWidth so the hold region matches the panel
          // width the user just dragged to.
          const w = useLayoutStore.getState().expandedWidth;
          setRevealed(isSidebarRevealHoldPoint(lastClientX, w));
        }
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
  const setSidebarMode = useLayoutStore((s) => s.setSidebarMode);
  const projectsExpanded = useLayoutStore((s) => s.projectsExpanded);
  const sessionsExpanded = useLayoutStore((s) => s.sessionsExpanded);
  const toggleProjects = useLayoutStore((s) => s.toggleProjects);
  const toggleSessions = useLayoutStore((s) => s.toggleSessions);
  const [revealed, setRevealed] = useState(false);
  // Button-toggled hide hands off to the floating-reveal hold logic: the user's
  // cursor is still inside the sidebar at click time, so we stay revealed and let
  // the width-only hold check decide closure as the cursor leaves horizontally.
  // Drag-to-hide already keeps reveal=true via SidebarResizer.onMove; this aligns
  // the button path with that behavior.
  const collapseSidebar = useCallback(() => {
    setRevealed(true);
    setSidebarMode('hidden');
  }, [setSidebarMode]);
  const expandSidebar = useCallback(() => {
    setRevealed(false);
    setSidebarMode('expanded');
  }, [setSidebarMode]);
  // Hamburger is mobile-only (Outline pattern on desktop). On mobile this is a
  // drawer toggle — sidebar-mode is not involved. Desktop CSS hides the button.
  const onHamburgerClick = useCallback(() => {
    setRevealed((v) => !v);
  }, []);
  // Grace delay before closing on mouse-leave so brief excursions outside the
  // floating sidebar don't snap it shut. ~250ms feels intentional but not sticky.
  const closeTimerRef = useRef<number | null>(null);
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
  const scheduleClose = useCallback((point?: { clientX: number }) => {
    cancelClose();
    if (point) {
      const w = useLayoutStore.getState().expandedWidth;
      if (isSidebarRevealHoldPoint(point.clientX, w)) return;
    }
    closeTimerRef.current = window.setTimeout(() => setRevealed(false), 250);
  }, [cancelClose]);
  useEffect(() => cancelClose, [cancelClose]);

  useEffect(() => {
    if (sidebarMode !== 'hidden' || !revealed) return;

    function onPointerMove(ev: PointerEvent) {
      // Width-only: y is intentionally ignored. Read latest width every move
      // so dragging the panel wider during reveal also expands the hold area.
      const w = useLayoutStore.getState().expandedWidth;
      if (isSidebarRevealHoldPoint(ev.clientX, w)) {
        cancelClose();
      } else {
        scheduleClose();
      }
    }

    window.addEventListener('pointermove', onPointerMove);
    return () => window.removeEventListener('pointermove', onPointerMove);
  }, [cancelClose, revealed, scheduleClose, sidebarMode]);

  const projectsQuery = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  // URL에 projectSlug가 있을 때만 활성 프로젝트로 인정한다 — /projects 같은 곳에서는
  // sub-nav(Home/Files/.../Sessions)가 보이지 않아야 사용자 멘탈 모델과 일치.
  const activeProjectSlug = useActiveProjectSlug();
  const activeProject = (projectsQuery.data ?? []).find((p) => p.slug === activeProjectSlug);

  const sessionsQuery = useQuery({
    queryKey: ['sessions', activeProjectSlug],
    queryFn: () => listSessions(activeProjectSlug!),
    enabled: Boolean(activeProjectSlug),
  });

  const activeSessionId = useActiveSessionId();
  const activeRoute = useActiveRouteKey();

  // Close on navigation for mobile drawers, but keep desktop hidden/reveal open
  // so sidebar clicks don't immediately tuck it away.
  useEffect(() => {
    const isMobile = window.innerWidth < SIDEBAR_MOBILE_BREAKPOINT;
    if (isMobile || sidebarMode !== 'hidden') setRevealed(false);
  }, [activeProjectSlug, activeSessionId, activeRoute, sidebarMode]);

  const createSessionMutation = useMutation({
    mutationFn: (projectRef: string) => createSession(projectRef),
    onSuccess: async (session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', activeProjectSlug] });
      showToast('새 세션이 만들어졌습니다');
      if (activeProject) {
        navigate({
          to: '/projects/$projectSlug/sessions/$sessionPrefix',
          params: { projectSlug: activeProject.slug, sessionPrefix: shortSessionId(session.id) },
        });
      }
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(`세션 생성 실패: ${msg}`);
    },
  });

  const [pendingDelete, setPendingDelete] = useState<Session | null>(null);
  const [pendingDuplicate, setPendingDuplicate] = useState<Session | null>(null);
  const projectCreator = useNewProjectDialog();
  const deleteMutation = useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId),
    onSuccess: async (_, deletedId) => {
      if (activeProjectSlug) {
        await queryClient.invalidateQueries({ queryKey: ['sessions', activeProjectSlug] });
      }
      // deletedId is the full UUID; invalidate both full-UUID and prefix-based keys.
      await queryClient.invalidateQueries({ queryKey: ['session', deletedId] });
      await queryClient.invalidateQueries({ queryKey: ['session', shortSessionId(deletedId)] });
      // activeSessionId is now a 12-char prefix — compare against the prefix of the deleted id.
      if (activeSessionId === shortSessionId(deletedId) && activeProject) {
        navigate({ to: '/projects/$projectSlug', params: { projectSlug: activeProject.slug } });
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

  const duplicateMutation = useDuplicateSession(activeProjectSlug ?? '');

  function openProject(slug: string) {
    navigate({ to: '/projects/$projectSlug', params: { projectSlug: slug } });
  }

  function openSession(projectSlug: string, sessionPrefix: string) {
    navigate({ to: '/projects/$projectSlug/sessions/$sessionPrefix', params: { projectSlug, sessionPrefix } });
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
        onClick={onHamburgerClick}
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
          if (sidebarMode === 'hidden') scheduleClose({ clientX: e.clientX });
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
        <button
          type="button"
          className="cw-sidebar-collapse-btn"
          onClick={sidebarMode === 'hidden' ? expandSidebar : collapseSidebar}
          aria-label={sidebarMode === 'hidden' ? '사이드바 고정' : '사이드바 접기'}
          title={sidebarMode === 'hidden' ? '사이드바 고정' : '사이드바 접기'}
        >
          <Icon name={sidebarMode === 'hidden' ? 'chevron-right' : 'chevron-left'} size={16} />
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
              className={`cw-nav-row cw-project-nav-row ${item.slug === activeProjectSlug ? 'is-active' : ''}`}
              onClick={() => openProject(item.slug)}
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
              onClick={() => openProject(activeProject.slug)}
            >
              <IconPocket tone="home" icon="home" /> <span>Home</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'files' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectSlug/files', params: { projectSlug: activeProject.slug } })}
            >
              <IconPocket tone="files" icon="folder-open" /> <span>Files</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'skills' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectSlug/skills', params: { projectSlug: activeProject.slug } })}
            >
              <IconPocket tone="skills" icon="zap" /> <span>Skills</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'automation' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectSlug/automation', params: { projectSlug: activeProject.slug } })}
            >
              <IconPocket tone="schedule" icon="circle-play" /> <span>Automation</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'members' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectSlug/members', params: { projectSlug: activeProject.slug } })}
            >
              <IconPocket tone="members" icon="users" /> <span>Members</span>
            </button>
            <button
              className={`cw-nav-row ${activeRoute === 'settings' ? 'is-active' : ''}`}
              onClick={() => navigate({ to: '/projects/$projectSlug/settings', params: { projectSlug: activeProject.slug } })}
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
              onAdd={() => createSessionMutation.mutate(activeProject.slug)}
              addLabel={createSessionMutation.isPending ? '세션 생성 중…' : '새 Session'}
              addDisabled={createSessionMutation.isPending}
            />
            <div className="cw-sessions-list" data-expanded={sessionsExpanded ? 'true' : 'false'}>
              {(sessionsQuery.data ?? []).filter((s) => s.origin === 'user').map((session) => {
                const canDelete = canAdministerSession(session, activeProject, currentUser);
                return (
                  <div
                    key={session.id}
                    className={[
                      'cw-session-row',
                      shortSessionId(session.id) === activeSessionId ? 'is-active' : '',
                      session.unreadCount > 0 ? 'is-unread' : '',
                    ].filter(Boolean).join(' ')}
                    onClick={() => openSession(activeProject.slug, shortSessionId(session.id))}
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
                    <span className="cw-session-menu-wrap">
                      <SessionCardMenu
                        onDuplicate={() => setPendingDuplicate(session)}
                        duplicateDisabled={!session.lastMessageAt}
                        onDelete={canDelete ? () => setPendingDelete(session) : undefined}
                      />
                    </span>
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

      {pendingDuplicate && (
        <ConfirmDialog
          title="세션을 복제하시겠어요?"
          body={`"${pendingDuplicate.title}"의 메시지와 sandbox 상태를 새 세션으로 복제합니다. 세션이 사용 중이면 복제에 실패할 수 있습니다.`}
          confirmLabel="복제"
          pending={duplicateMutation.isPending}
          onConfirm={() => {
            duplicateMutation.mutate(pendingDuplicate.id, {
              onSuccess: (newSession) => {
                setPendingDuplicate(null);
                if (activeProject) {
                  openSession(activeProject.slug, shortSessionId(newSession.id));
                }
              },
              onError: () => setPendingDuplicate(null),
            });
          }}
          onClose={() => {
            if (!duplicateMutation.isPending) setPendingDuplicate(null);
          }}
        />
      )}

      {projectCreator.dialog}
      </aside>
    </>
  );
}

function useActiveProjectSlug(): string | null {
  return useParamFromMatches('projectSlug');
}

function useActiveSessionId(): string | null {
  return useParamFromMatches('sessionPrefix');
}

function useParamFromMatches(key: string): string | null {
  const state = useRouterState();
  for (const match of state.matches) {
    const params = match.params as Record<string, string | undefined>;
    if (params[key]) return params[key]!;
  }
  return null;
}

function useActiveRouteKey(): 'project' | 'files' | 'skills' | 'automation' | 'members' | 'settings' | 'session' | 'projects' {
  const state = useRouterState();
  const path = state.location.pathname;
  if (path.includes('/sessions/')) return 'session';
  if (path.endsWith('/files')) return 'files';
  if (path.endsWith('/skills')) return 'skills';
  if (/\/automation(\/|$)/.test(path)) return 'automation';
  if (path.endsWith('/members')) return 'members';
  if (path.endsWith('/settings')) return 'settings';
  if (path.match(/\/projects\/[^/]+$/)) return 'project';
  return 'projects';
}
