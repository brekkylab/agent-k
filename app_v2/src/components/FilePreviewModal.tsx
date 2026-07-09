import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { getFileBlob } from '@/api/workspace';
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

// Render cost caps: media kinds decode off the main thread; text-family kinds
// decode + syntax-highlight synchronously, so they get a tighter cap.
const MAX_PREVIEW_BYTES = 50 * 1024 * 1024;
const MAX_TEXT_BYTES = 5 * 1024 * 1024;

interface Props {
  /** WebDAV path within the project workspace, e.g. "/report.pdf" */
  path: string;
  /** Display filename (derived from path by the caller). */
  name: string;
  onClose(): void;
}

type Loaded =
  | { status: 'loading' }
  | { status: 'fallback'; reason: FallbackReason }
  | { status: 'media'; objectUrl: string; kind: 'image' | 'html' | 'pdf' }
  | { status: 'text'; content: string; kind: 'markdown' | 'code' | 'table' | 'text' };

const MEDIA_KINDS: PreviewKind[] = ['image', 'html', 'pdf'];
const TEXT_KINDS: PreviewKind[] = ['markdown', 'code', 'table', 'text'];

export function FilePreviewModal({ path, name, onClose }: Props) {
  const { t } = useTranslation('common');
  const kind = resolvePreviewKind(name);
  const [state, setState] = useState<Loaded>({ status: 'loading' });

  useDialogEscape(onClose);

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
    const cap = TEXT_KINDS.includes(kind) ? MAX_TEXT_BYTES : MAX_PREVIEW_BYTES;
    void (async () => {
      try {
        const blob = await getFileBlob(path);
        if (cancelled) return;
        if (blob.size > cap) {
          setState({ status: 'fallback', reason: 'too-large' });
          return;
        }
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
  }, [path, kind]);

  function handleDownload() {
    // Create blob URL, trigger download via temporary anchor, then revoke.
    if (state.status === 'media') {
      const a = document.createElement('a');
      a.href = state.objectUrl;
      a.download = name;
      a.click();
      return;
    }
    // For text states or loading, re-fetch and trigger download.
    void getFileBlob(path).then((blob) => {
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = name;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10000);
    });
  }

  function maybeClose(e: React.MouseEvent) {
    if (!(e.target as HTMLElement).closest('.cw-preview-content, .cw-preview-chrome, .cw-zoom-controls')) {
      onClose();
    }
  }

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
      onClick={(e) => e.stopPropagation()}
    >
      <div className="cw-preview-chrome">
        <span id={titleId} className="cw-preview-title" title={name}>{name}</span>
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
          <FallbackCard filename={name} reason={state.reason} onDownload={handleDownload} />
        </div>
      )}
      {state.status === 'media' && state.kind === 'image' && <ImageView objectUrl={state.objectUrl} alt={name} />}
      {state.status === 'media' && state.kind === 'pdf' && <PdfView objectUrl={state.objectUrl} />}
      {state.status === 'media' && state.kind === 'html' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--frame"><HtmlView objectUrl={state.objectUrl} title={name} /></div>
      )}
      {state.status === 'text' && state.kind === 'markdown' && (
        <div className="cw-preview-content cw-preview-sheet"><MarkdownView content={state.content} /></div>
      )}
      {state.status === 'text' && state.kind === 'code' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--code"><CodeView content={state.content} lang={previewCodeLang(name)} /></div>
      )}
      {state.status === 'text' && state.kind === 'table' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--wide">
          <TableView content={state.content} delimiter={name.toLowerCase().endsWith('.tsv') ? '\t' : ''} />
        </div>
      )}
      {state.status === 'text' && state.kind === 'text' && (
        <div className="cw-preview-content cw-preview-sheet"><TextView content={state.content} /></div>
      )}
    </div>,
    document.body,
  );
}
