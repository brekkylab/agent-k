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
  return (
    <div className="cw-attach-chip" title={error}>
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
      <button type="button" aria-label="remove" onClick={onRemove} className="cw-attach-remove">
        <Icon name="x" size={11} />
      </button>
    </div>
  );
}
