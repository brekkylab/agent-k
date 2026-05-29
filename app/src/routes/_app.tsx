import { useEffect } from 'react';
import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getToken } from '@/api/client';
import { getMe } from '@/api/auth';
import { useAuthStore } from '@/stores/auth';
import { useLayoutStore } from '@/stores/layout';
import { Sidebar } from '@/components/layout/Sidebar';
import { appWs } from '@/api/ws';
import type { Project, Session } from '@/domain/types';

export const Route = createFileRoute('/_app')({
  beforeLoad: () => {
    if (!getToken()) throw redirect({ to: '/login' });
  },
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
        // Session lists are keyed by project slug; event carries only the UUID.
        // Resolve via the projects cache; no-op if it isn't loaded yet.
        const projects = queryClient.getQueryData<Project[]>(['projects']) ?? [];
        const slug = projects.find((p) => p.id === event.project_id)?.slug;
        if (slug) {
          void queryClient.invalidateQueries({ queryKey: ['sessions', slug] });
        }
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
