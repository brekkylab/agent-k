/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { FallbackCard } from '../FallbackCard';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

afterEach(cleanup);

describe('FallbackCard', () => {
  it('shows filename and triggers download on click', () => {
    const onDownload = vi.fn();
    render(<FallbackCard filename="report.docx" reason="unsupported" onDownload={onDownload} />);
    expect(screen.getByText('report.docx')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /preview.download/ }));
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it('renders distinct title per reason', () => {
    const { rerender } = render(<FallbackCard filename="a.bin" reason="too-large" onDownload={() => {}} />);
    expect(screen.getByText('preview.too_large_title')).toBeTruthy();
    rerender(<FallbackCard filename="a.bin" reason="error" onDownload={() => {}} />);
    expect(screen.getByText('preview.error_title')).toBeTruthy();
  });
});
