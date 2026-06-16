/** @vitest-environment jsdom */
import { describe, it, expect } from 'vitest';
import { renderHook } from '@testing-library/react';

import type { User } from '@/domain/types';
import { ALL_MENTION_SENTINEL } from '@/lib/mentionAll';

import { useMentionCommand } from './useMentionCommand';

const user = (id: string, name: string, username: string): User => ({
  id,
  name,
  username,
  roleLabel: 'Member',
  avatar: name[0],
  color: '#000',
  preferredLanguage: 'en',
});

const members = [user('u1', 'Olive Park', 'olive'), user('u2', 'Milo Chen', 'milo')];

describe('useMentionCommand — everyone item', () => {
  it('omits the everyone item when allMention is not provided', () => {
    const { result } = renderHook(() => useMentionCommand({ members, emptyLabel: 'none' }));
    expect(result.current.getItems('').some((i) => i.id === ALL_MENTION_SENTINEL)).toBe(false);
  });

  it('shows the everyone item on an empty query and on "all"/"모두"', () => {
    const { result } = renderHook(() =>
      useMentionCommand({ members, emptyLabel: 'none', allMention: { label: 'Everyone', token: '모두' } }),
    );
    expect(result.current.getItems('')[0]?.id).toBe(ALL_MENTION_SENTINEL);
    expect(result.current.getItems('all')[0]?.id).toBe(ALL_MENTION_SENTINEL);
    expect(result.current.getItems('모두')[0]?.id).toBe(ALL_MENTION_SENTINEL);
    // A specific member query should not surface the everyone item.
    expect(result.current.getItems('milo').some((i) => i.id === ALL_MENTION_SENTINEL)).toBe(false);
  });

  it('inserts the configured locale literal (tracks locale, not the query text)', () => {
    const ko = renderHook(() =>
      useMentionCommand({ members, emptyLabel: 'none', allMention: { label: '모두', token: '모두' } }),
    );
    const koItem = ko.result.current.getItems('all')[0]; // queried in English…
    expect(ko.result.current.onSelect(koItem!)).toEqual({ replaceWith: '@모두 ' }); // …still inserts ko token

    const en = renderHook(() =>
      useMentionCommand({ members, emptyLabel: 'none', allMention: { label: 'Everyone', token: 'all' } }),
    );
    const enItem = en.result.current.getItems('모두')[0];
    expect(en.result.current.onSelect(enItem!)).toEqual({ replaceWith: '@all ' });
  });

  it('still inserts @username for a regular member pick', () => {
    const { result } = renderHook(() =>
      useMentionCommand({ members, emptyLabel: 'none', allMention: { label: 'Everyone', token: 'all' } }),
    );
    const milo = result.current.getItems('milo').find((i) => i.id === 'u2');
    expect(result.current.onSelect(milo!)).toEqual({ replaceWith: '@milo ' });
  });
});
