import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { FileTypeIcon } from '../FileTypeIcon';

export type FallbackReason = 'unsupported' | 'too-large' | 'error';

interface Props {
  filename: string;
  reason: FallbackReason;
  onDownload: () => void;
}

export function FallbackCard({ filename, reason, onDownload }: Props) {
  const { t } = useTranslation('common');
  const title =
    reason === 'too-large' ? t('preview.too_large_title')
    : reason === 'error' ? t('preview.error_title')
    : t('preview.unsupported_title');
  const body =
    reason === 'too-large' ? t('preview.too_large_body')
    : reason === 'error' ? t('preview.error_body')
    : t('preview.unsupported_body');

  return (
    <div className="cw-preview-fallback">
      <FileTypeIcon filename={filename} size={48} />
      <div className="cw-preview-fallback-name">{filename}</div>
      <div className="cw-preview-fallback-title">{title}</div>
      <div className="cw-preview-fallback-body">{body}</div>
      <button type="button" className="cw-btn-primary" onClick={onDownload}>
        <Icon name="download" size={14} /> {t('preview.download')}
      </button>
    </div>
  );
}
