import { StrictMode, Suspense } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';
import './i18n';
import { routeTree } from './routeTree.gen';
import { ApiError, setUnauthorizedHandler } from '@/api/client';
import { forceLogout, setLogoutRouter } from '@/lib/forceLogout';
import './styles/globals.css';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (failureCount, error) =>
        !(error instanceof ApiError && error.status === 401) && failureCount < 1,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
});

const router = createRouter({
  routeTree,
  defaultPreload: 'intent',
  context: { queryClient },
});

// Inject router into forceLogout so it uses client-side navigation instead of
// window.location.href — avoiding a full-page reload that would race with
// TanStack Router's own redirect and consume the sessionStorage banner items.
setLogoutRouter((to) => {
  router.navigate({ to } as Parameters<typeof router.navigate>[0])
    .catch(() => { window.location.href = to; });
});

setUnauthorizedHandler((reason) => {
  forceLogout({ reason, redirectTo: window.location.pathname + window.location.search });
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
  // Transient navigation payloads handed off between routes via router state.
  // initialMessage: the first utterance created by the home composer, received and auto-sent by the session page.
  // focusComposer: sidebar '+' requests composer mode + input focus when entering home.
  interface HistoryState {
    initialMessage?: string;
    focusComposer?: boolean;
    // Agent selected in the home composer, carried to the session header chip.
    initialAgentId?: import('@/domain/agentSurfaces').AgentId;
    // Shared-file paths (scope-relative) dragged from Files onto a session row,
    // attached to the next message once the target session resolves.
    attachShared?: string[];
    // Global paths for files attached on the home composer — uploaded files
    // (inputs/) or shared files picked from the picker — carried to and attached
    // to the auto-sent first message of the newly created session.
    initialAttachments?: string[];
  }
}

const root = document.getElementById('root');
if (!root) throw new Error('#root not found');

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <Suspense fallback={null}>
        <RouterProvider router={router} />
      </Suspense>
    </QueryClientProvider>
  </StrictMode>,
);
