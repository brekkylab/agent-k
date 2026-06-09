import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

interface Props {
  scale: number;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onReset: () => void;
  canZoomIn: boolean;
  canZoomOut: boolean;
  /** Image preview shows a magnifier "reset to fit" glyph instead of the live %
   *  (the number is noise once wheel/drag drive zoom); PDF keeps the readable %. */
  resetAsIcon?: boolean;
}

/** Floating zoom pill for the image/pdf stage. Lives over the dark backdrop. */
export function ZoomControls({ scale, onZoomIn, onZoomOut, onReset, canZoomIn, canZoomOut, resetAsIcon = false }: Props) {
  const { t } = useTranslation('common');
  return (
    <div className="cw-zoom-controls" role="group" aria-label={t('preview.zoom')}>
      <button type="button" onClick={onZoomOut} disabled={!canZoomOut} aria-label={t('preview.zoom_out')}>−</button>
      <button
        type="button"
        className={`cw-zoom-pct${resetAsIcon ? ' cw-zoom-pct--icon' : ''}`}
        onClick={onReset}
        aria-label={t('preview.zoom_reset')}
        title={`${Math.round(scale * 100)}%`}
      >
        {resetAsIcon ? <Icon name="zoom-out" size={16} /> : `${Math.round(scale * 100)}%`}
      </button>
      <button type="button" onClick={onZoomIn} disabled={!canZoomIn} aria-label={t('preview.zoom_in')}>+</button>
    </div>
  );
}
