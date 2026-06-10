import { useLayoutEffect, useRef, useState } from 'react';
import { Document, Page } from 'react-pdf';
import 'react-pdf/dist/Page/AnnotationLayer.css';
import 'react-pdf/dist/Page/TextLayer.css';
import './pdfWorker'; // side-effect: configure worker once
import { useZoom, useWheelZoom } from './useZoom';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string }

export function PdfView({ objectUrl }: Props) {
  const stageRef = useRef<HTMLDivElement>(null);
  const [numPages, setNumPages] = useState(0);
  const [fitWidth, setFitWidth] = useState(760);
  const zoom = useZoom();
  useWheelZoom(stageRef, zoom);

  useLayoutEffect(() => {
    function measure() {
      if (stageRef.current) setFitWidth(Math.min(900, stageRef.current.clientWidth - 48));
    }
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, []);

  return (
    <>
      <div className="cw-preview-stage cw-preview-pdf" ref={stageRef} onDoubleClick={zoom.toggle}>
        <div className="cw-preview-content cw-preview-pdf-doc">
          <Document file={objectUrl} onLoadSuccess={({ numPages }) => setNumPages(numPages)}>
            {Array.from({ length: numPages }, (_, i) => (
              <Page key={i} pageNumber={i + 1} width={fitWidth * zoom.scale} renderAnnotationLayer renderTextLayer />
            ))}
          </Document>
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
