/** @vitest-environment jsdom */
import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from 'vitest';
import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { renderWithProviders } from '@/test-utils';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));
// useAuthStore가 transitively 끌어오는 @/i18n의 import-time init을 피하고,
// 다이얼로그가 쓰는 SUPPORTED_LANGUAGES / LANGUAGE_STORAGE_KEY를 제공한다.
vi.mock('@/i18n', () => ({
  SUPPORTED_LANGUAGES: ['en', 'ko'],
  LANGUAGE_STORAGE_KEY: 'cw-lang',
  default: { language: 'en', changeLanguage: vi.fn() },
}));
vi.mock('@/api/auth', () => ({ updateMe: vi.fn() }));

import { useUserSettingsDialog } from './useUserSettingsDialog';
import { updateMe } from '@/api/auth';
import { useAuthStore } from '@/stores/auth';

const USER = {
  id: 'u1', name: 'Jeffrey', username: 'jeffrey', roleLabel: 'Member',
  avatar: 'JE', color: 'var(--cw-accent)', preferredLanguage: 'en' as const,
};

const SAVE = 'actions.save';
const CANCEL = 'actions.cancel';
const NAME = 'user_settings.fields.display_name';
const LANG = 'user_settings.fields.language';
const SAVING = 'state.saving';

function Harness() {
  const { open, dialog } = useUserSettingsDialog();
  return (<><button onClick={open}>open</button>{dialog}</>);
}

beforeAll(() => {
  // <Select>가 열릴 때 scrollIntoView 호출 → jsdom엔 레이아웃 없음 → no-op stub.
  Element.prototype.scrollIntoView = vi.fn();
});
beforeEach(() => { useAuthStore.setState({ currentUser: USER }); vi.mocked(updateMe).mockReset(); });
afterEach(() => { cleanup(); useAuthStore.setState({ currentUser: null }); });

describe('useUserSettingsDialog', () => {
  it('renders nothing until opened', () => {
    renderWithProviders(<Harness />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens prefilled, with Save disabled until something changes', () => {
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect((screen.getByLabelText(NAME) as HTMLInputElement).value).toBe('Jeffrey');
    expect(screen.getByRole('button', { name: LANG }).textContent).toContain('English');
    expect((screen.getByRole('button', { name: SAVE }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('editing the name enables Save; saving PATCHes both fields and closes', async () => {
    vi.mocked(updateMe).mockResolvedValue({ ...USER, name: 'Jeff' });
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    fireEvent.change(screen.getByLabelText(NAME), { target: { value: 'Jeff' } });
    expect((screen.getByRole('button', { name: SAVE }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: SAVE }));
    await waitFor(() => expect(updateMe).toHaveBeenCalledWith({ displayName: 'Jeff', preferredLanguage: 'en' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('changing the language enables Save and is included in the PATCH', async () => {
    vi.mocked(updateMe).mockResolvedValue({ ...USER, preferredLanguage: 'ko' });
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    fireEvent.click(screen.getByRole('button', { name: LANG }));
    fireEvent.click(screen.getByRole('option', { name: '한국어' }));
    fireEvent.click(screen.getByRole('button', { name: SAVE }));
    await waitFor(() => expect(updateMe).toHaveBeenCalledWith({ displayName: 'Jeffrey', preferredLanguage: 'ko' }));
  });

  it('Cancel closes without saving', () => {
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    fireEvent.change(screen.getByLabelText(NAME), { target: { value: 'Jeff' } });
    fireEvent.click(screen.getByRole('button', { name: CANCEL }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(updateMe).not.toHaveBeenCalled();
  });

  it('ESC closes the dialog', () => {
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    expect(screen.getByRole('dialog')).toBeTruthy();
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('clicking the backdrop closes the dialog', () => {
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    const backdrop = document.querySelector('.cw-dialog-backdrop') as HTMLElement;
    fireEvent.mouseDown(backdrop);
    fireEvent.click(backdrop);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows the saving state and blocks resubmit while the save is in flight', async () => {
    // 미해결(pending) 프로미스로 묶어 두면 거부가 없어 unhandled-rejection 없이 pending 상태만 검증된다.
    vi.mocked(updateMe).mockReturnValue(new Promise(() => {}) as never);
    renderWithProviders(<Harness />);
    fireEvent.click(screen.getByText('open'));
    fireEvent.change(screen.getByLabelText(NAME), { target: { value: 'Jeff' } });
    fireEvent.click(screen.getByRole('button', { name: SAVE }));
    const saving = await screen.findByRole('button', { name: SAVING });
    expect((saving as HTMLButtonElement).disabled).toBe(true);
  });
});
