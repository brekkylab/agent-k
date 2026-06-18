import { useCallback, useState } from 'react';

interface UseFileDropzoneOpts {
  /** Called with the dropped computer files (`dataTransfer.files`). */
  onFiles: (files: File[]) => void;
  /** When true, the dropzone ignores drags (no highlight, no drop). */
  disabled?: boolean;
  /** Extra dataTransfer types (besides native 'Files') that also activate the
   *  zone — e.g. an app-internal MIME for dragging references in. Pass a stable
   *  reference (module constant) to keep the handlers memoized. */
  acceptTypes?: string[];
  /** Drop handler for an accepted *non-file* payload (matched `acceptTypes`).
   *  Receives the raw event so the caller can read its `dataTransfer`. */
  onAcceptedDrop?: (e: React.DragEvent) => void;
}

/**
 * Drag-and-drop target for computer files — and, optionally, app-internal drag
 * payloads via `acceptTypes`. Returns `isOver` (for a highlight class) and
 * `dropProps` to spread onto the target element. By default only reacts to
 * native file drags (`types` includes 'Files'), so it can coexist with other
 * drag payloads on the same element.
 */
export function useFileDropzone({ onFiles, disabled = false, acceptTypes, onAcceptedDrop }: UseFileDropzoneOpts) {
  const [isOver, setIsOver] = useState(false);

  const accepts = useCallback(
    (types: readonly string[]) =>
      types.includes('Files') || (acceptTypes?.some((t) => types.includes(t)) ?? false),
    [acceptTypes],
  );

  const onDragOver = useCallback((e: React.DragEvent) => {
    if (disabled || !accepts(e.dataTransfer.types)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    setIsOver(true);
  }, [disabled, accepts]);

  const onDragLeave = useCallback((e: React.DragEvent) => {
    // Ignore leaves into descendant elements — only clear when leaving the target.
    if (e.currentTarget.contains(e.relatedTarget as Node)) return;
    setIsOver(false);
  }, []);

  const onDrop = useCallback((e: React.DragEvent) => {
    if (disabled || !accepts(e.dataTransfer.types)) return;
    e.preventDefault();
    setIsOver(false);
    const files = Array.from(e.dataTransfer.files ?? []);
    if (files.length > 0) onFiles(files);
    else onAcceptedDrop?.(e);
  }, [disabled, accepts, onFiles, onAcceptedDrop]);

  return { isOver, dropProps: { onDragOver, onDragLeave, onDrop } };
}
