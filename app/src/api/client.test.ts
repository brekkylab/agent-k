import { describe, it, expect, beforeEach } from 'vitest';
import { setUnauthorizedHandler, notifyUnauthorized } from './client';

describe('notifyUnauthorized reason classification', () => {
  let receivedReason: string | undefined;

  beforeEach(() => {
    receivedReason = undefined;
    setUnauthorizedHandler((reason) => { receivedReason = reason; });
  });

  it('classifies JWT_EXPIRED_MESSAGE as "expired"', () => {
    notifyUnauthorized(401, { error: 'token has expired' });
    expect(receivedReason).toBe('expired');
  });

  it('classifies any other message as "invalid"', () => {
    notifyUnauthorized(401, { error: 'unauthorized' });
    expect(receivedReason).toBe('invalid');
  });

  it('classifies missing error field as "invalid"', () => {
    notifyUnauthorized(401, {});
    expect(receivedReason).toBe('invalid');
  });

  it('does not fire for non-401 status', () => {
    notifyUnauthorized(403, { error: 'token has expired' });
    expect(receivedReason).toBeUndefined();
  });

  it('does not fire when skipAuth is true', () => {
    notifyUnauthorized(401, { error: 'token has expired' }, true);
    expect(receivedReason).toBeUndefined();
  });
});
