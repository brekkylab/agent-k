import { appWs } from '@/api/ws';
import { useAuthStore } from '@/stores/auth';

export type LogoutReason = 'expired' | 'invalid' | 'manual';

const REASON_KEY = 'auth.expiredReason';
const REDIRECT_KEY = 'auth.redirectAfterLogin';

// Router reference injected from main.tsx to avoid circular imports.
// Using router.navigate instead of window.location.href prevents a full-page
// reload that would race with TanStack Router's own client-side redirect and
// consume the sessionStorage items before the login page can read them.
let _routerNavigate: ((to: string) => void) | null = null;
export function setLogoutRouter(navigate: (to: string) => void): void {
  _routerNavigate = navigate;
}

// Module-level guard so simultaneous 401s (e.g. SSE + REST both racing through
// `notifyUnauthorized` at the moment of token expiry) only trigger one logout.
// Without this, the second caller can overwrite the REASON_KEY ('expired' →
// 'invalid'), surfacing the wrong banner to the user on the login page.
let logoutInProgress = false;

export function resetLogoutGuard(): void {
  logoutInProgress = false;
}

export function forceLogout(opts: { reason?: LogoutReason; redirectTo?: string }): void {
  if (logoutInProgress) return;
  logoutInProgress = true;

  appWs.disconnect();
  useAuthStore.getState().reset();

  if (opts.reason && opts.reason !== 'manual') {
    sessionStorage.setItem(REASON_KEY, opts.reason);
  }

  // Guard against open redirect: only store same-origin paths.
  const redirectTo = opts.redirectTo;
  if (redirectTo && redirectTo.startsWith('/') && !redirectTo.startsWith('//') && redirectTo !== '/login') {
    sessionStorage.setItem(REDIRECT_KEY, redirectTo);
  }

  if (_routerNavigate) {
    _routerNavigate('/login');
  } else {
    window.location.href = '/login';
  }
}

export function consumeLogoutReason(): LogoutReason | null {
  const val = sessionStorage.getItem(REASON_KEY) as LogoutReason | null;
  if (val) sessionStorage.removeItem(REASON_KEY);
  return val;
}

export function consumeRedirectAfterLogin(): string | null {
  const val = sessionStorage.getItem(REDIRECT_KEY);
  if (val) sessionStorage.removeItem(REDIRECT_KEY);
  return val;
}
