import { useState } from 'react';
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router';
import { getMe, login, signupAndLogin } from '@/api/auth';
import { getToken, ApiError } from '@/api/client';
import { useAuthStore } from '@/stores/auth';
import { WelcomeCarousel } from '@/components/WelcomeCarousel';

type Mode = 'login' | 'signup';

export const Route = createFileRoute('/login')({
  beforeLoad: () => {
    if (getToken()) throw redirect({ to: '/projects' });
  },
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
  const navigate = useNavigate();
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);
  const [mode, setMode] = useState<Mode>('login');
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
    <main className="cw-welcome-panel">
      <div className="cw-welcome-card">
        <span className="cw-welcome-brand">Cowork for Teams</span>
        <h2 className="cw-welcome-card-title">{isSignup ? '계정 만들기' : '오늘은 무엇을 함께 만들까요?'}</h2>
        <p className="cw-welcome-card-sub">
          {isSignup
            ? '가입하면 개인 프로젝트가 자동으로 생성됩니다.'
            : '팀과 에이전트가 기다리고 있어요.'}
        </p>

        <div role="group" aria-label="로그인 또는 회원가입 선택" className="cw-welcome-tabs">
          <ModeTab active={!isSignup} onClick={() => switchMode('login')}>로그인</ModeTab>
          <ModeTab active={isSignup} onClick={() => switchMode('signup')}>회원가입</ModeTab>
        </div>

        <form onSubmit={onSubmit}>
          <label>
            Username
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
              Display name <span className="cw-welcome-optional">(선택)</span>
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
          {error && <div className="cw-form-error">{error}</div>}
          <button type="submit" className="cw-btn-primary wide" disabled={submitting}>
            {submitting
              ? (isSignup ? '가입 중…' : '로그인 중…')
              : (isSignup ? '회원가입 후 시작' : '로그인')}
          </button>
        </form>

        <p className="cw-welcome-switch">
          {isSignup ? (
            <>이미 계정이 있다면 <ModeLink onClick={() => switchMode('login')}>로그인</ModeLink>으로 돌아가세요.</>
          ) : (
            <>처음이세요? <ModeLink onClick={() => switchMode('signup')}>회원가입</ModeLink>으로 시작할 수 있어요.</>
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
