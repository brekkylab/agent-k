import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { useFilePreview } from './useFilePreview';
import { PreviewBody } from './PreviewBody';

interface Props {
  /** Global path of the file to preview, or null to show the empty hint. */
  globalPath: string | null;
  /** Hint shown when no file is selected. */
  emptyHint: string;
  /** When set, the header shows a "+" that attaches this file (or a check if already staged). */
  onAttach?: () => void;
  added?: boolean;
}

/**
 * Inline file preview (no modal chrome / portal). Reuses the same fetch + view
 * logic as FilePreviewModal via useFilePreview + PreviewBody, but renders into
 * a plain panel — e.g. the right pane of the home shared-file picker.
 */
export function FilePreviewPane({ globalPath, emptyHint, onAttach, added }: Props) {
  if (!globalPath) {
    return (
      <div className="cw-preview-pane is-empty">
        <Icon name="file" size={28} />
        <p>{emptyHint}</p>
      </div>
    );
  }
  return <FilePreviewPaneInner key={globalPath} globalPath={globalPath} onAttach={onAttach} added={added} />;
}

function FilePreviewPaneInner({ globalPath, onAttach, added }: { globalPath: string; onAttach?: () => void; added?: boolean }) {
  const { t } = useTranslation('common');
  const { t: tSession } = useTranslation('session');
  const { state, filename, download, isStage } = useFilePreview(globalPath);
  return (
    <div className="cw-preview-pane" data-mode={isStage ? 'stage' : 'sheet'}>
      <div className="cw-preview-pane-head">
        <span className="cw-preview-pane-title" title={filename}>{filename}</span>
        {onAttach && (added ? (
          <span className="cw-preview-pane-added" aria-label={tSession('shared_files.added')} title={tSession('shared_files.added')}>
            <Icon name="check" size={15} />
          </span>
        ) : (
          <button type="button" aria-label={tSession('shared_files.import')} title={tSession('shared_files.import')} onClick={onAttach}>
            <Icon name="plus" size={15} />
          </button>
        ))}
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
