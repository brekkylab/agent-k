import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getDocument, type PDFDocumentProxy } from 'pdfjs-dist';
import { EventBus, PDFPageView } from 'pdfjs-dist/web/pdf_viewer.mjs';
import 'pdfjs-dist/web/pdf_viewer.css';
import './pdfWorker'; // side-effect: configure worker once
import { useZoom, useWheelZoom } from './useZoom';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string }

// `pdfDoc`, not `document`: naming the state after the proxy would shadow the
// DOM global for the whole component body, so any later DOM call inside these
// two functions would silently resolve to a PDF document instead.
export function PdfView({ objectUrl }: Props) {
  const { t } = useTranslation('common');
  const stageRef = useRef<HTMLDivElement>(null);
  const [pdfDoc, setPdfDoc] = useState<PDFDocumentProxy | null>(null);
  const [numPages, setNumPages] = useState(0);
  const [failed, setFailed] = useState(false);
  const [fitWidth, setFitWidth] = useState(760);
  const zoom = useZoom();
  useWheelZoom(stageRef, zoom);

  useEffect(() => {
    setPdfDoc(null);
    setNumPages(0);
    setFailed(false);

    const loadingTask = getDocument({ url: objectUrl });
    let disposed = false;

    void loadingTask.promise
      .then((loaded) => {
        if (disposed) return;
        setPdfDoc(loaded);
        setNumPages(loaded.numPages);
      })
      .catch((error: unknown) => {
        if (disposed) return;
        console.error('Failed to load PDF preview', error);
        // Without this the stage stays empty, which reads as a blank PDF.
        setFailed(true);
      });

    return () => {
      disposed = true;
      void loadingTask.destroy();
    };
  }, [objectUrl]);

  useLayoutEffect(() => {
    function measure() {
      if (stageRef.current) {
        setFitWidth(Math.max(320, Math.min(900, stageRef.current.clientWidth - 48)));
      }
    }
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, []);

  return (
    <>
      <div className="cw-preview-stage cw-preview-pdf" ref={stageRef} onDoubleClick={zoom.toggle}>
        {failed ? (
          <div className="cw-preview-pdf-error" role="alert">
            <div className="cw-preview-fallback-title">{t('preview.error_title')}</div>
            <div className="cw-preview-fallback-body">{t('preview.error_body')}</div>
          </div>
        ) : (
          <div className="cw-preview-content cw-preview-pdf-doc pdfViewer">
            {pdfDoc && Array.from({ length: numPages }, (_, i) => (
              <PdfPage
                key={i}
                pdfDoc={pdfDoc}
                pageNumber={i + 1}
                width={fitWidth * zoom.scale}
              />
            ))}
          </div>
        )}
      </div>
      {!failed && (
        <ZoomControls
          scale={zoom.scale}
          onZoomIn={zoom.zoomIn}
          onZoomOut={zoom.zoomOut}
          onReset={zoom.reset}
          canZoomIn={zoom.canZoomIn}
          canZoomOut={zoom.canZoomOut}
        />
      )}
    </>
  );
}

function PdfPage({
  pdfDoc,
  pageNumber,
  width,
}: {
  pdfDoc: PDFDocumentProxy;
  pageNumber: number;
  width: number;
}) {
  const { t } = useTranslation('common');
  const containerRef = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    setFailed(false);
    const abort = new AbortController();
    let pageView: PDFPageView | undefined;

    void pdfDoc
      .getPage(pageNumber)
      .then(async (page) => {
        if (abort.signal.aborted) return;
        const unscaledViewport = page.getViewport({ scale: 1 });
        const scale = width / unscaledViewport.width;
        pageView = new PDFPageView({
          container,
          eventBus: new EventBus(),
          id: pageNumber,
          scale,
          defaultViewport: unscaledViewport,
          abortSignal: abort.signal,
        });
        pageView.setPdfPage(page);
        await pageView.draw();
      })
      .catch((error: unknown) => {
        if (abort.signal.aborted) return;
        console.error(`Failed to render PDF page ${pageNumber}`, error);
        // One bad page leaves a gap in an otherwise fine document; label it
        // rather than letting it read as a blank page.
        setFailed(true);
      });

    return () => {
      abort.abort();
      pageView?.destroy();
      container.replaceChildren();
    };
  }, [pdfDoc, pageNumber, width]);

  // The ref'd node is owned by PDF.js — the effect's cleanup calls
  // replaceChildren() on it, so React must not render children there. Keep the
  // error label as a sibling under a wrapper React owns.
  return (
    <div className="cw-pdfjs-page">
      <div ref={containerRef} />
      {failed && <div className="cw-pdfjs-page-error">{t('preview.error_title')}</div>}
    </div>
  );
}
