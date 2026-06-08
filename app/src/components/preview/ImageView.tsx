import { useRef } from 'react';
import { useZoom, useWheelZoom } from './useZoom';
import { ZoomControls } from './ZoomControls';

interface Props { objectUrl: string; alt: string }

export function ImageView({ objectUrl, alt }: Props) {
  const zoom = useZoom();
  const stageRef = useRef<HTMLDivElement>(null);
  useWheelZoom(stageRef, zoom);

  // At fit (scale 1) the image is contained within the stage (dark margins show,
  // so clicking them dismisses). Zoomed in, the image grows past the stage and
  // the stage scrolls to pan.
  const fit = zoom.scale === 1;

  return (
    <>
      <div className="cw-preview-stage" ref={stageRef} onDoubleClick={zoom.toggle}>
        <img
          className="cw-preview-content cw-preview-img"
          src={objectUrl}
          alt={alt}
          draggable={false}
          style={fit ? undefined : { width: `${zoom.scale * 100}%`, maxWidth: 'none', maxHeight: 'none' }}
        />
      </div>
      <ZoomControls
        scale={zoom.scale}
        onZoomIn={zoom.zoomIn}
        onZoomOut={zoom.zoomOut}
        onReset={zoom.reset}
        canZoomIn={zoom.canZoomIn}
        canZoomOut={zoom.canZoomOut}
      />
    </>
  );
}
