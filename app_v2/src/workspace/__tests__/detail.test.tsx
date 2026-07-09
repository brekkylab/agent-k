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
// Provider registry mock — each test installs its own detail() behavior
vi.mock('@/workspace/providers', () => ({
  getProvider: vi.fn(),
}));
// Lightbox mock — the real modal pulls in pdfjs (needs DOMMatrix, absent in
// jsdom) and its internals are covered elsewhere; here we only care that the
// panel can mount it.
vi.mock('@/components/FilePreviewModal', () => ({
  FilePreviewModal: ({ name }: { name: string }) => <div data-testid="lightbox">{name}</div>,
}));

import { deleteEntry } from '@/api/workspace';
import { getProvider } from '@/workspace/providers';
import { DetailPanel } from '../components/DetailPanel';
import type { SourceDetail, SourceEntry, SourceProvider } from '../types';

function wrapper(children: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

function installProvider(id: SourceProvider['id'], detail: SourceDetail): SourceProvider {
  const provider: SourceProvider = {
    id,
    nameKey: `workspace.src.${id}`,
    category: 'files',
    kind: 'files',
    connected: true,
    attachable: id === 'local',
    count: null,
    list: vi.fn().mockResolvedValue([]),
    recent: vi.fn().mockResolvedValue([]),
    detail: vi.fn().mockResolvedValue(detail),
  };
  (getProvider as ReturnType<typeof vi.fn>).mockImplementation((pid: string) =>
    pid === id ? provider : undefined,
  );
  return provider;
}

beforeEach(() => {
  vi.mocked(getProvider).mockReset();
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
});
