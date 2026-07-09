// Module singleton for handing off a workspace file to the home composer.
// Using plain module-level state (no Zustand) — same pattern as stores/workspace.ts.
// takePendingAttachment() is read-and-clear so StrictMode double-invoke in the
// useEffect consumer is a safe no-op (second call gets null).

export interface PendingAttachment {
  /** Display name shown in the chip (entry.title). */
  name: string;
  /** Absolute shared-mount path, e.g. "/root/shared/reports/q2.pdf". */
  sharedPath: string;
}

let pending: PendingAttachment | null = null;

export function setPendingAttachment(a: PendingAttachment): void {
  pending = a;
}

/**
 * Reads and clears the pending attachment in one atomic step.
 * Returns null if no attachment is waiting.
 */
export function takePendingAttachment(): PendingAttachment | null {
  const a = pending;
  pending = null;
  return a;
}
