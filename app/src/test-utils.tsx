import type { ReactElement, ReactNode } from 'react';
import { MutationCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';

export function makeTestQueryClient(): QueryClient {
  return new QueryClient({
    // Prevent React Query's internal promise from surfacing as an unhandled
    // rejection (which vitest fails the test on) when a test exercises a
    // rejected mutation. The component's onError does the real handling; this
    // noop is a cache-level safety net.
    mutationCache: new MutationCache({ onError: () => {} }),
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

export function renderWithProviders(ui: ReactElement) {
  const client = makeTestQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  // Also return the client so tests can assert on the query cache (e.g. ['me']).
  return { client, ...render(ui, { wrapper }) };
}
