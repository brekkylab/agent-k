import { useState } from 'react';
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getMe, login, signupAndLogin } from '@/api/auth';
import { getBaseUrl, setBaseUrl, getToken } from '@/api/client';
import { apiErrorToMessage } from '@/api/error-messages';
import { loadNs } from '@/i18n/loader';
import { useAuthStore } from '@/stores/auth';

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
  // Both ns are guaranteed by the route loader; `useTranslation` is purely
  // for the `t` binding here.
  const { t } = useTranslation(['auth', 'errors']);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);
  const [mode, setMode] = useState<Mode>('login');
  const [baseUrl, setUrl] = useState(getBaseUrl());
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  function switchMode(next: Mode) {
    setMode(next);
    setError(null);
  }

  async function onSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      setBaseUrl(baseUrl);
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
      navigate({ to: '/projects' });
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
    <div className="cw-live-login">
      <div className="cw-live-login-card">
        <h1>{t('card.title')}</h1>
        <p style={{ color: 'var(--cw-ink-3)', marginTop: 0 }}>
          {isSignup ? t('card.tagline_signup') : t('card.tagline_login')}
        </p>

        <div role="tablist" aria-label="auth mode" style={{
          display: 'inline-flex',
          gap: 4,
          padding: 4,
          marginTop: 6,
          marginBottom: 14,
          background: 'var(--cw-paper-3)',
          borderRadius: 999,
        }}>
          <ModeTab active={!isSignup} onClick={() => switchMode('login')}>{t('modes.login')}</ModeTab>
          <ModeTab active={isSignup} onClick={() => switchMode('signup')}>{t('modes.signup')}</ModeTab>
        </div>

        <form onSubmit={onSubmit}>
          <label>
            {t('fields.backend_url')}
            <input value={baseUrl} onChange={(e) => setUrl(e.target.value)} placeholder={t('fields.backend_url_placeholder')} />
          </label>
          <label>
            {t('fields.username')}
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete={isSignup ? 'username' : 'username'}
              autoFocus
              required
            />
          </label>
          {isSignup && (
            <label>
              {t('fields.display_name')} <span style={{ fontWeight: 400, color: 'var(--cw-ink-4)', textTransform: 'none', letterSpacing: 0 }}>{t('fields.display_name_optional')}</span>
              <input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={t('fields.display_name_placeholder')}
              />
            </label>
          )}
          <label>
            {t('fields.password')}
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete={isSignup ? 'new-password' : 'current-password'}
              required
            />
          </label>
          {error && <div className="cw-live-login-error">{error}</div>}
          <button type="submit" className="cw-btn-primary wide" disabled={submitting}>
            {submitting
              ? (isSignup ? t('submit.signup_busy') : t('submit.login_busy'))
              : (isSignup ? t('submit.signup') : t('submit.login'))}
          </button>
        </form>

        <p style={{ color: 'var(--cw-ink-3)', fontSize: 12, marginTop: 18 }}>
          {isSignup ? (
            <>{t('switch.to_login_prefix')} <ModeLink onClick={() => switchMode('login')}>{t('switch.to_login_link')}</ModeLink></>
          ) : (
            <>
              {t('switch.to_signup_prefix')} <ModeLink onClick={() => switchMode('signup')}>{t('switch.to_signup_link')}</ModeLink>{t('switch.to_signup_suffix')}
              {' '}{t('switch.demo_label')} <code>olive / cowork-demo</code>
            </>
          )}
        </p>
      </div>
    </div>
  );
}

function ModeTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      style={{
        appearance: 'none',
        border: 0,
        background: active ? 'var(--cw-paper)' : 'transparent',
        color: active ? 'var(--cw-ink)' : 'var(--cw-ink-3)',
        padding: '6px 14px',
        borderRadius: 999,
        fontSize: 12.5,
        fontWeight: active ? 600 : 500,
        boxShadow: active ? 'var(--cw-shadow-sm)' : 'none',
        cursor: 'pointer',
        transition: 'background 120ms, color 120ms',
        fontFamily: 'inherit',
      }}
    >
      {children}
    </button>
  );
}

function ModeLink({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        appearance: 'none',
        border: 0,
        background: 'transparent',
        padding: 0,
        color: 'var(--cw-accent)',
        textDecoration: 'underline',
        textUnderlineOffset: 2,
        cursor: 'pointer',
        fontSize: 'inherit',
        fontFamily: 'inherit',
      }}
    >
      {children}
    </button>
  );
}
