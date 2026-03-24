import type { Document, Knowledge } from "./types";

export const DUMMY_DOCUMENTS: Document[] = [
  { id: "doc-1", name: "시장분석.pdf", size: 2457600, uploadedAt: new Date("2026-03-15") },
  { id: "doc-2", name: "경쟁사.docx", size: 1126400, uploadedAt: new Date("2026-03-14") },
  { id: "doc-3", name: "API스펙.pdf", size: 3276800, uploadedAt: new Date("2026-03-13") },
  { id: "doc-4", name: "전략.pdf", size: 1843200, uploadedAt: new Date("2026-03-12") },
  { id: "doc-5", name: "법률검토.pdf", size: 921600, uploadedAt: new Date("2026-03-11") },
];

export const DUMMY_KNOWLEDGES: Knowledge[] = [
  { id: "kn-1", name: "마케팅 자료", description: "마케팅 전략 관련 문서 모음", documentIds: ["doc-1", "doc-2", "doc-4"] },
  { id: "kn-2", name: "기술 문서", description: "API 및 아키텍처 관련 자료", documentIds: ["doc-3", "doc-1"] },
  { id: "kn-3", name: "법률 검토", description: "계약서 및 규정 관련 문서", documentIds: ["doc-5"] },
];
