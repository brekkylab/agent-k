import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { useFilePreview } from './useFilePreview';
import { PreviewBody } from './PreviewBody';

interface Props {
  /** Global path of the file to preview, or null to show the empty hint. */
  globalPath: string | null;
  /** Hint shown when no file is selected. */
  emptyHint: string;
}

/**
 * Inline file preview (no modal chrome / portal). Reuses the same fetch + view
 * logic as FilePreviewModal via useFilePreview + PreviewBody, but renders into
 * a plain panel — e.g. the right pane of the home shared-file picker.
 */
export function FilePreviewPane({ globalPath, emptyHint }: Props) {
  if (!globalPath) {
    return (
      <div className="cw-preview-pane is-empty">
        <Icon name="file" size={28} />
        <p>{emptyHint}</p>
      </div>
    );
  }
  return <FilePreviewPaneInner key={globalPath} globalPath={globalPath} />;
}

function FilePreviewPaneInner({ globalPath }: { globalPath: string }) {
  const { t } = useTranslation('common');
  const { state, filename, download, isStage } = useFilePreview(globalPath);
  return (
    <div className="cw-preview-pane" data-mode={isStage ? 'stage' : 'sheet'}>
      <div className="cw-preview-pane-head">
        <span className="cw-preview-pane-title" title={filename}>{filename}</span>
        <button type="button" aria-label={t('preview.download')} title={t('preview.download')} onClick={download}>
          <Icon name="download" size={15} />
        </button>
      </div>
      <div className="cw-preview-pane-body">
        <PreviewBody state={state} filename={filename} onDownload={download} />
      </div>
    </div>
  );
}
