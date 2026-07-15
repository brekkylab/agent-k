/**
 * Integration test: workspace router Outlet path.
 *
 * Verifies that navigating to /workspace/$sourceId renders the child route
 * (SourcePage) INSIDE the workspace layout's real <Outlet/>. This test would
 * fail if WorkspaceShell lost its <Outlet/>, because the jira entry title would
 * never appear in the DOM.
 *
 * Approach: hand-made mini routeTree (root -> workspace layout -> $sourceId)
 * so we avoid pulling in the full routeTree.gen.ts (which drags in bootstrap /
 * AppShell / API calls). The real WorkspaceShell and SourcePage components are
 * used, keeping the Outlet path authentic.
 */

import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  RouterProvider,
  createRouter,
  createRootRoute,
  createRoute,
  createMemoryHistory,
  Outlet,
} from '@tanstack/react-router';

// ---------------------------------------------------------------------------
// i18n mock: t(key) => key so assertions can use i18n keys directly.
// ---------------------------------------------------------------------------
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

// ---------------------------------------------------------------------------
// Workspace providers mock: jira returns one fixture entry.
// ---------------------------------------------------------------------------
const MY_JIRA_TICKET = 'My Jira Ticket';

vi.mock('@/workspace/providers', () => ({
  PROVIDERS: [
    {
      id: 'jira',
      nameKey: 'workspace.src.jira',
      category: 'docs',
      kind: 'items',
      connected: true,
      attachable: false,
      count: 1,
      list: () =>
        Promise.resolve([
          {
            id: 'jira-issue-TEST-1',
            sourceId: 'jira',
            title: MY_JIRA_TICKET,
            subtitle: '[TEST-1] Done',
            kind: 'item' as const,
            modifiedAt: '2026-07-03T10:00:00.000Z',
          },
        ]),
      recent: () => Promise.resolve([]),
      detail: vi.fn().mockResolvedValue({ entry: null, externalUrl: '#' }),
    },
  ],
  getProviderMeta: (id: string) => (id === 'jira' ? { id: 'jira', kind: 'items' } : undefined),
  recentAcross: () => Promise.resolve([]),
}));

// Mount-aware resolution mock: SourcePage resolves the provider via useProvider.
vi.mock('@/workspace/hooks/useProviders', () => {
  const jira = {
    id: 'jira',
    nameKey: 'workspace.src.jira',
    category: 'docs',
    kind: 'items' as const,
    connected: true,
    attachable: false,
    count: 1,
    list: () =>
      Promise.resolve([
        {
          id: 'jira-issue-TEST-1',
          sourceId: 'jira',
          title: MY_JIRA_TICKET,
          subtitle: '[TEST-1] Done',
          kind: 'item' as const,
          modifiedAt: '2026-07-03T10:00:00.000Z',
        },
      ]),
    recent: () => Promise.resolve([]),
    detail: vi.fn().mockResolvedValue({ entry: null, externalUrl: '#' }),
  };
  return {
    useProvider: (id: string) => (id === 'jira' ? jira : undefined),
    useProviders: () => [jira],
    useMounts: () => ({ data: [] }),
  };
});

// ---------------------------------------------------------------------------
// Mock WorkspaceShell's heavy internal dependencies so we can use the REAL
// WorkspaceShell (which contains <Outlet/>) without needing a backend server.
//   - SourceRail: renders <Link> elements; not under test here.
//   - DetailPanel: selection side-panel; not under test here.
//   - Icon: SVG component; not needed in this test.
//   - putFile / setPendingAttachment: upload / attachment side-effects.
// ---------------------------------------------------------------------------
vi.mock('@/workspace/components/SourceRail', () => ({
  SourceRail: () => <div data-testid="source-rail" />,
}));

vi.mock('@/workspace/components/DetailPanel', () => ({
  DetailPanel: () => <div data-testid="detail-panel" />,
}));

vi.mock('@/components/Icon', () => ({
  Icon: () => null,
}));

vi.mock('@/api/workspace', () => ({
  putFile: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@/stores/pendingAttachment', () => ({
  setPendingAttachment: vi.fn(),
}));

// ---------------------------------------------------------------------------
// Import real WorkspaceShell and SourcePage AFTER mocks are registered.
// ---------------------------------------------------------------------------
import { WorkspaceShell } from '@/workspace/components/WorkspaceShell';
import { SourcePage } from '@/routes/workspace.$sourceId';

// ---------------------------------------------------------------------------
// Build a minimal routeTree that mirrors the real layout hierarchy:
//   root  ->  /workspace (layout: WorkspaceShell with <Outlet/>)
//         ->  /workspace/$sourceId (SourcePage)
//
// Using createRootRoute / createRoute rather than createFileRoute avoids the
// TanStack Router vite plugin's file-system coupling in test environments.
// ---------------------------------------------------------------------------
function buildTestRouter() {
  const rootRoute = createRootRoute({
    component: () => <Outlet />,
  });

  const workspaceLayoutRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/workspace',
    // Use the real WorkspaceShell — this is the component that owns <Outlet/>.
    // If WorkspaceShell ever removes its <Outlet/>, the child will not render
    // and the assertion below will timeout/fail.
    component: WorkspaceShell,
  });

  const workspaceSourceRoute = createRoute({
    getParentRoute: () => workspaceLayoutRoute,
    path: '/$sourceId',
    component: SourcePage,
  });

  const routeTree = rootRoute.addChildren([
    workspaceLayoutRoute.addChildren([workspaceSourceRoute]),
  ]);

  const history = createMemoryHistory({ initialEntries: ['/workspace/jira'] });

  return createRouter({ routeTree, history });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
describe('workspace router Outlet integration', () => {
  it('renders SourcePage child inside WorkspaceShell Outlet for /workspace/jira', async () => {
    const router = buildTestRouter();
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });

    render(
      <QueryClientProvider client={qc}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    );

    // The jira fixture entry title must appear, proving SourcePage was mounted
    // inside WorkspaceShell's <Outlet/>. If the Outlet were missing, the child
    // route component would never render and this assertion would fail.
    await waitFor(() =>
      expect(screen.getByText(MY_JIRA_TICKET)).toBeTruthy(),
    );
  });

  it('does NOT show the unified search placeholder when a specific source is active', async () => {
    const router = buildTestRouter();
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });

    render(
      <QueryClientProvider client={qc}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    );

    // Wait for the jira content to appear.
    await waitFor(() =>
      expect(screen.getByText(MY_JIRA_TICKET)).toBeTruthy(),
    );

    // The unified search placeholder ('workspace.searchAll') only appears in
    // UnifiedList (the /workspace index page), not in a source-specific view.
    expect(screen.queryByText('workspace.searchAll')).toBeNull();
  });
});
