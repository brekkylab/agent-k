import { request, setToken } from './client';
import type { BackendUser, LoginResponse } from './backend-types';
import { toUser } from './transformers';
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
export async function signupAndLogin(input: SignupInput): Promise<User> {
  await signup(input);
  return login({ username: input.username, password: input.password });
}

export async function getMe(): Promise<User> {
  const raw = await request<BackendUser>('/me');
  return toUser(raw);
}

export function logout(): void {
  setToken(null);
}
