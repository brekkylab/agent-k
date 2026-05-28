import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';
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
setLogoutRouter((to) => { void router.navigate({ to } as Parameters<typeof router.navigate>[0]); });

setUnauthorizedHandler((reason) => {
  forceLogout({ reason, redirectTo: window.location.pathname + window.location.search });
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

const root = document.getElementById('root');
if (!root) throw new Error('#root not found');

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
