import type { ReactElement, ReactNode } from 'react';
import { MutationCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';

export function makeTestQueryClient(): QueryClient {
  return new QueryClient({
    // 거부된 mutation을 테스트에서 다룰 때 React Query 내부 프로미스가
    // unhandled rejection으로 새어 vitest가 테스트를 실패시키는 것을 막는다.
    // 컴포넌트의 onError가 실제 처리를 담당하고, 이 noop은 cache 레벨 안전망이다.
    mutationCache: new MutationCache({ onError: () => {} }),
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

export function renderWithProviders(ui: ReactElement) {
  const client = makeTestQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  // client를 함께 반환해 테스트에서 ['me'] 등 쿼리 캐시를 단언할 수 있게 한다.
  return { client, ...render(ui, { wrapper }) };
}
