import { useCallback, useEffect, useRef, useState } from 'react';
import { useZoom, useWheelZoom } from './useZoom';
import { useDragPan } from './usePan';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string; alt: string }

export function ImageView({ objectUrl, alt }: Props) {
  const zoom = useZoom();
  const stageRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  // Plain wheel zooms (no modifier needed) since panning is by drag, not scroll.
  useWheelZoom(stageRef, zoom, { plain: true });
  const { pannable, dragging } = useDragPan(stageRef, imgRef);

  // `fitWidth` is the px width at which the image is fully contained in the
  // stage without upscaling past its natural size (the 100% baseline). Every
  // zoom level renders at `fitWidth * scale`, so the % label maps linearly to
  // on-screen size for portrait and landscape alike. Computing it from the
  // measured natural + stage dimensions replaces the old dual-model approach
  // (object-fit:contain at scale 1, width-percent otherwise) whose two
  // baselines disagreed — making "100%" jump to a thumbnail for tall images.
  const [fitWidth, setFitWidth] = useState<number | null>(null);

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
    setFitWidth(img.naturalWidth * fitScale);
  }, []);

  // Re-measure when the stage resizes (window resize); the image's natural size
  // is stable so it's read fresh inside `measure` each time.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const ro = new ResizeObserver(measure);
    ro.observe(stage);
    return () => ro.disconnect();
  }, [measure]);

  const width = fitWidth != null ? fitWidth * zoom.scale : null;

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
