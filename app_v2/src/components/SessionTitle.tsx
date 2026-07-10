import { useEffect, useRef, useState } from 'react';

// Per-character reveal speed and the ceiling on how long the shimmer runs
// before giving up and showing the fallback (a recent session whose generation
// hangs must not shimmer forever).
const TYPE_MS = 32;
const SHIMMER_TIMEOUT_MS = 15_000;
// A title is generated on a new session's first run; only sessions created
// within this window are still plausibly "generating". Older untitled sessions
// (predating the feature, or a failed generation) show their fallback at once
// instead of a misleading shimmer.
const PENDING_WINDOW_MS = 2 * 60_000;

/** Whether an untitled session is plausibly still generating its title. */
export function isTitlePending(title: string | null, createdAt?: string): boolean {
  if (title != null) return false;
  if (createdAt == null) return true; // unknown yet — assume a title may be incoming
  const created = Date.parse(createdAt);
  if (Number.isNaN(created)) return true;
  return Date.now() - created < PENDING_WINDOW_MS;
}

interface SessionTitleProps {
  /** The session's title; `null` while it is still being generated. */
  title: string | null;
  /** Session creation time (RFC3339); gates whether a null title shimmers. */
  createdAt?: string;
  /** Class for the text span (e.g. `cw-chat-title`, `cw-session-title`). */
  className?: string;
  /** Extra class on the shimmer skeleton for context-specific sizing. */
  skeletonClassName?: string;
  /** Shown when the title is absent and not (or no longer) pending. */
  fallback?: string;
}

/**
 * Renders a session title with two flourishes:
 *   - a shimmer skeleton while the auto-title is being generated (`title` null),
 *   - a typewriter reveal when the title *first* arrives (a `null → value`
 *     transition observed while mounted — i.e. a fresh generation).
 *
 * A title that is already present at mount (revisiting an existing session) or
 * a viewer who prefers reduced motion gets the text instantly, with no typing.
 */
export function SessionTitle({
  title,
  createdAt,
  className,
  skeletonClassName,
  fallback,
}: SessionTitleProps) {
  const pending = isTitlePending(title, createdAt);
  const [shown, setShown] = useState(title ?? '');
  const [expired, setExpired] = useState(false);
  const prevTitle = useRef(title);

  useEffect(() => {
    const prev = prevTitle.current;
    prevTitle.current = title;

    if (title == null) {
      setShown('');
      setExpired(false);
      // Only arm the backstop while we believe a title is coming; a
      // non-pending (legacy) session renders its fallback immediately.
      if (!pending) return;
      const t = setTimeout(() => setExpired(true), SHIMMER_TIMEOUT_MS);
      return () => clearTimeout(t);
    }

    setExpired(false);
    const freshlyGenerated = prev == null; // null → value: the title just became known
    const reduceMotion =
      typeof window !== 'undefined' &&
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (!freshlyGenerated || reduceMotion) {
      setShown(title);
      return;
    }

    // Typewriter reveal.
    setShown('');
    let i = 0;
    const id = setInterval(() => {
      i += 1;
      setShown(title.slice(0, i));
      if (i >= title.length) clearInterval(id);
    }, TYPE_MS);
    return () => clearInterval(id);
  }, [title, pending]);

  if (title == null) {
    // Shimmer only while a title is plausibly incoming; otherwise (legacy
    // session, or generation gave up) show the fallback with no shimmer/swap.
    if (!pending || expired) {
      return <span className={className}>{fallback}</span>;
    }
    return (
      <span
        className={`cw-title-skeleton${skeletonClassName ? ` ${skeletonClassName}` : ''}`}
        role="status"
        aria-label="제목 생성 중"
      />
    );
  }

  const typing = shown.length < title.length;
  return (
    <span className={className} aria-label={title}>
      {shown}
      {typing && <span className="cw-title-caret" aria-hidden="true" />}
    </span>
  );
}
