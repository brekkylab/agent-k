import { useEffect } from 'react';
import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, getToken } from '@/api/client';
import { getMe } from '@/api/auth';
import { useAuthStore } from '@/stores/auth';
import { useLayoutStore } from '@/stores/layout';
import { Sidebar } from '@/components/layout/Sidebar';
import { appWs } from '@/api/ws';
import type { Session } from '@/domain/types';

export const Route = createFileRoute('/_app')({
  beforeLoad: async ({ context }) => {
    if (!getToken()) throw redirect({ to: '/login' });

    // 캐시 데이터가 있으면 블로킹 없이 통과 — AppShell의 useQuery가 백그라운드에서 갱신
    const cached = context.queryClient.getQueryData(['me']);
    if (cached) return;

    // 첫 로드(콜드 캐시)에서만 서버 확인 — 만료된 토큰을 초기 진입 시점에 잡음
    try {
      await context.queryClient.fetchQuery({ queryKey: ['me'], queryFn: getMe, staleTime: 5 * 60_000 });
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        throw redirect({ to: '/login' });
      }
      // 네트워크 오류, 5xx: 통과 — 페이지가 자체 에러 상태 처리
    }
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
