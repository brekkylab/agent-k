import { useLayoutEffect, useRef, useState } from 'react';
import { Document, Page } from 'react-pdf';
import 'react-pdf/dist/Page/AnnotationLayer.css';
import 'react-pdf/dist/Page/TextLayer.css';
import './pdfWorker'; // side-effect: configure worker once

interface Props { objectUrl: string }

export function PdfView({ objectUrl }: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [numPages, setNumPages] = useState(0);
  const [width, setWidth] = useState(760);

  useLayoutEffect(() => {
    function measure() {
      if (wrapRef.current) setWidth(Math.min(900, wrapRef.current.clientWidth - 24));
    }
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, []);

  return (
    <div className="cw-preview-pdf" ref={wrapRef}>
      <Document file={objectUrl} onLoadSuccess={({ numPages }) => setNumPages(numPages)}>
        {Array.from({ length: numPages }, (_, i) => (
          <Page key={i} pageNumber={i + 1} width={width} renderAnnotationLayer renderTextLayer />
        ))}
      </Document>
    </div>
  );
}
