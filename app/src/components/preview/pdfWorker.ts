// PDF.js worker wiring — imported once for its side effect.
// Resolve the worker from the same direct pdfjs-dist package as the API so the
// two versions cannot drift independently.
import { GlobalWorkerOptions } from 'pdfjs-dist';

GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.min.mjs',
  import.meta.url,
).toString();
