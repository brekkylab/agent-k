// App shell — fixed left sidebar + scrollable main content, ported from the
// original Cowork app's shell/Sidebar visual language (class names reused so the
// design-system CSS applies). app_v2's sidebar is single-user: brand at top, a
// prominent "New Chat" primary action, a Workspace nav link (with a subtle
// divider below it), a Recents session list, and the LanguageToggle in the footer.

import type { ReactNode } from 'react';
import { Link, useNavigate, useParams, useRouterState } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listSessions } from '@/api/sessions';
import { LanguageToggle } from '@/components/LanguageToggle';
import { Icon } from '@/components/Icon';

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
      {sessions.map((s) => {
        const isActive = s.id === activeSessionId;
        const title = s.title ?? s.id.slice(0, 8);
        return (
          <div
            key={s.id}
            className={`cw-session-row${isActive ? ' is-active' : ''}`}
            role="button"
            tabIndex={0}
            onClick={() => navigate({ to: '/sessions/$sessionId', params: { sessionId: s.id } })}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                void navigate({ to: '/sessions/$sessionId', params: { sessionId: s.id } });
              }
            }}
            title={title}
          >
            <span className="cw-pocket"><Icon name="message-square" size={12} /></span>
            <span className="cw-session-title">{title}</span>
          </div>
        );
      })}
    </div>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const { t } = useTranslation('common');
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const onSessions = pathname === '/' || pathname.startsWith('/sessions');
  const onWorkspace = pathname.startsWith('/workspace');

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
