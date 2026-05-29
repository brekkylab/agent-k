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

export function forceLogout(opts: { reason?: LogoutReason; redirectTo?: string }): void {
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
