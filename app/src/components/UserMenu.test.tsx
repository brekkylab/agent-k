/** @vitest-environment jsdom */
import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

import { UserMenu } from './UserMenu';

afterEach(() => cleanup());

describe('UserMenu', () => {
  it('is closed initially and opens on trigger click', () => {
    render(<UserMenu onOpenSettings={vi.fn()} onLogout={vi.fn()} />);
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'actions.user_menu' }));
    expect(screen.getByRole('menu')).toBeTruthy();
  });

  it('User Settings item calls onOpenSettings and closes; does NOT log out', () => {
    const onOpenSettings = vi.fn();
    const onLogout = vi.fn();
    render(<UserMenu onOpenSettings={onOpenSettings} onLogout={onLogout} />);
    fireEvent.click(screen.getByRole('button', { name: 'actions.user_menu' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'actions.user_settings' }));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    expect(onLogout).not.toHaveBeenCalled();
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('Log out item calls onLogout only after being selected from the menu', () => {
    const onLogout = vi.fn();
    render(<UserMenu onOpenSettings={vi.fn()} onLogout={onLogout} />);
    // Clicking only the trigger must NOT log out (regression: it used to log out instantly).
    fireEvent.click(screen.getByRole('button', { name: 'actions.user_menu' }));
    expect(onLogout).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('menuitem', { name: 'actions.logout' }));
    expect(onLogout).toHaveBeenCalledTimes(1);
  });

  it('closes on outside mousedown', () => {
    render(<UserMenu onOpenSettings={vi.fn()} onLogout={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'actions.user_menu' }));
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
  });
});
