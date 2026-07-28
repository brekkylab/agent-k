import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { getDocument, type PDFDocumentProxy } from 'pdfjs-dist';
import { EventBus, PDFPageView } from 'pdfjs-dist/web/pdf_viewer.mjs';
import 'pdfjs-dist/web/pdf_viewer.css';
import './pdfWorker'; // side-effect: configure worker once
import { useZoom, useWheelZoom } from './useZoom';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string }

export function PdfView({ objectUrl }: Props) {
  const stageRef = useRef<HTMLDivElement>(null);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [numPages, setNumPages] = useState(0);
  const [fitWidth, setFitWidth] = useState(760);
  const zoom = useZoom();
  useWheelZoom(stageRef, zoom);

  useEffect(() => {
    setDocument(null);
    setNumPages(0);

    const loadingTask = getDocument({ url: objectUrl });
    let disposed = false;

    void loadingTask.promise
      .then((loaded) => {
        if (disposed) return;
        setDocument(loaded);
        setNumPages(loaded.numPages);
      })
      .catch((error: unknown) => {
        if (!disposed) console.error('Failed to load PDF preview', error);
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
        <div className="cw-preview-content cw-preview-pdf-doc pdfViewer">
          {document && Array.from({ length: numPages }, (_, i) => (
            <PdfPage
              key={i}
              document={document}
              pageNumber={i + 1}
              width={fitWidth * zoom.scale}
            />
          ))}
        </div>
      </div>
      <ZoomControls
        scale={zoom.scale}
        onZoomIn={zoom.zoomIn}
        onZoomOut={zoom.zoomOut}
        onReset={zoom.reset}
        canZoomIn={zoom.canZoomIn}
        canZoomOut={zoom.canZoomOut}
      />
    </>
  );
}

function PdfPage({
  document,
  pageNumber,
  width,
}: {
  document: PDFDocumentProxy;
  pageNumber: number;
  width: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const abort = new AbortController();
    let pageView: PDFPageView | undefined;

    void document
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
        if (!abort.signal.aborted) console.error(`Failed to render PDF page ${pageNumber}`, error);
      });

    return () => {
      abort.abort();
      pageView?.destroy();
      container.replaceChildren();
    };
  }, [document, pageNumber, width]);

  return <div className="cw-pdfjs-page" ref={containerRef} />;
}
