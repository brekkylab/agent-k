import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';

const navigate = vi.fn();

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, className }: { children: ReactNode; className?: string }) => (
    <a className={className}>{children}</a>
  ),
  useNavigate: () => navigate,
}));

import { SourceRail } from '../components/SourceRail';

describe('SourceRail add source catalog', () => {
  beforeEach(() => {
    localStorage.clear();
    navigate.mockClear();
  });

  it('shows unconnected sources faintly and can hide them from the rail', () => {
    render(<SourceRail activeSourceId={null} />);

    expect(screen.getByText('workspace.groups.sources')).toBeTruthy();
    expect(screen.getByText('workspace.groups.knowledge')).toBeTruthy();
    expect(screen.getByText('workspace.src.knowledge')).toBeTruthy();

    const github = screen.getByRole('button', { name: /workspace.src.github/ });
    expect(github.className).toContain('is-unconnected');

    fireEvent.click(screen.getByLabelText('workspace.hideUnconnected'));
    expect(screen.queryByRole('button', { name: /workspace.src.github/ })).toBeNull();
  });

  it('shows connection instructions, required values, then opens the connected source', () => {
    render(<SourceRail activeSourceId={null} />);

    fireEvent.click(screen.getByText('workspace.addSource'));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).queryByRole('button', { name: /workspace.src.knowledge/ })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: /workspace.src.github/ }));

    expect(within(dialog).getByText('workspace.connect.instructions.github')).toBeTruthy();
    expect(within(dialog).getByLabelText('workspace.connect.fields.repositoryUrl')).toBeTruthy();
    expect(within(dialog).getByLabelText('workspace.connect.fields.accessToken')).toBeTruthy();

    fireEvent.change(within(dialog).getByLabelText('workspace.connect.fields.repositoryUrl'), {
      target: { value: 'https://github.com/acme/app' },
    });
    fireEvent.change(within(dialog).getByLabelText('workspace.connect.fields.accessToken'), {
      target: { value: 'ghp_mock' },
    });
    fireEvent.click(within(dialog).getByText('workspace.connect.action'));

    expect(localStorage.getItem('cw.workspace.connectedSources')).toContain('github');
    expect(navigate).toHaveBeenCalledWith({
      to: '/workspace/$sourceId',
      params: { sourceId: 'github' },
    });
  });

  it('shows a setup guide inside the same dialog for token-based sources', () => {
    render(<SourceRail activeSourceId={null} />);

    fireEvent.click(screen.getByText('workspace.addSource'));
    const dialog = screen.getByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: /workspace.src.github/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'workspace.connect.tabs.guide' }));

    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(within(dialog).getByText('workspace.connect.guides.github.title')).toBeTruthy();
    expect(within(dialog).getByText('workspace.connect.guides.github.step2')).toBeTruthy();
    expect(within(dialog).getByText('workspace.connect.guides.github.scopePullRequests')).toBeTruthy();

    fireEvent.click(within(dialog).getByRole('button', { name: /workspace.src.linear/ }));
    expect(within(dialog).getByText('workspace.connect.instructions.linear')).toBeTruthy();

    fireEvent.click(within(dialog).getByRole('button', { name: 'workspace.connect.tabs.guide' }));
    expect(within(dialog).getByText('workspace.connect.guides.linear.title')).toBeTruthy();
    expect(within(dialog).getByText('workspace.connect.guides.linear.step3')).toBeTruthy();

    fireEvent.click(within(dialog).getByRole('button', { name: 'workspace.connect.tabs.connect' }));
    expect(within(dialog).getByLabelText('workspace.connect.fields.workspaceUrl')).toBeTruthy();
    expect(within(dialog).getByLabelText('workspace.connect.fields.apiKey')).toBeTruthy();
  });
});
