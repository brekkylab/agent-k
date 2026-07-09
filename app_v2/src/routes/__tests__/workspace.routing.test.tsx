import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { type ReactNode } from 'react';

// i18n mock: t(k) => k so assertions match i18n keys directly.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

// TanStack Router mock: createFileRoute is a passthrough; useParams returns sourceId='jira'.
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (opts: unknown) => opts,
  redirect: (args: unknown) => args,
  useNavigate: () => vi.fn(),
  useParams: () => ({ sourceId: 'jira' }),
}));

// Jira fixture entries (subset matching the real fixture).
const jiraEntries = [
  {
    id: 'jira-issue-DEV-201',
    sourceId: 'jira',
    title: '결제 모듈 성능 개선 (응답 시간 50% 단축)',
    subtitle: '[DEV-201] 진행 중',
    kind: 'item' as const,
    modifiedAt: '2026-07-02T11:00:00.000Z',
  },
  {
    id: 'jira-issue-DEV-198',
    sourceId: 'jira',
    title: '회원가입 이메일 인증 버그 수정',
    subtitle: '[DEV-198] 완료',
    kind: 'item' as const,
    modifiedAt: '2026-07-01T16:00:00.000Z',
  },
];

vi.mock('@/workspace/providers', () => ({
  getProvider: (id: string) => {
    if (id !== 'jira') return undefined;
    return {
      id: 'jira',
      nameKey: 'workspace.src.jira',
      category: 'docs',
      kind: 'items',
      connected: true,
      attachable: false,
      count: jiraEntries.length,
      list: () => Promise.resolve(jiraEntries),
      recent: () => Promise.resolve(jiraEntries),
      detail: vi.fn().mockResolvedValue({ entry: jiraEntries[0], externalUrl: '#' }),
    };
  },
}));

// Mock WorkspaceShell so useWorkspaceSelection returns a mock onSelect via
// React context, without needing the full shell (router, upload, etc.).
const mockOnSelect = vi.fn();
vi.mock('@/workspace/components/WorkspaceShell', () => ({
  useWorkspaceSelection: () => ({ onSelect: mockOnSelect }),
}));

import { SourcePage } from '@/routes/workspace.$sourceId';

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe('workspace.$sourceId routing (SourcePage)', () => {
  it('renders ItemListView content for jira (kind=items)', async () => {
    render(<SourcePage />, { wrapper });
    // Jira fixture first entry title must appear via ItemListView.
    await waitFor(() =>
      expect(screen.getByText('결제 모듈 성능 개선 (응답 시간 50% 단축)')).toBeTruthy()
    );
  });

  it('does NOT render unified search placeholder when showing jira source', async () => {
    render(<SourcePage />, { wrapper });
    await waitFor(() =>
      expect(screen.getByText('결제 모듈 성능 개선 (응답 시간 50% 단축)')).toBeTruthy()
    );
    // The unified search placeholder only appears in UnifiedList (workspace index), not here.
    expect(screen.queryByText('workspace.searchAll')).toBeNull();
  });
});
