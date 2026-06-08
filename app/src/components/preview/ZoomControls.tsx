import { useTranslation } from 'react-i18next';

interface Props {
  scale: number;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onReset: () => void;
  canZoomIn: boolean;
  canZoomOut: boolean;
}

/** Floating zoom pill for the image/pdf stage. Lives over the dark backdrop. */
export function ZoomControls({ scale, onZoomIn, onZoomOut, onReset, canZoomIn, canZoomOut }: Props) {
  const { t } = useTranslation('common');
  return (
    <div className="cw-zoom-controls" role="group" aria-label={t('preview.zoom')}>
      <button type="button" onClick={onZoomOut} disabled={!canZoomOut} aria-label={t('preview.zoom_out')}>−</button>
      <button type="button" className="cw-zoom-pct" onClick={onReset} aria-label={t('preview.zoom_reset')}>
        {Math.round(scale * 100)}%
      </button>
      <button type="button" onClick={onZoomIn} disabled={!canZoomIn} aria-label={t('preview.zoom_in')}>+</button>
    </div>
  );
}
