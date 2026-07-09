// pdf.js worker wiring — imported once for its side effect.
// The worker URL is resolved from the installed pdfjs-dist package so the
// API version and worker version are always in sync.
import { pdfjs } from 'react-pdf';

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.min.mjs',
  import.meta.url,
).toString();
