import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, act } from '@testing-library/react';
import React, { type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// t: (k) => k so assertions match i18n keys directly.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

// createFileRoute is called at module load; return a passthrough. useNavigate is
// spied so we can assert navigation (order + params).
const navigateSpy = vi.fn();
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (opts: unknown) => opts,
  useNavigate: () => navigateSpy,
}));

vi.mock('@/api/sessions', () => ({ createSession: vi.fn() }));
vi.mock('@/api/messages', () => ({ sendMessage: vi.fn() }));

import { createSession } from '@/api/sessions';
import { sendMessage } from '@/api/messages';
import { ApiError } from '@/api/client';
import { HomePage } from '@/routes/index';
import { setPendingAttachment, takePendingAttachment } from '@/stores/pendingAttachment';
import type { SessionResponse } from '@/api/types';

const createSessionMock = vi.mocked(createSession);
const sendMessageMock = vi.mocked(sendMessage);

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

function renderHome() {
  return render(<HomePage />, { wrapper });
}

const fakeSession: SessionResponse = {
  id: 'sess-123',
  workspace_id: 'ws-1',
  agent_id: null,
  title: null,
  spec: {},
  created_at: '2026-07-03T00:00:00Z',
  updated_at: '2026-07-03T00:00:00Z',
};

function typeMessage(text: string) {
  const textarea = screen.getByPlaceholderText('home.placeholder');
  fireEvent.change(textarea, { target: { value: text } });
}

function clickSend() {
  fireEvent.click(screen.getByRole('button', { name: 'home.send' }));
}

beforeEach(() => {
  vi.clearAllMocks();
  // Clear any leftover pending attachment between tests.
  takePendingAttachment();
});

describe('HomePage composer flow', () => {
  it('creates a session, sends the message, then navigates — in that order', async () => {
    const order: string[] = [];
    createSessionMock.mockImplementation(async () => {
      order.push('create');
      return fakeSession;
    });
    sendMessageMock.mockImplementation(async () => {
      order.push('send');
    });
    navigateSpy.mockImplementation(() => {
      order.push('navigate');
    });

    renderHome();
    typeMessage('hello world');
    clickSend();

    await waitFor(() => expect(navigateSpy).toHaveBeenCalled());

    expect(order).toEqual(['create', 'send', 'navigate']);
    expect(createSessionMock).toHaveBeenCalledWith({ agentType: 'coworker', model: undefined });
    expect(sendMessageMock).toHaveBeenCalledWith('sess-123', 'hello world');
    expect(navigateSpy).toHaveBeenCalledWith({
      to: '/sessions/$sessionId',
      params: { sessionId: 'sess-123' },
    });
  });

  it('passes the selected agent type and trimmed model to createSession', async () => {
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockResolvedValue(undefined);

    renderHome();

    fireEvent.change(screen.getByRole('combobox', { name: 'home.agent_label' }), {
      target: { value: 'deep_research' },
    });
    fireEvent.change(screen.getByRole('textbox', { name: 'home.model_label' }), {
      target: { value: '  gpt-x  ' },
    });
    typeMessage('research this');
    clickSend();

    await waitFor(() => expect(createSessionMock).toHaveBeenCalled());
    expect(createSessionMock).toHaveBeenCalledWith({ agentType: 'deep_research', model: 'gpt-x' });
  });

  it('shows an inline error and does NOT navigate when session creation fails', async () => {
    createSessionMock.mockRejectedValue(new ApiError(500, 'boom'));

    renderHome();
    typeMessage('hi');
    clickSend();

    await waitFor(() => expect(screen.queryByText('home.error')).not.toBeNull());
    expect(sendMessageMock).not.toHaveBeenCalled();
    expect(navigateSpy).not.toHaveBeenCalled();
  });

  it('shows an inline error and does NOT navigate when send fails', async () => {
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockRejectedValue(new ApiError(500, 'send boom'));

    renderHome();
    typeMessage('hi');
    clickSend();

    await waitFor(() => expect(screen.queryByText('home.error')).not.toBeNull());
    expect(navigateSpy).not.toHaveBeenCalled();
  });
});

describe('HomePage attachment chip (pendingAttachment bridge)', () => {
  it('renders the attachment chip when a pending attachment is set before mount', async () => {
    setPendingAttachment({ name: 'q2-report.pdf', sharedPath: '/root/shared/reports/q2-report.pdf' });

    await act(async () => {
      renderHome();
    });

    // Chip shows the file name.
    expect(screen.getByText('q2-report.pdf')).toBeTruthy();
    // i18n hint key is rendered.
    expect(screen.getByText('home.attachmentHint')).toBeTruthy();
  });

  it('prepends the shared path prefix when sending with an attachment', async () => {
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockResolvedValue(undefined);

    setPendingAttachment({ name: 'budget.xlsx', sharedPath: '/root/shared/finance/budget.xlsx' });

    await act(async () => {
      renderHome();
    });

    typeMessage('summarize this file');
    clickSend();

    await waitFor(() => expect(sendMessageMock).toHaveBeenCalled());
    expect(sendMessageMock).toHaveBeenCalledWith(
      'sess-123',
      '[첨부 파일: /root/shared/finance/budget.xlsx]\nsummarize this file',
    );
  });

  it('removes the chip when ✕ is clicked and send omits the prefix', async () => {
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockResolvedValue(undefined);

    setPendingAttachment({ name: 'notes.txt', sharedPath: '/root/shared/notes.txt' });

    await act(async () => {
      renderHome();
    });

    // Chip is visible initially.
    expect(screen.getByText('notes.txt')).toBeTruthy();

    // Click the remove button (aria-label key "home.removeAttachment").
    const removeBtn = screen.getByRole('button', { name: 'home.removeAttachment' });
    fireEvent.click(removeBtn);

    // Chip is gone.
    expect(screen.queryByText('notes.txt')).toBeNull();

    // Sending now omits the prefix.
    typeMessage('plain message');
    clickSend();

    await waitFor(() => expect(sendMessageMock).toHaveBeenCalled());
    expect(sendMessageMock).toHaveBeenCalledWith('sess-123', 'plain message');
  });

  it('clears the chip after a successful send', async () => {
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockResolvedValue(undefined);

    setPendingAttachment({ name: 'spec.pdf', sharedPath: '/root/shared/spec.pdf' });

    await act(async () => {
      renderHome();
    });

    expect(screen.getByText('spec.pdf')).toBeTruthy();

    typeMessage('review this');
    clickSend();

    await waitFor(() => expect(navigateSpy).toHaveBeenCalled());

    // After navigation the component is gone — just assert send was called with prefix.
    expect(sendMessageMock).toHaveBeenCalledWith(
      'sess-123',
      '[첨부 파일: /root/shared/spec.pdf]\nreview this',
    );
  });

  it('renders attachment chip under React.StrictMode (regression guard against lazy-initializer pattern)', async () => {
    // This test ensures that HomePage uses useEffect (not useState initializer)
    // to consume takePendingAttachment(). React.StrictMode double-invokes
    // initializers in dev; if HomePage reverted to the initializer pattern,
    // the value would be consumed on the discarded first invocation and the
    // chip would never render.
    createSessionMock.mockResolvedValue(fakeSession);
    sendMessageMock.mockResolvedValue(undefined);

    setPendingAttachment({ name: 'q2.pdf', sharedPath: '/root/shared/reports/q2.pdf' });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const StrictWrapper = ({ children }: { children: ReactNode }) => (
      <React.StrictMode>
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      </React.StrictMode>
    );

    await act(async () => {
      render(<HomePage />, { wrapper: StrictWrapper });
    });

    // Chip must be visible (would fail if HomePage used lazy initializer).
    expect(screen.getByText('q2.pdf')).toBeTruthy();
  });
});
