import { useState, useEffect, useCallback } from 'react';
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getMe, login, signupAndLogin } from '@/api/auth';
import { getToken } from '@/api/client';
import { apiErrorToMessage } from '@/api/error-messages';
import { loadNs } from '@/i18n/loader';
import { useAuthStore } from '@/stores/auth';
import { WelcomeCarousel } from '@/components/WelcomeCarousel';
import { Icon } from '@/components/Icon';
import { consumeLogoutReason, consumeRedirectAfterLogin, type LogoutReason } from '@/lib/forceLogout';

type Mode = 'login' | 'signup';

export const Route = createFileRoute('/login')({
  beforeLoad: () => {
    if (getToken()) throw redirect({ to: '/projects' });
  },
  // Login page consumes `auth` + `errors` only. `common` is intentionally
  // skipped here so the auth screen stays as light as possible.
  loader: () => loadNs('auth', 'errors'),
  component: LoginPage,
});

function LoginPage() {
  return (
    <div className="cw-welcome">
      <aside className="cw-welcome-showcase">
        <WelcomeCarousel />
      </aside>
      <AuthPanel />
    </div>
  );
}

function AuthPanel() {
  // Both ns are guaranteed by the route loader; `useTranslation` is purely
  // for the `t` binding here.
  const { t } = useTranslation(['auth', 'errors']);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);
  const [mode, setMode] = useState<Mode>('login');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [expiredReason, setExpiredReason] = useState<LogoutReason | null>(null);
  const dismissExpiredReason = useCallback(() => setExpiredReason(null), []);

  useEffect(() => {
    const reason = consumeLogoutReason();
    if (reason) setExpiredReason(reason);
  }, []);

  function switchMode(next: Mode) {
    setMode(next);
    setError(null);
  }

  async function onSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      if (mode === 'login') {
        await login({ username, password });
      } else {
        await signupAndLogin({ username, password, displayName });
      }
      const me = await getMe();
      // Prime the ['me'] cache so /_app's useQuery hits cache instead of
      // refetching the identical payload right after this navigation.
      queryClient.setQueryData(['me'], me);
      setCurrentUser(me);
      const redirectTo = consumeRedirectAfterLogin();
      navigate({ to: redirectTo ?? '/projects' });
    } catch (err) {
      const scope = mode === 'signup' ? 'auth_signup' : 'auth_login';
      const fallbackKey = mode === 'signup'
        ? 'errors.fallback_signup'
        : 'errors.fallback_login';
      const { key, params, fallback } = apiErrorToMessage(err, scope);
      setError(t(key, { ...params, defaultValue: fallback ?? t(fallbackKey) }));
    } finally {
      setSubmitting(false);
    }
  }

  const isSignup = mode === 'signup';

  return (
    <main className="cw-welcome-panel">
      <div className="cw-welcome-card">
        {/* session-expired toast lives inline at the top of the card —
            "왜 다시 로그인해야 하는지" context lands before the form does. */}
        {expiredReason && <SessionExpiredBanner reason={expiredReason} onDismiss={dismissExpiredReason} />}
        <span className="cw-welcome-brand">Cowork for Teams</span>
        <h2 className="cw-welcome-card-title">
          {isSignup ? t('welcome.title_signup') : t('welcome.title_login')}
        </h2>
        <p className="cw-welcome-card-sub">
          {isSignup ? t('welcome.subtitle_signup') : t('welcome.subtitle_login')}
        </p>

        <div role="group" aria-label={t('welcome.tabs_aria')} className="cw-welcome-tabs">
          <ModeTab active={!isSignup} onClick={() => switchMode('login')}>{t('modes.login')}</ModeTab>
          <ModeTab active={isSignup} onClick={() => switchMode('signup')}>{t('modes.signup')}</ModeTab>
        </div>

        <form onSubmit={onSubmit}>
          <label>
            {t('fields.username')}
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              autoFocus
              required
            />
          </label>
          {isSignup && (
            <label>
              {t('fields.display_name')} <span className="cw-welcome-optional">{t('fields.display_name_optional')}</span>
              <input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={t('fields.display_name_placeholder')}
              />
            </label>
          )}
          <label>
            {t('fields.password')}
            <div className="cw-input-with-toggle">
              <input
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={isSignup ? 'new-password' : 'current-password'}
                required
              />
              <button
                type="button"
                className="cw-input-toggle"
                aria-label={showPassword ? t('welcome.password_hide') : t('welcome.password_show')}
                aria-pressed={showPassword}
                onClick={() => setShowPassword((v) => !v)}
                tabIndex={-1}
              >
                <Icon name={showPassword ? 'eye-off' : 'eye'} size={16} />
              </button>
            </div>
          </label>
          {error && <div className="cw-form-error">{error}</div>}
          <button type="submit" className="cw-btn-primary wide" disabled={submitting}>
            {submitting
              ? (isSignup ? t('submit.signup_busy') : t('submit.login_busy'))
              : (isSignup ? t('submit.signup') : t('submit.login'))}
          </button>
        </form>

        <p className="cw-welcome-switch">
          {isSignup ? (
            <>{t('switch.to_login_prefix')} <ModeLink onClick={() => switchMode('login')}>{t('switch.to_login_link')}</ModeLink>{t('switch.to_login_suffix')}</>
          ) : (
            <>{t('switch.to_signup_prefix')} <ModeLink onClick={() => switchMode('signup')}>{t('switch.to_signup_link')}</ModeLink>{t('switch.to_signup_suffix')}</>
          )}
        </p>
      </div>
    </main>
  );
}

function ModeTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      className="cw-welcome-tab"
      data-active={active}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function ModeLink({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" className="cw-welcome-link" onClick={onClick}>
      {children}
    </button>
  );
}

function SessionExpiredBanner({ reason, onDismiss }: { reason: LogoutReason; onDismiss: () => void }) {
  const { t } = useTranslation('auth');

  useEffect(() => {
    const timer = setTimeout(onDismiss, 5000);
    return () => clearTimeout(timer);
  }, [onDismiss]);

  const message = reason === 'expired'
    ? t('session_expired.expired')
    : t('session_expired.invalid');

  return (
    <div className="cw-welcome-notice" role="status" aria-live="polite">
      <span>{message}</span>
      <button
        type="button"
        className="cw-welcome-notice-close"
        aria-label={t('session_expired.dismiss')}
        onClick={onDismiss}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}
