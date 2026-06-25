import { useTranslation } from 'react-i18next';
import { previewCodeLang } from '@/domain/files';
import type { PreviewState } from './useFilePreview';
import { FallbackCard } from './FallbackCard';
import { ImageView } from './ImageView';
import { HtmlView } from './HtmlView';
import { PdfView } from './PdfView';
import { MarkdownView } from './MarkdownView';
import { CodeView } from './CodeView';
import { TableView } from './TableView';
import { TextView } from './TextView';

interface Props {
  state: PreviewState;
  filename: string;
  onDownload: () => void;
}

/**
 * Renders a loaded preview (from useFilePreview) — the kind-specific view inside
 * the standard `.cw-preview-content` wrappers. Chrome (title bar, backdrop,
 * focus trap) is owned by the caller: FilePreviewModal wraps this in a portal;
 * FilePreviewPane wraps it in an inline panel.
 */
export function PreviewBody({ state, filename, onDownload }: Props) {
  const { t } = useTranslation('common');
  return (
    <>
      {state.status === 'loading' && <div className="cw-preview-loading">{t('preview.loading')}</div>}
      {state.status === 'fallback' && (
        <div className="cw-preview-content cw-preview-fallcard">
          <FallbackCard filename={filename} reason={state.reason} onDownload={onDownload} />
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
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--code"><CodeView content={state.content} lang={previewCodeLang(filename)} /></div>
      )}
      {state.status === 'text' && state.kind === 'table' && (
        <div className="cw-preview-content cw-preview-sheet cw-preview-sheet--wide">
          <TableView content={state.content} delimiter={filename.toLowerCase().endsWith('.tsv') ? '\t' : ''} />
        </div>
      )}
      {state.status === 'text' && state.kind === 'text' && (
        <div className="cw-preview-content cw-preview-sheet"><TextView content={state.content} /></div>
      )}
    </>
  );
}
