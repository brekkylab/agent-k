import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { fetchFileForPreview, downloadFileByGlobalPath } from '@/api/dirents';
import { resolvePreviewKind, previewCodeLang, type PreviewKind } from '@/domain/files';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { Icon } from './Icon';
import { FallbackCard, type FallbackReason } from './preview/FallbackCard';
import { ImageView } from './preview/ImageView';
import { HtmlView } from './preview/HtmlView';
import { PdfView } from './preview/PdfView';
import { MarkdownView } from './preview/MarkdownView';
import { CodeView } from './preview/CodeView';
import { TableView } from './preview/TableView';
import { TextView } from './preview/TextView';

const MAX_PREVIEW_BYTES = 20 * 1024 * 1024;
// Decoding text on the main thread blocks the UI, so cap text-family previews
// (markdown/code/text) well below the media cap.
const MAX_TEXT_BYTES = 2 * 1024 * 1024;

interface Props {
  globalPath: string;
  onClose: () => void;
}

type Loaded =
  | { status: 'loading' }
  | { status: 'fallback'; reason: FallbackReason }
  | { status: 'media'; objectUrl: string; kind: 'image' | 'html' | 'pdf' }
  | { status: 'text'; content: string; kind: 'markdown' | 'code' | 'table' | 'text' };

const MEDIA_KINDS: PreviewKind[] = ['image', 'html', 'pdf'];
const TEXT_KINDS: PreviewKind[] = ['markdown', 'code', 'table', 'text'];

export function FilePreviewModal({ globalPath, onClose }: Props) {
  const { t } = useTranslation('common');
  const filename = globalPath.split('/').pop() ?? globalPath;
  const kind = resolvePreviewKind(filename);
  const [state, setState] = useState<Loaded>({ status: 'loading' });

  useDialogEscape(onClose);

  // Initial focus + restore on close (mirrors SessionsOverlay/WebhookTokenDialog),
  // a minimal Tab focus trap, and background scroll lock for the large modal.
  const modalRef = useRef<HTMLDivElement>(null);
  const closeBtnRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const prevActive = document.activeElement as HTMLElement | null;
    closeBtnRef.current?.focus();
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = prevOverflow;
      prevActive?.focus?.();
    };
  }, []);

  function onTrapKeyDown(e: React.KeyboardEvent) {
    if (e.key !== 'Tab') return;
    const root = modalRef.current;
    if (!root) return;
    // Deliberately excludes `iframe`: focusing the sandboxed HTML-preview frame
    // would swallow keydowns (ESC/Tab) inside its browsing context, breaking the
    // dialog's keyboard handling. The frame stays scrollable via mouse.
    const focusables = root.querySelectorAll<HTMLElement>(
      'button, [href], input, [tabindex]:not([tabindex="-1"])',
    );
    if (focusables.length === 0) return;
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }

  useEffect(() => {
    let cancelled = false;
    let createdUrl: string | null = null;

    if (kind === 'unsupported') {
      setState({ status: 'fallback', reason: 'unsupported' });
      return;
    }

    setState({ status: 'loading' });
    // Text decodes on the main thread, so cap it lower than the media cap. The
    // cap is chosen before the fetch so oversized files are rejected from the
    // Content-Length header without downloading the body.
    const cap = TEXT_KINDS.includes(kind) ? MAX_TEXT_BYTES : MAX_PREVIEW_BYTES;
    void (async () => {
      try {
        const result = await fetchFileForPreview(globalPath, cap);
        if (cancelled) return;
        if (result.tooLarge) {
          setState({ status: 'fallback', reason: 'too-large' });
          return;
        }
        const { blob } = result;
        if (MEDIA_KINDS.includes(kind)) {
          createdUrl = URL.createObjectURL(blob);
          setState({ status: 'media', objectUrl: createdUrl, kind: kind as 'image' | 'html' | 'pdf' });
        } else if (TEXT_KINDS.includes(kind)) {
          const content = await blob.text();
          if (cancelled) return;
          setState({ status: 'text', content, kind: kind as 'markdown' | 'code' | 'table' | 'text' });
        }
      } catch {
        if (!cancelled) setState({ status: 'fallback', reason: 'error' });
      }
    })();

    return () => {
      cancelled = true;
      if (createdUrl) URL.revokeObjectURL(createdUrl);
    };
  }, [globalPath, kind]);

  function handleDownload() {
    void downloadFileByGlobalPath(globalPath);
  }

  // Click anywhere that isn't the content/chrome/zoom-pill (i.e. the dimmed
  // area) dismisses — works for both the media stage's margins and the sheet's
  // surrounding dim, regardless of nesting depth.
  function maybeClose(e: React.MouseEvent) {
    if (!(e.target as HTMLElement).closest('.cw-preview-content, .cw-preview-chrome, .cw-zoom-controls')) {
      onClose();
    }
  }

  // image/pdf render full-bleed on the dark "stage" with zoom; everything else
  // (html/markdown/code/text/fallback) shows on a readable light "sheet".
  const isStage = state.status === 'media' && (state.kind === 'image' || state.kind === 'pdf');

  const titleId = useId();
  return createPortal(
    <div
      className="cw-preview-backdrop"
      data-mode={isStage ? 'stage' : 'sheet'}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      ref={modalRef}
      onKeyDown={onTrapKeyDown}
      onMouseDown={maybeClose}
      // Stop clicks bubbling through the React portal tree to an ancestor (e.g.
      // the AttachmentPreview chip's onClick toggle) when mounted under one.
      onClick={(e) => e.stopPropagation()}
    >
      <div className="cw-preview-chrome">
        <span id={titleId} className="cw-preview-title" title={filename}>{filename}</span>
        <div className="cw-preview-actions">
          <button type="button" aria-label={t('preview.download')} onClick={handleDownload}>
            <Icon name="download" size={16} />
          </button>
          <button type="button" ref={closeBtnRef} aria-label={t('preview.close')} onClick={onClose}>
            <Icon name="x" size={18} />
          </button>
        </div>
      </div>

      {state.status === 'loading' && <div className="cw-preview-loading">{t('preview.loading')}</div>}
      {state.status === 'fallback' && (
        <div className="cw-preview-content cw-preview-fallcard">
          <FallbackCard filename={filename} reason={state.reason} onDownload={handleDownload} />
        </div>
      )}
      {state.status === 'media' && state.kind === 'image' && <ImageView objectUrl={state.objectUrl} alt={filename} />}
      {state.status === 'media' && state.kind === 'pdf' && <PdfView objectUrl={state.objectUrl} />}
      {state.status === 'media' && state.kind === 'html' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--frame"><HtmlView objectUrl={state.objectUrl} title={filename} /></div>
      )}
      {state.status === 'text' && state.kind === 'markdown' && (
        <div className="cw-preview-content cw-preview-sheet"><MarkdownView content={state.content} /></div>
      )}
      {state.status === 'text' && state.kind === 'code' && (
        <div className="cw-preview-content cw-preview-sheet"><CodeView content={state.content} lang={previewCodeLang(filename)} /></div>
      )}
      {state.status === 'text' && state.kind === 'table' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--wide">
          <TableView content={state.content} delimiter={filename.toLowerCase().endsWith('.tsv') ? '\t' : ''} />
        </div>
      )}
      {state.status === 'text' && state.kind === 'text' && (
        <div className="cw-preview-content cw-preview-sheet"><TextView content={state.content} /></div>
      )}
    </div>,
    document.body,
  );
}
