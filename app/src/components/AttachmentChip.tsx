import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

interface Props {
  filename: string;
  // 'staged' = picked locally but not uploaded yet (home composer, uploaded on
  // send); rendered like 'uploaded'. 'uploading'/'uploaded'/'error' = live state
  // of a file already being/been uploaded to a session (session composer).
  status: 'staged' | 'uploading' | 'uploaded' | 'error';
  /** True for files referenced from the project's shared files (vs. uploaded
   *  via the clip). Shows a small marker next to the file-type icon. */
  shared?: boolean;
  error?: string;
  onRemove: () => void;
}

export function AttachmentChip({ filename, status, shared, error, onRemove }: Props) {
  const { t } = useTranslation('session');
  // The whole chip is click-to-remove. It shows a persistent × and reddens on hover so
  // the destructive action is unmistakable (per PR review — colour alone read as ambiguous).
  // The × is a visual hint, not a separate button; the chip itself handles the click.
  return (
    <div
      className="cw-attach-chip cw-attach-chip--removable"
      role="button"
      tabIndex={0}
      title={error ?? t('shared_files.remove')}
      aria-label={t('shared_files.remove')}
      onClick={onRemove}
      onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onRemove(); } }}
    >
      {status === 'uploading' && <span className="cw-attach-spinner">⏳</span>}
      {status === 'error' && <span className="cw-attach-error"><Icon name="x" size={11} /></span>}
      {(status === 'uploaded' || status === 'staged') && (
        <>
          {/* Source marker: cloud = referenced from shared files, clip = uploaded. */}
          <span className={`cw-attach-source cw-attach-source--${shared ? 'shared' : 'upload'}`}>
            <Icon name={shared ? 'cloud' : 'paperclip'} size={11} />
          </span>
          <FileTypeIcon filename={filename} size={14} />
        </>
      )}
      <span className="cw-attach-name">{filename}</span>
      {/* Persistent × hint — signals that clicking the chip removes it. */}
      <span className="cw-attach-remove-hint" aria-hidden="true"><Icon name="x" size={11} /></span>
    </div>
  );
}
