import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { fetchFileBlob, downloadFileByGlobalPath } from '@/api/dirents';
import { resolvePreviewKind, previewCodeLang, type PreviewKind } from '@/domain/files';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { Icon } from './Icon';
import { FallbackCard, type FallbackReason } from './preview/FallbackCard';
import { ImageView } from './preview/ImageView';
import { HtmlView } from './preview/HtmlView';
import { PdfView } from './preview/PdfView';
import { MarkdownView } from './preview/MarkdownView';
import { CodeView } from './preview/CodeView';
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
  | { status: 'text'; content: string; kind: 'markdown' | 'code' | 'text' };

const MEDIA_KINDS: PreviewKind[] = ['image', 'html', 'pdf'];
const TEXT_KINDS: PreviewKind[] = ['markdown', 'code', 'text'];

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
    const focusables = root.querySelectorAll<HTMLElement>(
      'button, [href], iframe, input, [tabindex]:not([tabindex="-1"])',
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
    void (async () => {
      try {
        const blob = await fetchFileBlob(globalPath);
        if (cancelled) return;
        if (blob.size > MAX_PREVIEW_BYTES) {
          setState({ status: 'fallback', reason: 'too-large' });
          return;
        }
        if (MEDIA_KINDS.includes(kind)) {
          createdUrl = URL.createObjectURL(blob);
          setState({ status: 'media', objectUrl: createdUrl, kind: kind as 'image' | 'html' | 'pdf' });
        } else if (TEXT_KINDS.includes(kind)) {
          if (blob.size > MAX_TEXT_BYTES) {
            setState({ status: 'fallback', reason: 'too-large' });
            return;
          }
          const content = await blob.text();
          if (cancelled) return;
          setState({ status: 'text', content, kind: kind as 'markdown' | 'code' | 'text' });
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

  const titleId = useId();
  return createPortal(
    <div
      className="cw-dialog-backdrop"
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        className="cw-preview-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        ref={modalRef}
        onKeyDown={onTrapKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cw-preview-head">
          <span id={titleId} className="cw-preview-title" title={filename}>{filename}</span>
          <div className="cw-preview-actions">
            <button type="button" aria-label={t('preview.download')} onClick={handleDownload}>
              <Icon name="download" size={15} />
            </button>
            <button type="button" ref={closeBtnRef} aria-label={t('preview.close')} onClick={onClose}>
              <Icon name="x" size={16} />
            </button>
          </div>
        </header>
        <div className="cw-preview-body">
          {state.status === 'loading' && <div className="cw-preview-loading">{t('preview.loading')}</div>}
          {state.status === 'fallback' && (
            <FallbackCard filename={filename} reason={state.reason} onDownload={handleDownload} />
          )}
          {state.status === 'media' && state.kind === 'image' && <ImageView objectUrl={state.objectUrl} alt={filename} />}
          {state.status === 'media' && state.kind === 'html' && <HtmlView objectUrl={state.objectUrl} title={filename} />}
          {state.status === 'media' && state.kind === 'pdf' && <PdfView objectUrl={state.objectUrl} />}
          {state.status === 'text' && state.kind === 'markdown' && <MarkdownView content={state.content} />}
          {state.status === 'text' && state.kind === 'code' && <CodeView content={state.content} lang={previewCodeLang(filename)} />}
          {state.status === 'text' && state.kind === 'text' && <TextView content={state.content} />}
        </div>
      </div>
    </div>,
    document.body,
  );
}
