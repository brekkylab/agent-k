import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

interface Props {
  filename: string;
  status: 'uploading' | 'uploaded' | 'error';
  /** True for files referenced from the project's shared files (vs. uploaded
   *  via the clip). Shows a small marker next to the file-type icon. */
  shared?: boolean;
  error?: string;
  onRemove: () => void;
}

export function AttachmentChip({ filename, status, shared, error, onRemove }: Props) {
  const { t } = useTranslation('session');
  // The whole chip is click-to-remove (hover turns it red); no separate × button.
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
      {status === 'uploaded' && (
        <>
          {/* Source marker: cloud = referenced from shared files, clip = uploaded. */}
          <span className={`cw-attach-source cw-attach-source--${shared ? 'shared' : 'upload'}`}>
            <Icon name={shared ? 'cloud' : 'paperclip'} size={11} />
          </span>
          <FileTypeIcon filename={filename} size={14} />
        </>
      )}
      <span className="cw-attach-name">{filename}</span>
    </div>
  );
}
