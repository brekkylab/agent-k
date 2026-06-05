// Maps API errors to i18n keys. The frontend owns the translation table —
// the backend always responds in English so unmapped errors can fall back
// to the raw message without breaking the UI.

import { ApiError } from './client';

export type ErrorScope = 'auth_login' | 'auth_signup' | 'generic';

export interface ErrorMessage {
  key: string;
  params?: Record<string, unknown>;
  /** English fallback shown when the key has no translation (e.g. unknown status). */
  fallback?: string;
}

export function apiErrorToMessage(
  err: unknown,
  scope: ErrorScope = 'generic',
): ErrorMessage {
  if (err instanceof ApiError) {
    if (scope === 'auth_login') {
      if (err.status === 401) return { key: 'auth:errors.login.invalid_credentials' };
      if (err.status === 403) return { key: 'auth:errors.login.deactivated' };
    }
    if (scope === 'auth_signup') {
      if (err.status === 409) return { key: 'auth:errors.signup.username_taken' };
      if (err.status === 422 || err.status === 400) {
        return { key: 'auth:errors.signup.validation', params: { message: err.message } };
      }
    }
    switch (err.status) {
      case 401: return { key: 'errors:http.401' };
      case 403: return { key: 'errors:http.403' };
      case 404: return { key: 'errors:http.404' };
      case 409: return { key: 'errors:http.409', fallback: err.message };
      case 400: return { key: 'errors:http.400', fallback: err.message };
      default:
        return {
          key: 'errors:http.unknown',
          params: { status: err.status },
          fallback: err.message,
        };
    }
  }
  if (err instanceof Error) {
    return { key: 'errors:unknown', fallback: err.message };
  }
  return { key: 'errors:unknown' };
}
