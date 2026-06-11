import { useCallback, useState } from 'react';

interface UseFileDropzoneOpts {
  /** Called with the dropped computer files (`dataTransfer.files`). */
  onFiles: (files: File[]) => void;
  /** When true, the dropzone ignores drags (no highlight, no drop). */
  disabled?: boolean;
}

/**
 * Drag-and-drop target for computer files. Returns `isOver` (for a highlight
 * class) and `dropProps` to spread onto the target element. Only reacts to
 * native file drags (`types` includes 'Files'), so it can coexist with other
 * drag payloads on the same element.
 */
export function useFileDropzone({ onFiles, disabled = false }: UseFileDropzoneOpts) {
  const [isOver, setIsOver] = useState(false);

  const onDragOver = useCallback((e: React.DragEvent) => {
    if (disabled || !e.dataTransfer.types.includes('Files')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    setIsOver(true);
  }, [disabled]);

  const onDragLeave = useCallback((e: React.DragEvent) => {
    // Ignore leaves into descendant elements — only clear when leaving the target.
    if (e.currentTarget.contains(e.relatedTarget as Node)) return;
    setIsOver(false);
  }, []);

  const onDrop = useCallback((e: React.DragEvent) => {
    if (disabled || !e.dataTransfer.types.includes('Files')) return;
    e.preventDefault();
    setIsOver(false);
    onFiles(Array.from(e.dataTransfer.files ?? []));
  }, [disabled, onFiles]);

  return { isOver, dropProps: { onDragOver, onDragLeave, onDrop } };
}
