import { useEffect } from 'react';
import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, getToken } from '@/api/client';
import { getMe } from '@/api/auth';
import { loadNs } from '@/i18n/loader';
import { useAuthStore } from '@/stores/auth';
import { useLayoutStore } from '@/stores/layout';
import { Sidebar } from '@/components/layout/Sidebar';
import { appWs } from '@/api/ws';
import type { Session } from '@/domain/types';

export const Route = createFileRoute('/_app')({
  beforeLoad: async ({ context }) => {
    if (!getToken()) throw redirect({ to: '/login' });

    // Warm cache: pass through without blocking — AppShell's useQuery handles background revalidation.
    const cached = context.queryClient.getQueryData(['me']);
    if (cached) return;

    // Cold cache: verify the token with the server on first load.
    try {
      await context.queryClient.fetchQuery({ queryKey: ['me'], queryFn: getMe, staleTime: 5 * 60_000 });
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        throw redirect({ to: '/login' });
      }
      // Network error or 5xx: fall through and let the page handle it.
    }
  },
  // Sidebar mounts SessionCardMenu (`session`) and triggers NewProjectDialog
  // (`dialogs`) in addition to its own `common`/`project` ns. These must all
  // be guaranteed at the shell level — any route that doesn't independently
  // load them would otherwise unmount the entire shell when the user opens
  // a sidebar menu, re-introducing the blank-flash.
  loader: () => loadNs('common', 'project', 'session', 'dialogs'),
  component: AppShell,
});

function AppShell() {
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);
  const sidebarMode = useLayoutStore((s) => s.sidebarMode);
  const expandedWidth = useLayoutStore((s) => s.expandedWidth);
  const sidebarWidth = sidebarMode === 'expanded' ? expandedWidth : 0;
  const queryClient = useQueryClient();
  const me = useQuery({
    queryKey: ['me'],
    queryFn: getMe,
    staleTime: 5 * 60_000,
  });

  useEffect(() => {
    if (me.data) setCurrentUser(me.data);
  }, [me.data, setCurrentUser]);

  useEffect(() => {
    const token = getToken();
    if (!token) return;
    appWs.connect(token);
    const unsub = appWs.subscribe((event) => {
      if (event.type === 'session_title_updated') {
        queryClient.setQueryData<Session | undefined>(
          ['session', event.session_id],
          (old) => (old ? { ...old, title: event.title } : old),
        );
        void queryClient.invalidateQueries({ queryKey: ['sessions', event.project_id] });
      }
    });
    return () => {
      unsub();
      appWs.disconnect();
    };
  }, [queryClient]);

  return (
    <div
      className="cw-app-shell"
      data-sidebar-mode={sidebarMode}
      style={{
        // grid column width — collapses to 0 in hidden mode so main goes full bleed.
        '--cw-sidebar-w': `${sidebarWidth}px`,
        // sidebar's own width — always tracks expandedWidth so the floating reveal
        // matches whatever width the user has set. Option B: drag in hidden adjusts
        // this too, the panel just doesn't auto-pin back to expanded.
        '--cw-sidebar-floating-w': `${expandedWidth}px`,
      } as React.CSSProperties}
    >
      <Sidebar />
      <main className="cw-main-shell cw-scroll-quiet">
        <Outlet />
      </main>
    </div>
  );
}
