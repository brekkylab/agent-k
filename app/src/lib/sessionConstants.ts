/**
 * Non-displaying sentinel used to mark sessions without a backend-assigned
 * title. Kept in a hook-free module so both the data layer (`api/transformers.ts`)
 * and the UI layer (`useTypingText`) can import it without one having to
 * depend on the other.
 */
export const FALLBACK_TITLE = '__cw_untitled__';
