import { beforeEach, describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { type ReactNode } from 'react';

// i18n mock: t(k) => k so assertions match i18n keys directly.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

let mockSourceId = 'jira';

// TanStack Router mock: createFileRoute is a passthrough; useParams returns the active source id.
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (opts: unknown) => opts,
  redirect: (args: unknown) => args,
  useNavigate: () => vi.fn(),
  useParams: () => ({ sourceId: mockSourceId }),
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

const notionEntries = [
  {
    id: 'notion-page-company-os',
    sourceId: 'notion',
    title: 'Company OS',
    subtitle: 'Workspace',
    kind: 'page' as const,
    modifiedAt: '2026-07-03T12:30:00.000Z',
    parentId: null,
    emoji: '🏢',
  },
];

const knowledgeEntries = [
  {
    id: 'knowledge-decision-q3',
    sourceId: 'knowledge',
    title: 'Q3 priority is mobile performance and reliability',
    subtitle: 'Decision · approved',
    kind: 'record' as const,
    collection: 'Decisions',
    status: 'approved' as const,
    confidence: 0.92,
    evidenceRefs: [
      {
        id: 'ev-test-route',
        sourceId: 'notion',
        entryId: 'notion-page-q3-product-strategy',
        label: 'Notion / Q3 Product Strategy',
        excerpt: 'Q3 product priority',
        usedFor: 'decision.summary',
      },
    ],
    modifiedAt: '2026-07-03T12:20:00.000Z',
  },
];

// providers.ts is imported by SourcePage's loader guard (getProviderMeta); the
// component itself resolves the provider via the mount-aware useProvider hook.
vi.mock('@/workspace/providers', () => ({
  getProviderMeta: (id: string) => ({ id, kind: 'items' }),
}));

vi.mock('@/workspace/hooks/useProviders', () => ({
  useProviders: () => [],
  useMounts: () => ({ data: [] }),
  useProvider: (id: string) => {
    if (id === 'knowledge') {
      return {
        id: 'knowledge',
        nameKey: 'workspace.src.knowledge',
        category: 'knowledge',
        kind: 'records',
        connected: true,
        attachable: false,
        count: knowledgeEntries.length,
        list: () => Promise.resolve(knowledgeEntries),
        recent: () => Promise.resolve(knowledgeEntries),
        detail: vi.fn().mockResolvedValue({
          entry: knowledgeEntries[0],
          bodyPreview: 'Approved workspace decision',
          externalUrl: '#',
        }),
      };
    }
    if (id === 'notion') {
      return {
        id: 'notion',
        nameKey: 'workspace.src.notion',
        category: 'docs',
        kind: 'pages',
        connected: true,
        attachable: false,
        count: notionEntries.length,
        list: () => Promise.resolve(notionEntries),
        recent: () => Promise.resolve(notionEntries),
        detail: vi.fn().mockResolvedValue({
          entry: notionEntries[0],
          bodyPreview: 'Company OS page preview',
          externalUrl: '#',
        }),
      };
    }
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
  beforeEach(() => {
    mockSourceId = 'jira';
    mockOnSelect.mockClear();
  });

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

  it('renders NotionPageView content for notion (kind=pages)', async () => {
    mockSourceId = 'notion';

    render(<SourcePage />, { wrapper });

    await waitFor(() => expect(screen.getByText('Company OS')).toBeTruthy());
    expect(screen.getByTestId('notion-page-tree')).toBeTruthy();
  });

  it('renders KnowledgeRecordView content for knowledge (kind=records)', async () => {
    mockSourceId = 'knowledge';

    render(<SourcePage />, { wrapper });

    await waitFor(() => expect(screen.getByTestId('knowledge-record-view')).toBeTruthy());
    expect(screen.getByText('Q3 priority is mobile performance and reliability')).toBeTruthy();
  });
});
