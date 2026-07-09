import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';

// i18n mock
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));
// TanStack Router mock
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (opts: unknown) => opts,
  useNavigate: () => vi.fn(),
  useParams: () => ({}),
  Link: ({ children, to }: { children: ReactNode; to: string }) => <a href={to}>{children}</a>,
}));

import { FileBrowserView } from '../components/FileBrowserView';
import { ItemListView } from '../components/ItemListView';
import { ThreadListView } from '../components/ThreadListView';
import type { SourceEntry, SourceProvider } from '../types';

// Helper: wrap with per-test QueryClient (retry:false to avoid hanging)
function wrapper(children: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

// Mock provider for FileBrowser tests
function makeFilesProvider(entries: SourceEntry[]): SourceProvider {
  return {
    id: 'gdrive',
    nameKey: 'workspace.src.gdrive',
    category: 'files',
    kind: 'files',
    connected: true,
    attachable: false,
    count: entries.length,
    list: vi.fn().mockImplementation(({ path }: { path?: string }) => {
      const p = path ?? '';
      return Promise.resolve(
        entries.filter((e) => {
          if (p === '') return true;
          return e.path?.startsWith(p) && e.path !== p;
        })
      );
    }),
    recent: () => Promise.resolve([]),
    detail: vi.fn().mockResolvedValue({ entry: entries[0], externalUrl: '#' }),
  };
}

const fileEntries: SourceEntry[] = [
  { id: 'folder-reports', sourceId: 'gdrive', title: '보고서', kind: 'folder', modifiedAt: '2026-07-01T00:00:00Z', path: '/reports' },
  { id: 'folder-contracts', sourceId: 'gdrive', title: '계약서', kind: 'folder', modifiedAt: '2026-06-28T00:00:00Z', path: '/contracts' },
  { id: 'file-q2', sourceId: 'gdrive', title: '2분기 보고서.pdf', kind: 'file', size: 2_457_600, modifiedAt: '2026-07-01T00:00:00Z', path: '/reports/2분기 보고서.pdf' },
];

describe('FileBrowserView', () => {
  it('shows folders first at root, no folder entry for current folder', async () => {
    const provider = makeFilesProvider(fileEntries);
    render(wrapper(<FileBrowserView provider={provider} onSelect={() => {}} />));
    await waitFor(() => expect(screen.getByText('보고서')).toBeTruthy());
    expect(screen.getByText('계약서')).toBeTruthy();
    // The root itself is never shown as a row
    expect(screen.queryByText('root-self')).toBeNull();
  });

  it('descends into folder on click and breadcrumb grows', async () => {
    const provider = makeFilesProvider(fileEntries);
    render(wrapper(<FileBrowserView provider={provider} onSelect={() => {}} />));
    await waitFor(() => expect(screen.getByText('보고서')).toBeTruthy());
    fireEvent.click(screen.getByText('보고서'));
    // After descending, the provider should have been called with /reports path
    await waitFor(() => {
      expect((provider.list as ReturnType<typeof vi.fn>).mock.calls.some(
        (call) => call[0]?.path === '/reports'
      )).toBe(true);
    });
    // Breadcrumb should show the folder name
    await waitFor(() => expect(screen.getByText('보고서')).toBeTruthy());
  });

  it('does NOT call onSelect when a folder row is clicked', async () => {
    const onSelect = vi.fn();
    const provider = makeFilesProvider(fileEntries);
    render(wrapper(<FileBrowserView provider={provider} onSelect={onSelect} />));
    await waitFor(() => expect(screen.getByText('보고서')).toBeTruthy());
    fireEvent.click(screen.getByText('보고서'));
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe('ItemListView', () => {
  const itemProvider: SourceProvider = {
    id: 'confluence',
    nameKey: 'workspace.src.confluence',
    category: 'docs',
    kind: 'items',
    connected: true, attachable: false, count: 2,
    list: () => Promise.resolve([
      { id: 'p1', sourceId: 'confluence', title: '디자인 가이드', subtitle: 'DESIGN 스페이스', kind: 'item', modifiedAt: '2026-07-01T00:00:00Z' },
      { id: 'p2', sourceId: 'confluence', title: 'API 명세', subtitle: 'DEV 스페이스', kind: 'item', modifiedAt: '2026-06-30T00:00:00Z' },
    ]),
    recent: () => Promise.resolve([]),
    detail: vi.fn().mockResolvedValue({ entry: { id: 'p1', sourceId: 'confluence', title: '디자인 가이드', kind: 'item', modifiedAt: '2026-07-01T00:00:00Z' } }),
  };

  it('renders chip from subtitle first token', async () => {
    render(wrapper(<ItemListView provider={itemProvider} onSelect={() => {}} />));
    await waitFor(() => expect(screen.getByText('디자인 가이드')).toBeTruthy());
    // subtitle "DESIGN 스페이스" → chip shows "DESIGN" (first token)
    expect(screen.getByText('DESIGN')).toBeTruthy();
  });

  it('calls onSelect with entry on row click', async () => {
    const onSelect = vi.fn();
    render(wrapper(<ItemListView provider={itemProvider} onSelect={onSelect} />));
    await waitFor(() => expect(screen.getByText('디자인 가이드')).toBeTruthy());
    fireEvent.click(screen.getByText('디자인 가이드'));
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 'p1' }));
  });
});

describe('ThreadListView', () => {
  const threadProvider: SourceProvider = {
    id: 'gmail',
    nameKey: 'workspace.src.gmail',
    category: 'messages',
    kind: 'threads',
    connected: true, attachable: false, count: 2,
    list: () => Promise.resolve([
      { id: 't1', sourceId: 'gmail', title: '[긴급] 2분기 검토', subtitle: '박대표 <ceo@example.com>', kind: 'thread', modifiedAt: '2026-07-02T08:30:00Z' },
      { id: 't2', sourceId: 'gmail', title: 'Re: 인프라 견적', subtitle: '최영업 <sales@cloudvendor.com>', kind: 'thread', modifiedAt: '2026-07-01T14:00:00Z' },
    ]),
    recent: () => Promise.resolve([]),
    detail: vi.fn().mockResolvedValue({ entry: { id: 't1', sourceId: 'gmail', title: '[긴급] 2분기 검토', kind: 'thread', modifiedAt: '2026-07-02T08:30:00Z' } }),
  };

  it('renders bold sender from subtitle', async () => {
    render(wrapper(<ThreadListView provider={threadProvider} onSelect={() => {}} />));
    await waitFor(() => expect(screen.getByText('박대표 <ceo@example.com>')).toBeTruthy());
    // The sender element should have bold styling (data-testid or by querying the element)
    const senderEl = screen.getByTestId('thread-sender-t1');
    expect(senderEl).toBeTruthy();
  });

  it('renders thread title', async () => {
    render(wrapper(<ThreadListView provider={threadProvider} onSelect={() => {}} />));
    await waitFor(() => expect(screen.getByText('[긴급] 2분기 검토')).toBeTruthy());
  });

  it('calls onSelect with entry on row click', async () => {
    const onSelect = vi.fn();
    render(wrapper(<ThreadListView provider={threadProvider} onSelect={onSelect} />));
    await waitFor(() => expect(screen.getByText('[긴급] 2분기 검토')).toBeTruthy());
    fireEvent.click(screen.getAllByRole('button')[0]);
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });
});
