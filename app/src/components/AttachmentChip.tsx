import { Icon } from './Icon';

interface Props {
  filename: string;
  status: 'uploading' | 'uploaded' | 'error';
  error?: string;
  onRemove: () => void;
}

export function AttachmentChip({ filename, status, error, onRemove }: Props) {
  return (
    <div className="cw-attach-chip" title={error}>
      {status === 'uploading' && <span className="cw-attach-spinner">⏳</span>}
      {status === 'error' && <span className="cw-attach-error"><Icon name="x" size={11} /></span>}
      {status === 'uploaded' && <Icon name="file-text" size={13} />}
      <span className="cw-attach-name">{filename}</span>
      <button type="button" aria-label="remove" onClick={onRemove} className="cw-attach-remove">
        <Icon name="x" size={11} />
      </button>
    </div>
  );
}
