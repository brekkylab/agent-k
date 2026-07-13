// App shell — fixed left sidebar + scrollable main content. Reuses the
// design system's shell/Sidebar class names directly (see
// cowork-design-system.css) so its CSS applies without duplication. app_v2's
// sidebar is single-user: brand at top, a prominent "New Chat" primary
// action, a Workspace nav link (with a subtle divider below it), a Recents
// session list, and the LanguageToggle in the footer.

import { useState, useRef, type ReactNode } from 'react';
import { Link, useNavigate, useParams, useRouterState } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listSessions, updateSessionTitle, deleteSession, SESSION_TITLE_MAX_LEN } from '@/api/sessions';
import type { SessionResponse } from '@/api/types';
import { LanguageToggle } from '@/components/LanguageToggle';
import { SessionTitle } from '@/components/SessionTitle';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Icon } from '@/components/Icon';

/** One Recents row: opens the session on click, with a hover ⋯ menu that
 *  swaps the title into an inline rename input (same save/cancel rules as the
 *  chat header). */
function SessionRow({
  session,
  isActive,
  onOpen,
}: {
  session: SessionResponse;
  isActive: boolean;
  onOpen: () => void;
}) {
  const { t } = useTranslation('common');
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const cancelEditRef = useRef(false);
  const [deleting, setDeleting] = useState(false);
  const [deletePending, setDeletePending] = useState(false);

  function patchTitle(next: string | null) {
    qc.setQueryData<SessionResponse[]>(['sessions'], (list) =>
      list?.map((s) => (s.id === session.id ? { ...s, title: next } : s)),
    );
  }

  function startEdit() {
    setMenuOpen(false);
    cancelEditRef.current = false;
    setDraft(session.title ?? '');
    setEditing(true);
  }

  async function commitEdit() {
    setEditing(false);
    if (cancelEditRef.current) {
      cancelEditRef.current = false;
      return;
    }
    const next = draft.trim();
    if (next === '' || next === session.title) return;
    const prev = session.title;
    patchTitle(next); // optimistic; the server echoes a title event too
    try {
      await updateSessionTitle(session.id, next);
    } catch {
      patchTitle(prev); // revert on failure
    }
  }

  function openDelete() {
    setMenuOpen(false);
    setDeleting(true);
  }

  async function confirmDelete() {
    setDeletePending(true);
    try {
      await deleteSession(session.id);
      // Drop it from the cached list immediately, then reconcile from the server.
      qc.setQueryData<SessionResponse[]>(['sessions'], (list) =>
        list?.filter((s) => s.id !== session.id),
      );
      void qc.invalidateQueries({ queryKey: ['sessions'] });
      setDeleting(false);
      setDeletePending(false);
      // If the open session was the one deleted, leave its (now-404) route.
      if (isActive) void navigate({ to: '/' });
    } catch {
      setDeletePending(false); // keep the dialog open so the user can retry
    }
  }

  if (editing) {
    return (
      <div className={`cw-session-row${isActive ? ' is-active' : ''}`}>
        <span className="cw-pocket"><Icon name="message-square" size={12} /></span>
        <input
          className="cw-session-rename-input"
          value={draft}
          autoFocus
          maxLength={SESSION_TITLE_MAX_LEN}
          aria-label={t('nav.rename', 'Rename session')}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={(e) => e.currentTarget.select()}
          onClick={(e) => e.stopPropagation()}
          onBlur={() => void commitEdit()}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              e.currentTarget.blur(); // routes through onBlur → commit
            } else if (e.key === 'Escape') {
              e.preventDefault();
              cancelEditRef.current = true;
              e.currentTarget.blur();
            }
          }}
        />
        {draft.length >= SESSION_TITLE_MAX_LEN - 15 && (
          <span className="cw-title-count">
            {draft.length}/{SESSION_TITLE_MAX_LEN}
          </span>
        )}
      </div>
    );
  }

  return (
    <>
      <div
        className={`cw-session-row${isActive ? ' is-active' : ''}`}
        role="button"
        tabIndex={0}
        onClick={onOpen}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onOpen();
          }
        }}
        title={session.title ?? undefined}
      >
        <span className="cw-pocket"><Icon name="message-square" size={12} /></span>
        <SessionTitle
          title={session.title}
          createdAt={session.created_at}
          className="cw-session-title"
          skeletonClassName="cw-title-skeleton--row"
          fallback={session.id.slice(0, 8)}
        />
        <span className="cw-session-menu-wrap">
          <button
            type="button"
            className="cw-session-menu-btn"
            aria-label={t('nav.more', 'More')}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={(e) => {
              e.stopPropagation();
              setMenuOpen((v) => !v);
            }}
          >
            <Icon name="more" size={14} />
          </button>
          {menuOpen && (
            <>
              {/* Full-viewport click-away catcher; closes the menu without a
                  document listener and swallows the click so it doesn't open the
                  session. */}
              <div
                className="cw-session-menu-overlay"
                onClick={(e) => {
                  e.stopPropagation();
                  setMenuOpen(false);
                }}
              />
              <div className="cw-session-menu" role="menu">
                {/* Rename only after the title lands (not during generation). */}
                {session.title != null && (
                  <button
                    type="button"
                    className="cw-session-menu-item"
                    role="menuitem"
                    onClick={(e) => {
                      e.stopPropagation();
                      startEdit();
                    }}
                  >
                    <Icon name="writing" size={13} />
                    <span>{t('nav.rename', 'Rename')}</span>
                  </button>
                )}
                <button
                  type="button"
                  className="cw-session-menu-item cw-session-menu-item--danger"
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation();
                    openDelete();
                  }}
                >
                  <Icon name="trash" size={13} />
                  <span>{t('nav.delete', 'Delete')}</span>
                </button>
              </div>
            </>
          )}
        </span>
      </div>
      {deleting && (
        <ConfirmDialog
          title={t('session_delete.title', 'Delete session')}
          body={t(
            'session_delete.body',
            'This session and its messages will be permanently deleted. This cannot be undone.',
          )}
          confirmLabel={t('session_delete.confirm', 'Delete')}
          destructive
          pending={deletePending}
          onConfirm={() => void confirmDelete()}
          onClose={() => setDeleting(false)}
        />
      )}
    </>
  );
}

function SessionNavList() {
  const { t } = useTranslation('common');
  const navigate = useNavigate();
  const activeSessionId = useParams({ strict: false }).sessionId as string | undefined;
  const { data: sessions = [] } = useQuery({
    queryKey: ['sessions'],
    queryFn: listSessions,
    staleTime: 30_000,
  });

  if (sessions.length === 0) {
    return <p className="cw-recents-empty">{t('nav.recents_empty', 'No recent chats yet')}</p>;
  }

  return (
    <div className="cw-sessions-list">
      {sessions.map((s) => (
        <SessionRow
          key={s.id}
          session={s}
          isActive={s.id === activeSessionId}
          onOpen={() => navigate({ to: '/sessions/$sessionId', params: { sessionId: s.id } })}
        />
      ))}
    </div>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const { t } = useTranslation('common');
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const onSessions = pathname === '/' || pathname.startsWith('/sessions');
  // Exact-prefix match: '/workspace-b' etc. are sibling candidate tabs, not sub-routes.
  const onWorkspace = pathname === '/workspace' || pathname.startsWith('/workspace/');
  const onWorkspaceB = pathname.startsWith('/workspace-b');
  const onWorkspaceC = pathname.startsWith('/workspace-c');
  const onWorkspaceD = pathname.startsWith('/workspace-d');
  const onWorkspaceE = pathname.startsWith('/workspace-e');
  const onWorkspaceF = pathname.startsWith('/workspace-f');

  return (
    <div className="cw-app-shell">
      <aside className="cw-sidebar-app">
        <div className="cw-sidebar-header">
          <Link to="/" className="cw-brand-lockup" aria-label="Cowork">
            <span className="cw-brand-mark" aria-hidden="true">C</span>
            <strong>Cowork</strong>
          </Link>
        </div>

        <div className="cw-sidebar-scroll">
          <button
            type="button"
            className={`cw-new-chat${onSessions ? ' is-active' : ''}`}
            onClick={() => navigate({ to: '/' })}
          >
            <span className="cw-pocket"><Icon name="plus" size={14} /></span>
            <span>{t('nav.new_chat', 'New Chat')}</span>
          </button>

          <Link
            to="/workspace"
            className={`cw-nav-row${onWorkspace ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="folder-open" size={13} /></span>
            <span>{t('nav.workspace', 'Workspace')}</span>
          </Link>

          {/* Workspace design candidates — mockup tabs for comparing workspace directions. */}
          <Link
            to="/workspace-b"
            className={`cw-nav-row${onWorkspaceB ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="file-text" size={13} /></span>
            <span>WS · NotebookLM형</span>
          </Link>

          <Link
            to="/workspace-c"
            className={`cw-nav-row${onWorkspaceC ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="image" size={13} /></span>
            <span>WS · 가꾸기 캔버스</span>
          </Link>

          <Link
            to="/workspace-d"
            className={`cw-nav-row${onWorkspaceD ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="sheet" size={13} /></span>
            <span>WS · 경작형</span>
          </Link>

          <Link
            to="/workspace-e"
            className={`cw-nav-row${onWorkspaceE ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="grid" size={13} /></span>
            <span>WS · Astryx</span>
          </Link>

          <Link
            to="/workspace-f"
            className={`cw-nav-row${onWorkspaceF ? ' is-active' : ''}`}
          >
            <span className="cw-pocket"><Icon name="shield" size={13} /></span>
            <span>WS · Glean형</span>
          </Link>

          {/* Subtle divider between Workspace and Recents */}
          <hr className="cw-sidebar-divider" />

          <div className="cw-section-header">
            <span className="cw-section-toggle" style={{ cursor: 'default' }}>
              <span>{t('nav.recents', 'Recents')}</span>
            </span>
          </div>

          <SessionNavList />
        </div>

        <div className="cw-sidebar-footer">
          <span className="cw-sidebar-footer-label">{t('language.label', 'Language')}</span>
          <LanguageToggle />
        </div>
      </aside>

      <main className="cw-main-shell">{children}</main>
    </div>
  );
}
