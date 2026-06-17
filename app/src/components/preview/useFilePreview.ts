import { useEffect, useState } from 'react';
import { fetchFileForPreview, downloadFileByGlobalPath } from '@/api/dirents';
import { resolvePreviewKind, type PreviewKind } from '@/domain/files';
import type { FallbackReason } from '@/components/preview/FallbackCard';

// These caps bound client-side RENDER cost, not storage: we fetch the original
// into the browser (unlike Drive/Dropbox/Slack, which preview server-made
// derivatives). Image/PDF/HTML decode off the main thread (objectURL →
// browser / pdf.js / sandboxed iframe), so they get headroom; text-family
// previews decode + syntax-highlight synchronously on the main thread, so they
// stay tighter. (PDF Range streaming would lift its cap entirely — separate task.)
const MAX_PREVIEW_BYTES = 50 * 1024 * 1024; // image / html / pdf
const MAX_TEXT_BYTES = 5 * 1024 * 1024; // markdown / code / table / text

export type PreviewState =
  | { status: 'loading' }
  | { status: 'fallback'; reason: FallbackReason }
  | { status: 'media'; objectUrl: string; kind: 'image' | 'html' | 'pdf' }
  | { status: 'text'; content: string; kind: 'markdown' | 'code' | 'table' | 'text' };

const MEDIA_KINDS: PreviewKind[] = ['image', 'html', 'pdf'];
const TEXT_KINDS: PreviewKind[] = ['markdown', 'code', 'table', 'text'];

/**
 * Fetches a file by its global path and decides how to render it. Shared by the
 * full-screen FilePreviewModal and the inline FilePreviewPane (e.g. the home
 * shared-file picker's right pane) so both decode/cap identically.
 */
export function useFilePreview(globalPath: string) {
  const filename = globalPath.split('/').pop() ?? globalPath;
  const kind = resolvePreviewKind(filename);
  const [state, setState] = useState<PreviewState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    let createdUrl: string | null = null;

    if (kind === 'unsupported') {
      setState({ status: 'fallback', reason: 'unsupported' });
      return;
    }

    setState({ status: 'loading' });
    // Text decodes on the main thread, so cap it lower than the media cap. The
    // cap is chosen before the fetch so oversized files are rejected from the
    // Content-Length header without downloading the body.
    const cap = TEXT_KINDS.includes(kind) ? MAX_TEXT_BYTES : MAX_PREVIEW_BYTES;
    void (async () => {
      try {
        const result = await fetchFileForPreview(globalPath, cap);
        if (cancelled) return;
        if (result.tooLarge) {
          setState({ status: 'fallback', reason: 'too-large' });
          return;
        }
        const { blob } = result;
        if (MEDIA_KINDS.includes(kind)) {
          createdUrl = URL.createObjectURL(blob);
          setState({ status: 'media', objectUrl: createdUrl, kind: kind as 'image' | 'html' | 'pdf' });
        } else if (TEXT_KINDS.includes(kind)) {
          const content = await blob.text();
          if (cancelled) return;
          setState({ status: 'text', content, kind: kind as 'markdown' | 'code' | 'table' | 'text' });
        }
      } catch {
        if (!cancelled) setState({ status: 'fallback', reason: 'error' });
      }
    })();

    return () => {
      cancelled = true;
      if (createdUrl) URL.revokeObjectURL(createdUrl);
    };
  }, [globalPath, kind]);

  const download = () => { void downloadFileByGlobalPath(globalPath); };

  // image/pdf render full-bleed on the dark "stage" with zoom; everything else
  // (html/markdown/code/text/fallback) shows on a readable light "sheet".
  const isStage = state.status === 'media' && (state.kind === 'image' || state.kind === 'pdf');

  return { state, kind, filename, download, isStage };
}
