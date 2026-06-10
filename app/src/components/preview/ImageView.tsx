import { useCallback, useEffect, useRef, useState } from 'react';
import { useZoom, useWheelZoom } from './useZoom';
import { useDragPan } from './usePan';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string; alt: string }

/** The fit baseline measured on load/resize: the px width at which the image is
 *  fully contained in the stage (never upscaled past natural), plus the
 *  available stage box and the image's height/width ratio — enough to derive
 *  both the rendered size and whether it overflows at any zoom. */
interface Fit { width: number; availW: number; availH: number; ratio: number }

export function ImageView({ objectUrl, alt }: Props) {
  const zoom = useZoom();
  const stageRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  // Plain wheel zooms (no modifier needed) since panning is by drag, not scroll.
  useWheelZoom(stageRef, zoom, { plain: true });

  // `fit.width * scale` is the rendered px width — every zoom level is a clean
  // multiple of one fit baseline, so the % label maps linearly to on-screen
  // size for portrait and landscape alike. (Replaces an old dual model whose
  // two baselines disagreed, making "100%" a thumbnail for tall images.)
  const [fit, setFit] = useState<Fit | null>(null);

  const measure = useCallback(() => {
    const stage = stageRef.current;
    const img = imgRef.current;
    if (!stage || !img || !img.naturalWidth || !img.naturalHeight) return;
    const cs = getComputedStyle(stage);
    const padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
    const padY = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
    const availW = stage.clientWidth - padX;
    const availH = stage.clientHeight - padY;
    // never upscale past natural — mirrors the old `max-width/height: 100%`.
    const fitScale = Math.min(availW / img.naturalWidth, availH / img.naturalHeight, 1);
    setFit({ width: img.naturalWidth * fitScale, availW, availH, ratio: img.naturalHeight / img.naturalWidth });
  }, []);

  // Re-measure when the stage resizes (window resize); the image's natural size
  // is stable so it's read fresh inside `measure` each time. Zoom changes don't
  // re-measure — `pannable`/`width` derive from `fit` + scale on render.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const ro = new ResizeObserver(measure);
    ro.observe(stage);
    return () => ro.disconnect();
  }, [measure]);

  const width = fit ? fit.width * zoom.scale : null;
  // The rendered image overflows the stage box (so a drag can pan it). Derived
  // arithmetically from the already-measured fit — equivalent to comparing the
  // stage's scrollWidth/Height against its clientWidth/Height, without a second
  // ResizeObserver or any extra layout reads.
  const pannable =
    width != null && fit != null && (width > fit.availW + 1 || width * fit.ratio > fit.availH + 1);

  const { dragging } = useDragPan(stageRef, imgRef, pannable);

  return (
    <>
      <div
        className={`cw-preview-stage cw-preview-img-stage${dragging ? ' is-panning' : ''}`}
        ref={stageRef}
        onDoubleClick={zoom.toggle}
      >
        <img
          ref={imgRef}
          className={`cw-preview-content cw-preview-img${pannable ? ' is-pannable' : ''}`}
          src={objectUrl}
          alt={alt}
          draggable={false}
          onLoad={measure}
          style={width != null ? { width: `${width}px`, maxWidth: 'none', maxHeight: 'none' } : undefined}
        />
      </div>
      <ZoomControls
        scale={zoom.scale}
        onZoomIn={zoom.zoomIn}
        onZoomOut={zoom.zoomOut}
        onReset={zoom.reset}
        canZoomIn={zoom.canZoomIn}
        canZoomOut={zoom.canZoomOut}
        resetAsIcon
      />
    </>
  );
}
