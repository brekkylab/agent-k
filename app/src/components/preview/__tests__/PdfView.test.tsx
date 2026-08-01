/** @vitest-environment jsdom */
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));

const mocks = vi.hoisted(() => {
  const destroyLoadingTask = vi.fn();
  const getPage = vi.fn(async () => ({
    getViewport: () => ({ width: 500 }),
  }));
  const document = {
    numPages: 2,
    getPage,
  };
  const getDocument = vi.fn(() => ({
    promise: Promise.resolve(document),
    destroy: destroyLoadingTask,
  }));
  const draw = vi.fn(async () => {});
  const destroyPage = vi.fn();
  return {
    destroyLoadingTask,
    getPage,
    document,
    getDocument,
    draw,
    destroyPage,
  };
});

vi.mock('pdfjs-dist', () => ({
  getDocument: mocks.getDocument,
  GlobalWorkerOptions: {},
}));

vi.mock('pdfjs-dist/web/pdf_viewer.mjs', () => ({
  EventBus: class EventBus {},
  PDFPageView: class PDFPageView {
    constructor({ container }: { container: HTMLDivElement }) {
      container.append(document.createElement('div'));
    }
    setPdfPage() {}
    draw = mocks.draw;
    destroy = mocks.destroyPage;
  },
}));

import { PdfView } from '../PdfView';

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('PdfView', () => {
  it('loads and renders every page with the direct PDF.js 6 API', async () => {
    const { unmount } = render(<PdfView objectUrl="blob:pdf" />);

    await waitFor(() => expect(mocks.getPage).toHaveBeenCalledTimes(2));
    expect(mocks.getDocument).toHaveBeenCalledWith({ url: 'blob:pdf' });
    expect(mocks.getPage).toHaveBeenNthCalledWith(1, 1);
    expect(mocks.getPage).toHaveBeenNthCalledWith(2, 2);
    expect(mocks.draw).toHaveBeenCalledTimes(2);

    unmount();
    expect(mocks.destroyLoadingTask).toHaveBeenCalledOnce();
    expect(mocks.destroyPage).toHaveBeenCalledTimes(2);
  });

  it('shows an error instead of an empty stage when the document fails to load', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.getDocument.mockImplementationOnce(() => ({
      promise: Promise.reject(new Error('corrupt pdf')),
      destroy: mocks.destroyLoadingTask,
    }));

    render(<PdfView objectUrl="blob:broken" />);

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.getByText('preview.error_title')).toBeTruthy();
    // Zoom controls are meaningless with nothing rendered.
    expect(screen.queryByLabelText('preview.zoom_in')).toBeNull();
    expect(logged).toHaveBeenCalled();
    logged.mockRestore();
  });

  it('labels a single failed page without discarding the rest of the document', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.getPage.mockImplementationOnce(() => Promise.reject(new Error('bad page')));

    render(<PdfView objectUrl="blob:pdf" />);

    await waitFor(() => expect(screen.getByText('preview.error_title')).toBeTruthy());
    // The document itself loaded, so the surviving page still draws.
    expect(mocks.draw).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(logged).toHaveBeenCalled();
    logged.mockRestore();
  });
});
