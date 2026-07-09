import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { ReactNode } from 'react';

// i18n: t:(k) => k so assertions match i18n keys directly
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

// Must mock TanStack Router — createFileRoute called at module load
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (opts: unknown) => opts,
  useNavigate: () => vi.fn(),
  useParams: () => ({}),
  Link: ({ children, to }: { children: ReactNode; to: string }) => <a href={to}>{children}</a>,
}));

import { UnifiedList } from '../components/UnifiedList';
import type { SourceEntry } from '../types';

const entries: SourceEntry[] = [
  { id: 'e1', sourceId: 'gdrive', title: 'Q2 보고서.pdf', kind: 'file', modifiedAt: '2026-07-01T00:00:00Z' },
  { id: 'e2', sourceId: 'slack', title: '배포 알림 스레드', kind: 'thread', modifiedAt: '2026-06-30T00:00:00Z' },
  { id: 'e3', sourceId: 'confluence', title: '온보딩 가이드', kind: 'item', modifiedAt: '2026-06-29T00:00:00Z' },
];

describe('UnifiedList', () => {
  it('renders source name in fixed column for each row', () => {
    render(<UnifiedList entries={entries} onSelect={() => {}} />);
    // Each row should show sourceId as source name column text (with t(key) = key,
    // the source name column shows 'workspace.src.gdrive', 'workspace.src.slack', 'workspace.src.confluence')
    expect(screen.getByText('workspace.src.gdrive')).toBeTruthy();
    expect(screen.getByText('workspace.src.slack')).toBeTruthy();
    expect(screen.getByText('workspace.src.confluence')).toBeTruthy();
  });

  it('renders rows in given order', () => {
    render(<UnifiedList entries={entries} onSelect={() => {}} />);
    const titles = screen.getAllByTestId('unified-row-title').map((el) => el.textContent);
    expect(titles).toEqual(['Q2 보고서.pdf', '배포 알림 스레드', '온보딩 가이드']);
  });

  it('filters by title when search input typed', () => {
    render(<UnifiedList entries={entries} onSelect={() => {}} />);
    const input = screen.getByPlaceholderText('workspace.searchAll');
    fireEvent.change(input, { target: { value: '보고서' } });
    expect(screen.getByTestId('unified-row-title').textContent).toBe('Q2 보고서.pdf');
    expect(screen.queryByText('배포 알림 스레드')).toBeNull();
  });

  it('calls onSelect with entry on row click', () => {
    const onSelect = vi.fn();
    render(<UnifiedList entries={entries} onSelect={onSelect} />);
    const rows = screen.getAllByRole('button');
    fireEvent.click(rows[0]);
    expect(onSelect).toHaveBeenCalledWith(entries[0]);
  });
});
