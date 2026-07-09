import { describe, it, expect, beforeEach } from 'vitest';
import {
  setPendingAttachment,
  takePendingAttachment,
} from '@/stores/pendingAttachment';

// Reset module state between tests by taking any leftover value.
beforeEach(() => {
  takePendingAttachment();
});

describe('pendingAttachment store', () => {
  it('returns null when nothing has been set', () => {
    expect(takePendingAttachment()).toBeNull();
  });

  it('set → take returns the value exactly once', () => {
    setPendingAttachment({ name: 'report.pdf', sharedPath: '/root/shared/reports/report.pdf' });
    const result = takePendingAttachment();
    expect(result).toEqual({ name: 'report.pdf', sharedPath: '/root/shared/reports/report.pdf' });
  });

  it('second take returns null (read-and-clear)', () => {
    setPendingAttachment({ name: 'data.csv', sharedPath: '/root/shared/data.csv' });
    takePendingAttachment(); // consume
    expect(takePendingAttachment()).toBeNull();
  });

  it('overwrites a previous value with the latest set', () => {
    setPendingAttachment({ name: 'first.pdf', sharedPath: '/root/shared/first.pdf' });
    setPendingAttachment({ name: 'second.pdf', sharedPath: '/root/shared/second.pdf' });
    const result = takePendingAttachment();
    expect(result?.name).toBe('second.pdf');
  });

  it('StrictMode-safe: multiple takes after one set yield value then null', () => {
    // Simulates: StrictMode discards first render → effect runs twice.
    // First run gets the value; second run (cleanup + re-run) should get null.
    setPendingAttachment({ name: 'q2.pdf', sharedPath: '/root/shared/q2.pdf' });
    const first = takePendingAttachment();
    const second = takePendingAttachment();
    expect(first).not.toBeNull();
    expect(second).toBeNull();
  });
});
