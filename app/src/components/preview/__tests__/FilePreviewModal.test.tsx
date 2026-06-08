/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));

const fetchFileBlob = vi.fn();
const downloadFileByGlobalPath = vi.fn();
vi.mock('@/api/dirents', () => ({
  fetchFileBlob: (...a: unknown[]) => fetchFileBlob(...a),
  downloadFileByGlobalPath: (...a: unknown[]) => downloadFileByGlobalPath(...a),
}));

// PdfView statically imports react-pdf → pdfjs-dist, which calls DOMMatrix at
// module-init time and crashes under jsdom (DOMMatrix is not defined). Stub it
// so FilePreviewModal's module graph can load in the unit test.
vi.mock('@/components/preview/PdfView', () => ({ PdfView: () => null }));

import { FilePreviewModal } from '../../FilePreviewModal';

const revoke = vi.fn();
beforeEach(() => {
  fetchFileBlob.mockReset();
  revoke.mockReset();
  URL.createObjectURL = vi.fn(() => 'blob:x');
  URL.revokeObjectURL = revoke;
});
afterEach(cleanup);

function blobOf(size: number, type: string) {
  const b = new Blob([new Uint8Array(0)], { type });
  Object.defineProperty(b, 'size', { value: size });
  return b;
}

describe('FilePreviewModal', () => {
  it('shows too-large fallback when blob exceeds 20MB', async () => {
    fetchFileBlob.mockResolvedValue(blobOf(21 * 1024 * 1024, 'application/pdf'));
    render(<FilePreviewModal globalPath="projects/p/shared/a.pdf" onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText('preview.too_large_title')).toBeTruthy());
  });

  it('shows error fallback when fetch fails', async () => {
    fetchFileBlob.mockRejectedValue(new Error('boom'));
    render(<FilePreviewModal globalPath="projects/p/shared/a.png" onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText('preview.error_title')).toBeTruthy());
  });

  it('renders image and revokes object URL on unmount', async () => {
    fetchFileBlob.mockResolvedValue(blobOf(1000, 'image/png'));
    const { unmount } = render(<FilePreviewModal globalPath="projects/p/shared/a.png" onClose={() => {}} />);
    await waitFor(() => expect(document.querySelector('img')).toBeTruthy());
    unmount();
    expect(revoke).toHaveBeenCalledWith('blob:x');
  });
});
