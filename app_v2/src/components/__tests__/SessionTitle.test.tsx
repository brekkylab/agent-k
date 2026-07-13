import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { SessionTitle, isTitlePending } from '@/components/SessionTitle';

const recentIso = () => new Date(Date.now() - 1_000).toISOString();
const oldIso = () => new Date(Date.now() - 10 * 60_000).toISOString();

describe('isTitlePending', () => {
  it('is false once a title exists', () => {
    expect(isTitlePending('Some title', recentIso())).toBe(false);
  });

  it('is true for a null title with no createdAt (unknown yet)', () => {
    expect(isTitlePending(null, undefined)).toBe(true);
  });

  it('is true for a null title on a recently created session', () => {
    expect(isTitlePending(null, recentIso())).toBe(true);
  });

  it('is false for a null title on an old session (legacy / gave up)', () => {
    expect(isTitlePending(null, oldIso())).toBe(false);
  });

  it('is true for an unparseable createdAt', () => {
    expect(isTitlePending(null, 'not-a-date')).toBe(true);
  });
});

describe('SessionTitle rendering', () => {
  it('shows the title text when present', () => {
    render(<SessionTitle title="Walking benefits" createdAt={recentIso()} fallback="abcd1234" />);
    expect(screen.getByText('Walking benefits')).toBeTruthy();
  });

  it('shows a shimmer skeleton while a recent session is still generating', () => {
    render(<SessionTitle title={null} createdAt={recentIso()} fallback="abcd1234" />);
    // The skeleton carries role="status"; no fallback text is shown.
    expect(screen.getByRole('status')).toBeTruthy();
    expect(screen.queryByText('abcd1234')).toBeNull();
  });

  it('shows the fallback (not a skeleton) for an old untitled session', () => {
    render(<SessionTitle title={null} createdAt={oldIso()} fallback="abcd1234" />);
    expect(screen.getByText('abcd1234')).toBeTruthy();
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('shows an already-present title instantly (no typewriter on mount)', () => {
    render(<SessionTitle title="Instant title" createdAt={oldIso()} fallback="abcd1234" />);
    // Full text is present on the first paint — not revealed one char at a time.
    expect(screen.getByText('Instant title')).toBeTruthy();
  });
});

describe('SessionTitle typewriter on first arrival', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('reveals the title progressively when it transitions from null to a value', () => {
    const createdAt = recentIso();
    const { rerender } = render(
      <SessionTitle title={null} createdAt={createdAt} fallback="abcd1234" />,
    );
    // Generating → skeleton, no final text yet.
    expect(screen.getByRole('status')).toBeTruthy();

    // Title arrives: null → value transition triggers the typewriter.
    act(() => {
      rerender(<SessionTitle title="Hello" createdAt={createdAt} fallback="abcd1234" />);
    });
    // Mid-reveal: not yet the full string.
    act(() => {
      vi.advanceTimersByTime(32);
    });
    expect(screen.queryByText('Hello')).toBeNull();

    // After enough ticks for every character, the full title is shown.
    act(() => {
      vi.advanceTimersByTime(32 * 'Hello'.length);
    });
    expect(screen.getByText('Hello')).toBeTruthy();
  });
});
