-- Per-project PDF-to-markdown engine for the knowledge corpus.
-- 'kreuzberg' (native, fast) | 'docling' (slower, better layout).
-- NULL/absent resolves to 'kreuzberg'.
ALTER TABLE projects ADD COLUMN pdf_engine TEXT;
