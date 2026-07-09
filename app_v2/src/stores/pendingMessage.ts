// Hands the first user message from the home composer to the session view so it
// renders optimistically on arrival, instead of waiting for the SSE catch-up to
// echo it back (which only happens after the run's sandbox restore).
// Plain module singleton — same pattern as stores/workspace.ts / pendingAttachment.ts.
// takePendingMessage() is read-and-clear so a StrictMode double-invoked consumer
// is a safe no-op on the second call (gets null).

let pending: string | null = null;

export function setPendingMessage(text: string): void {
  pending = text;
}

/** Reads and clears the pending first message in one step. Null if none waiting. */
export function takePendingMessage(): string | null {
  const t = pending;
  pending = null;
  return t;
}
