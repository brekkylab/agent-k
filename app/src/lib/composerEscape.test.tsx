/**
 * @vitest-environment jsdom
 *
 * Escape-to-stop is carried by a window listener, not the textarea's onKeyDown:
 * the composer is `disabled` while streaming, and a real browser fires no key
 * events on a disabled control. (jsdom does NOT model that — it dispatches to a
 * disabled element's handlers anyway — so the root cause is verified manually in
 * the browser, not here.) These tests lock in the listener's gate:
 * streaming && ownedRunId && !stopping, IME-guarded, cleaned up on unmount.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { useEffect } from 'react';

afterEach(cleanup);

// The exact effect used by the session page, isolated for testing.
function StopOnEscape({
  streaming,
  ownedRunId,
  stopping,
  onStop,
}: {
  streaming: boolean;
  ownedRunId: string | null;
  stopping: boolean;
  onStop: () => void;
}) {
  useEffect(() => {
    if (!streaming || !ownedRunId || stopping) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.isComposing) {
        e.preventDefault();
        onStop();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [streaming, ownedRunId, stopping, onStop]);

  return <textarea data-testid="composer" disabled={streaming} />;
}

describe('Escape-to-stop wiring', () => {
  it('window Escape stops when streaming, run owned, not already stopping', () => {
    const onStop = vi.fn();
    render(<StopOnEscape streaming ownedRunId="run-1" stopping={false} onStop={onStop} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('window Escape is a no-op when not streaming', () => {
    const onStop = vi.fn();
    render(<StopOnEscape streaming={false} ownedRunId="run-1" stopping={false} onStop={onStop} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onStop).not.toHaveBeenCalled();
  });

  it('window Escape is a no-op when the run is not owned by this tab', () => {
    const onStop = vi.fn();
    render(<StopOnEscape streaming ownedRunId={null} stopping={false} onStop={onStop} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onStop).not.toHaveBeenCalled();
  });

  it('window Escape is a no-op when a stop is already in flight', () => {
    const onStop = vi.fn();
    render(<StopOnEscape streaming ownedRunId="run-1" stopping onStop={onStop} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onStop).not.toHaveBeenCalled();
  });

  it('window Escape is a no-op during IME composition', () => {
    const onStop = vi.fn();
    render(<StopOnEscape streaming ownedRunId="run-1" stopping={false} onStop={onStop} />);
    fireEvent.keyDown(window, { key: 'Escape', isComposing: true });
    expect(onStop).not.toHaveBeenCalled();
  });

  it('removes the window listener on unmount', () => {
    const onStop = vi.fn();
    const { unmount } = render(
      <StopOnEscape streaming ownedRunId="run-1" stopping={false} onStop={onStop} />,
    );
    unmount();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onStop).not.toHaveBeenCalled();
  });
});
