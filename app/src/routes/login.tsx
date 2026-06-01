import { useState, useEffect, useCallback } from 'react';
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router';
import { getMe, login, signupAndLogin } from '@/api/auth';
import { getToken, ApiError } from '@/api/client';
import { useAuthStore } from '@/stores/auth';
import { WelcomeCarousel } from '@/components/WelcomeCarousel';
import { Icon } from '@/components/Icon';
import { consumeLogoutReason, consumeRedirectAfterLogin, type LogoutReason } from '@/lib/forceLogout';

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
      setCurrentUser(me);
      const redirectTo = consumeRedirectAfterLogin();
      navigate({ to: redirectTo ?? '/projects' });
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
        {/* 옵션 A: session-expired toast를 카드 안 상단에 inline으로. brand 위에
           먼저 보여서 "왜 다시 로그인해야 하는지" 컨텍스트가 폼보다 먼저 닿는다. */}
        {expiredReason && <SessionExpiredBanner reason={expiredReason} onDismiss={dismissExpiredReason} />}
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
                aria-label={showPassword ? '비밀번호 숨기기' : '비밀번호 보이기'}
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

function SessionExpiredBanner({ reason, onDismiss }: { reason: LogoutReason; onDismiss: () => void }) {
  useEffect(() => {
    const t = setTimeout(onDismiss, 5000);
    return () => clearTimeout(t);
  }, [onDismiss]);

  const message = reason === 'expired'
    ? '세션이 만료되어 다시 로그인이 필요합니다.'
    : '인증 정보가 유효하지 않습니다. 다시 로그인해 주세요.';

  return (
    <div className="cw-welcome-notice" role="status" aria-live="polite">
      <span>{message}</span>
      <button
        type="button"
        className="cw-welcome-notice-close"
        aria-label="닫기"
        onClick={onDismiss}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
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
