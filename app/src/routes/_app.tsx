import { useEffect } from 'react';
import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getToken } from '@/api/client';
import { getMe } from '@/api/auth';
import { useAuthStore } from '@/stores/auth';
import { useLayoutStore } from '@/stores/layout';
import { Sidebar } from '@/components/layout/Sidebar';
import { appWs } from '@/api/ws';
import type { Session } from '@/domain/types';

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
        // Update both full-UUID key (legacy) and 12-char prefix key (current nav)
        const prefix12 = event.session_id.slice(0, 12);
        const updater = (old: Session | undefined) => (old ? { ...old, title: event.title } : old);
        queryClient.setQueryData<Session | undefined>(['session', event.session_id], updater);
        queryClient.setQueryData<Session | undefined>(['session', prefix12], updater);
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
      style={{ '--cw-sidebar-w': `${sidebarWidth}px` } as React.CSSProperties}
    >
      <Sidebar />
      <main className="cw-main-shell cw-scroll-quiet">
        <Outlet />
      </main>
    </div>
  );
}
