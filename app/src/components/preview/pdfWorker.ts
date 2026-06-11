// pdf.js worker wiring — imported once for its side effect.
// worker는 CDN/하드코딩 버전이 아니라 설치된 pdfjs-dist에서 번들러가 해석하게 한다.
// 이렇게 하면 react-pdf가 쓰는 pdfjs API 버전과 worker 버전이 구조적으로 일치한다.
import { pdfjs } from 'react-pdf';

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.min.mjs',
  import.meta.url,
).toString();
