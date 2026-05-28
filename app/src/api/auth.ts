import { request, setToken } from './client';
import type { BackendUser, LoginResponse } from './backend-types';
import { toUser } from './transformers';
import i18n, { SUPPORTED_LANGUAGES, type SupportedLanguage } from '@/i18n';
import type { User } from '@/domain/types';

export async function login(input: { username: string; password: string }): Promise<User> {
  const res = await request<LoginResponse>('/auth/login', {
    method: 'POST',
    body: { username: input.username, password: input.password },
    skipAuth: true,
  });
  setToken(res.access_token);
  return toUser(res.user);
}

export interface SignupInput {
  username: string;
  password: string;
  displayName?: string;
}

// Signup creates the account (and a personal project on the backend) but does
// NOT return a token. Callers usually chain `login()` immediately after to
// drop the user straight into /projects.
export async function signup(input: SignupInput): Promise<User> {
  const raw = await request<BackendUser>('/auth/signup', {
    method: 'POST',
    body: {
      username: input.username,
      password: input.password,
      display_name: input.displayName?.trim() || null,
    },
    skipAuth: true,
  });
  return toUser(raw);
}

// Convenience: signup then login in one shot so the UI gets a usable token.
// New users get `preferred_language='en'` from the DB default. If the browser
// language detector picked a different supported language, sync that to the
// backend before returning so the auth store doesn't flip the UI on hydrate.
export async function signupAndLogin(input: SignupInput): Promise<User> {
  await signup(input);
  const user = await login({ username: input.username, password: input.password });

  const detected = i18n.language;
  const isSupported = (SUPPORTED_LANGUAGES as readonly string[]).includes(detected);
  if (isSupported && detected !== user.preferredLanguage) {
    return updateMe({ preferredLanguage: detected as SupportedLanguage });
  }
  return user;
}

export async function getMe(): Promise<User> {
  const raw = await request<BackendUser>('/me');
  return toUser(raw);
}

export interface UpdateMeInput {
  displayName?: string;
  password?: string;
  currentPassword?: string;
  preferredLanguage?: 'en' | 'ko';
}

export async function updateMe(input: UpdateMeInput): Promise<User> {
  const body: Record<string, unknown> = {};
  if (input.displayName !== undefined) body.display_name = input.displayName;
  if (input.password !== undefined) body.password = input.password;
  if (input.currentPassword !== undefined) body.current_password = input.currentPassword;
  if (input.preferredLanguage !== undefined) body.preferred_language = input.preferredLanguage;

  const raw = await request<BackendUser>('/me', { method: 'PATCH', body });
  return toUser(raw);
}

export function logout(): void {
  setToken(null);
}
