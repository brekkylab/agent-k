import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';

// i18n mock — identity t so assertions target raw keys
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));
// webdav-backed API mock (FilePreviewModal and delete flow reach it)
vi.mock('@/api/workspace', () => ({
  listDirectory: vi.fn().mockResolvedValue([]),
  getFileBlob: vi.fn(),
  putFile: vi.fn(),
  deleteEntry: vi.fn().mockResolvedValue(undefined),
  createDirectory: vi.fn(),
  workspaceClient: vi.fn(),
}));
vi.mock('@/stores/workspace', () => ({
  getWorkspaceId: vi.fn(() => 'test-wid'),
  setWorkspaceId: vi.fn(),
}));
// Provider resolution mock — DetailPanel resolves the mount-aware provider via
// useProvider(); each test installs its own detail() behavior.
vi.mock('@/workspace/hooks/useProviders', () => ({
  useProvider: vi.fn(),
}));
// Lightbox mock — the real modal pulls in pdfjs (needs DOMMatrix, absent in
// jsdom) and its internals are covered elsewhere; here we only care that the
// panel can mount it.
vi.mock('@/components/FilePreviewModal', () => ({
  FilePreviewModal: ({ name }: { name: string }) => <div data-testid="lightbox">{name}</div>,
}));

import { deleteEntry } from '@/api/workspace';
import { useProvider } from '@/workspace/hooks/useProviders';
import { DetailPanel } from '../components/DetailPanel';
import type { SourceDetail, SourceEntry, SourceProvider } from '../types';

function wrapper(children: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

function installProvider(
  id: SourceProvider['id'],
  detail: SourceDetail,
  overrides: Partial<Pick<SourceProvider, 'category' | 'kind' | 'count'>> = {},
): SourceProvider {
  const provider: SourceProvider = {
    id,
    nameKey: `workspace.src.${id}`,
    category: overrides.category ?? 'files',
    kind: overrides.kind ?? 'files',
    connected: true,
    attachable: id === 'local',
    count: overrides.count ?? null,
    list: vi.fn().mockResolvedValue([]),
    recent: vi.fn().mockResolvedValue([]),
    detail: vi.fn().mockResolvedValue(detail),
  };
  (useProvider as ReturnType<typeof vi.fn>).mockImplementation((pid: string) =>
    pid === id ? provider : undefined,
  );
  return provider;
}

beforeEach(() => {
  vi.mocked(useProvider).mockReset();
  vi.mocked(deleteEntry).mockClear();
});

describe('DetailPanel', () => {
  it('renders meta from provider.detail', async () => {
    const entry: SourceEntry = {
      id: 'gdrive-file-report',
      sourceId: 'gdrive',
      title: '2분기 보고서.pdf',
      kind: 'file',
      size: 2_457_600,
      modifiedAt: '2026-07-01T00:00:00.000Z',
      path: '/reports/2분기 보고서.pdf',
    };
    installProvider('gdrive', { entry, externalUrl: '#' });

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByText('2분기 보고서.pdf')).toBeTruthy());
    // Source badge shows the provider name key (identity t)
    expect(screen.getByText('workspace.src.gdrive')).toBeTruthy();
    // Size appears in the meta line
    expect(screen.getByText(/2\.3 MB/)).toBeTruthy();
  });

  it('renders one bubble per bodyPreview line for threads, marking 나: as self', async () => {
    const entry: SourceEntry = {
      id: 'gmail-thread-1',
      sourceId: 'gmail',
      title: '[긴급] 2분기 검토',
      subtitle: '박대표 <ceo@example.com>',
      kind: 'thread',
      modifiedAt: '2026-07-02T08:30:00.000Z',
    };
    installProvider('gmail', {
      entry,
      bodyPreview: '박대표: 미팅 일정 공유 부탁드립니다.\n나: 목요일 오후 2시 가능합니다.',
      externalUrl: '#',
    });

    const { container } = render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByText('미팅 일정 공유 부탁드립니다.')).toBeTruthy());
    const bubbles = container.querySelectorAll('.cw-message-bubble');
    expect(bubbles.length).toBe(2);
    // The 나: line renders as a self message (right-aligned bubble)
    const selfMsg = screen.getByText('목요일 오후 2시 가능합니다.').closest('.cw-message');
    expect(selfMsg?.classList.contains('is-self')).toBe(true);
    // The other speaker's line is not self
    const otherMsg = screen.getByText('미팅 일정 공유 부탁드립니다.').closest('.cw-message');
    expect(otherMsg?.classList.contains('is-self')).toBe(false);
  });

  it('deletes a local file after ConfirmDialog confirm', async () => {
    const entry: SourceEntry = {
      id: '/test.pdf',
      sourceId: 'local',
      title: 'test.pdf',
      kind: 'file',
      size: 1024,
      modifiedAt: '2026-07-01T00:00:00.000Z',
      path: '/test.pdf',
    };
    installProvider('local', { entry });
    const onClose = vi.fn();

    render(wrapper(<DetailPanel entry={entry} onClose={onClose} />));

    await waitFor(() => expect(screen.getByText('workspace.detail.delete')).toBeTruthy());
    fireEvent.click(screen.getByText('workspace.detail.delete'));

    // ConfirmDialog appears with the shared delete confirm label
    const confirmBtn = await screen.findByText('delete.confirm');
    fireEvent.click(confirmBtn);

    await waitFor(() => expect(deleteEntry).toHaveBeenCalledWith('/test.pdf'));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('shows the mock attach notice when attach is clicked without onAttach', async () => {
    const entry: SourceEntry = {
      id: 'gdrive-file-report',
      sourceId: 'gdrive',
      title: 'Report.pdf',
      kind: 'file',
      size: 100,
      modifiedAt: '2026-07-01T00:00:00.000Z',
      path: '/Report.pdf',
    };
    installProvider('gdrive', { entry, externalUrl: '#' });

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByText('workspace.detail.openChat')).toBeTruthy());
    fireEvent.click(screen.getByText('workspace.detail.openChat'));
    expect(screen.getByText('workspace.detail.mockAttachToast')).toBeTruthy();
  });

  it('calls onAttach instead of showing the notice when provided', async () => {
    const entry: SourceEntry = {
      id: '/doc.md',
      sourceId: 'local',
      title: 'doc.md',
      kind: 'file',
      size: 42,
      modifiedAt: '2026-07-01T00:00:00.000Z',
      path: '/doc.md',
    };
    installProvider('local', { entry });
    const onAttach = vi.fn();

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} onAttach={onAttach} />));

    await waitFor(() => expect(screen.getByText('workspace.detail.openChat')).toBeTruthy());
    fireEvent.click(screen.getByText('workspace.detail.openChat'));
    expect(onAttach).toHaveBeenCalledWith(expect.objectContaining({ id: '/doc.md' }));
    expect(screen.queryByText('workspace.detail.mockAttachToast')).toBeNull();
  });

  it('shows 원본 열기 link for mock sources pointing at externalUrl', async () => {
    const entry: SourceEntry = {
      id: 'jira-1',
      sourceId: 'gdrive',
      title: 'Spec.docx',
      kind: 'file',
      modifiedAt: '2026-07-01T00:00:00.000Z',
      path: '/Spec.docx',
    };
    installProvider('gdrive', { entry, externalUrl: 'https://drive.example.com/spec' });

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    const link = await screen.findByText('workspace.detail.openOriginal');
    const anchor = link.closest('a');
    expect(anchor?.getAttribute('href')).toBe('https://drive.example.com/spec');
    expect(anchor?.getAttribute('target')).toBe('_blank');
  });

  it('renders Notion page bodyPreview as a document-like page body', async () => {
    const entry: SourceEntry = {
      id: 'notion-page-workspace-source-grounding',
      sourceId: 'notion',
      title: 'Workspace Source Grounding',
      subtitle: 'Product',
      kind: 'page',
      modifiedAt: '2026-07-06T11:45:00.000Z',
      parentId: 'notion-page-q3-product-strategy',
      emoji: '📚',
    };
    installProvider(
      'notion',
      {
        entry,
        bodyPreview:
          'Workspace Source Grounding은 Notion page tree를 유지합니다.\n\n## Principle\n- 답변은 source별 provenance chip을 가져야 합니다.\n- [x] Notion page tree는 source view 내부에서 펼친다',
        externalUrl: 'https://notion.example.com/workspace-source-grounding',
      },
      { category: 'docs', kind: 'pages', count: 1 },
    );

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByTestId('source-document-preview')).toBeTruthy());
    expect(screen.getByText('Principle')).toBeTruthy();
    expect(screen.getByText('답변은 source별 provenance chip을 가져야 합니다.')).toBeTruthy();
    expect(screen.getByText('Notion page tree는 source view 내부에서 펼친다')).toBeTruthy();
    const link = screen.getByText('workspace.detail.openOriginal').closest('a');
    expect(link?.getAttribute('href')).toBe('https://notion.example.com/workspace-source-grounding');
  });

  it('renders knowledge record status, confidence, and provenance', async () => {
    const entry: SourceEntry = {
      id: 'knowledge-decision-q3-mobile-performance',
      sourceId: 'knowledge',
      title: 'Q3 priority is mobile performance and reliability',
      subtitle: 'Decision · approved',
      kind: 'record',
      collection: 'Decisions',
      status: 'approved',
      confidence: 0.92,
      evidenceRefs: [
        {
          id: 'ev-test-notion',
          sourceId: 'notion',
          entryId: 'notion-page-q3-product-strategy',
          label: 'Notion / Q3 Product Strategy',
          excerpt: 'Q3 mobile performance priority.',
          usedFor: 'decision.summary',
        },
        {
          id: 'ev-test-slack',
          sourceId: 'slack',
          entryId: 'slack-thread-q3-planning',
          label: 'Slack #product',
          excerpt: '이번 분기 핵심은 성능 개선과 모바일 대응입니다.',
          usedFor: 'supporting evidence',
        },
      ],
      modifiedAt: '2026-07-03T12:20:00.000Z',
    };
    installProvider(
      'knowledge',
      {
        entry,
        bodyPreview:
          'Approved workspace decision for Q3 planning.\nMobile performance and reliability should be treated as the primary engineering priority.',
        externalUrl: '#',
      },
      { category: 'knowledge', kind: 'records', count: 1 },
    );

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByTestId('knowledge-detail-preview')).toBeTruthy());
    expect(screen.getByText('Decisions')).toBeTruthy();
    expect(screen.getByText('workspace.knowledge.status.approved')).toBeTruthy();
    expect(screen.getByText('workspace.knowledge.confidence: 92%')).toBeTruthy();
    expect(screen.getByText('Notion / Q3 Product Strategy')).toBeTruthy();
    expect(screen.getByText('Slack #product')).toBeTruthy();
    expect(screen.getByText('Q3 mobile performance priority.')).toBeTruthy();
  });

  it('shows knowledge records created from the selected source document', async () => {
    const entry: SourceEntry = {
      id: 'notion-page-q3-product-strategy',
      sourceId: 'notion',
      title: 'Q3 Product Strategy',
      subtitle: 'Product',
      kind: 'page',
      modifiedAt: '2026-07-05T10:00:00.000Z',
      parentId: 'notion-page-company-os',
    };
    installProvider(
      'notion',
      {
        entry,
        bodyPreview: 'Q3 product strategy page body.',
        externalUrl: '#',
      },
      { category: 'docs', kind: 'pages', count: 1 },
    );
    const onSelectEntry = vi.fn();

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} onSelectEntry={onSelectEntry} />));

    await waitFor(() => expect(screen.getByTestId('source-knowledge-links')).toBeTruthy());
    expect(screen.getByText('workspace.knowledge.fromThisSource')).toBeTruthy();
    fireEvent.click(screen.getByText('Q3 priority is mobile performance and reliability'));
    expect(onSelectEntry).toHaveBeenCalledWith(expect.objectContaining({
      id: 'knowledge-decision-q3-mobile-performance',
      sourceId: 'knowledge',
    }));
  });

  it('renders Confluence items as document-like bodies', async () => {
    const entry: SourceEntry = {
      id: 'confluence-page-api-spec',
      sourceId: 'confluence',
      title: 'REST API 명세서 v3.1',
      subtitle: 'DEV 스페이스',
      kind: 'item',
      modifiedAt: '2026-06-30T09:00:00.000Z',
    };
    installProvider(
      'confluence',
      {
        entry,
        bodyPreview:
          'REST API 명세서 v3.1입니다.\n\n## Changes\n- POST /workspaces/{id}/members 추가\n- [x] Cursor pagination 적용',
        externalUrl: 'https://confluence.example.com/api-spec',
      },
      { category: 'docs', kind: 'items', count: 1 },
    );

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByTestId('source-document-preview')).toBeTruthy());
    expect(screen.getByText('Changes')).toBeTruthy();
    expect(screen.getByText('POST /workspaces/{id}/members 추가')).toBeTruthy();
    expect(screen.getByText('Cursor pagination 적용')).toBeTruthy();
  });

  it('renders Jira items as document-like bodies', async () => {
    const entry: SourceEntry = {
      id: 'jira-issue-DEV-201',
      sourceId: 'jira',
      title: '결제 모듈 성능 개선 (응답 시간 50% 단축)',
      subtitle: '[DEV-201] 진행 중',
      kind: 'item',
      modifiedAt: '2026-07-02T11:00:00.000Z',
    };
    installProvider(
      'jira',
      {
        entry,
        bodyPreview:
          '결제 API 평균 응답 시간이 SLA를 초과합니다.\n\n## Scope\n- N+1 쿼리 패턴을 배치 조회로 교체합니다.\n- [ ] Batch loader 적용',
        externalUrl: 'https://jira.example.com/DEV-201',
      },
      { category: 'docs', kind: 'items', count: 1 },
    );

    render(wrapper(<DetailPanel entry={entry} onClose={vi.fn()} />));

    await waitFor(() => expect(screen.getByTestId('source-document-preview')).toBeTruthy());
    expect(screen.getByText('Scope')).toBeTruthy();
    expect(screen.getByText('N+1 쿼리 패턴을 배치 조회로 교체합니다.')).toBeTruthy();
    expect(screen.getByText('Batch loader 적용')).toBeTruthy();
  });
});
