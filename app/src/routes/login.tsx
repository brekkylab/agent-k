import { useState } from 'react';
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router';
import { getMe, login, signupAndLogin } from '@/api/auth';
import { getBaseUrl, setBaseUrl, getToken, ApiError } from '@/api/client';
import { useAuthStore } from '@/stores/auth';

type Mode = 'login' | 'signup';

export const Route = createFileRoute('/login')({
  beforeLoad: () => {
    if (getToken()) throw redirect({ to: '/projects' });
  },
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
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
      setCurrentUser(me);
      navigate({ to: '/projects' });
    } catch (err) {
      setError(messageOf(err, mode));
    } finally {
      setSubmitting(false);
    }
  }

  const isSignup = mode === 'signup';

  return (
    <div className="cw-live-login">
      <div className="cw-live-login-card">
        <h1>Cowork for Teams</h1>
        <p style={{ color: 'var(--cw-ink-3)', marginTop: 0 }}>
          {isSignup
            ? '새 계정을 만듭니다. 가입 후 personal project가 자동으로 생성됩니다.'
            : '로그인하여 시작하세요.'}
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
          <ModeTab active={!isSignup} onClick={() => switchMode('login')}>로그인</ModeTab>
          <ModeTab active={isSignup} onClick={() => switchMode('signup')}>회원가입</ModeTab>
        </div>

        <form onSubmit={onSubmit}>
          <label>
            Backend URL
            <input value={baseUrl} onChange={(e) => setUrl(e.target.value)} placeholder="http://127.0.0.1:8080" />
          </label>
          <label>
            Username
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
              Display name <span style={{ fontWeight: 400, color: 'var(--cw-ink-4)', textTransform: 'none', letterSpacing: 0 }}>(선택)</span>
              <input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder="팀원들이 보게 될 이름"
              />
            </label>
          )}
          <label>
            Password
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
              ? (isSignup ? '가입 중…' : '로그인 중…')
              : (isSignup ? '회원가입 후 시작' : '로그인')}
          </button>
        </form>

        <p style={{ color: 'var(--cw-ink-3)', fontSize: 12, marginTop: 18 }}>
          {isSignup ? (
            <>이미 계정이 있다면 <ModeLink onClick={() => switchMode('login')}>로그인</ModeLink>으로 돌아가세요.</>
          ) : (
            <>처음이세요? <ModeLink onClick={() => switchMode('signup')}>회원가입</ModeLink>으로 시작할 수 있어요. 데모 계정: <code>olive / cowork-demo</code></>
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

function messageOf(err: unknown, mode: Mode): string {
  if (err instanceof ApiError) {
    if (mode === 'signup') {
      if (err.status === 409) return '이미 사용 중인 username입니다. 다른 username을 시도해 주세요.';
      if (err.status === 422 || err.status === 400) return `입력 검증 실패: ${err.message}`;
    }
    if (mode === 'login') {
      if (err.status === 401) return '아이디 또는 비밀번호가 올바르지 않습니다.';
      if (err.status === 403) return '비활성화된 계정입니다.';
    }
    return `${err.status} — ${err.message}`;
  }
  if (err instanceof Error) return err.message;
  return mode === 'signup' ? 'Signup failed' : 'Login failed';
}
