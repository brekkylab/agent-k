import { describe, it, expect } from 'vitest';

import { ALL_MENTION_SENTINEL, withAllMentionKeys, expandAllMentions } from './mentionAll';
import { scanMentions } from './mentionScan';

const OLIVE = '11111111-1111-4111-8111-111111111111';
const MILO = '22222222-2222-4222-8222-222222222222';
const OWEN = '33333333-3333-4333-8333-333333333333';

describe('withAllMentionKeys', () => {
  it('adds both reserved tokens to a clean roster map', () => {
    const map = withAllMentionKeys(new Map([['olive', OLIVE]]));
    expect(map.get('all')).toBe(ALL_MENTION_SENTINEL);
    expect(map.get('모두')).toBe(ALL_MENTION_SENTINEL);
    expect(map.get('olive')).toBe(OLIVE);
  });

  it('does NOT overwrite a member who owns "all" — they stay mentionable', () => {
    // Member literally named "all"; the everyone feature must not clobber them.
    const map = withAllMentionKeys(new Map([['all', OLIVE]]));
    expect(map.get('all')).toBe(OLIVE); // member preserved
    expect(map.get('모두')).toBe(ALL_MENTION_SENTINEL); // everyone still reachable
  });

  it('skips both tokens when both are owned (everyone feature unavailable)', () => {
    const map = withAllMentionKeys(new Map([['all', OLIVE], ['모두', MILO]]));
    expect(map.get('all')).toBe(OLIVE);
    expect(map.get('모두')).toBe(MILO);
  });

  it('does not mutate the input map', () => {
    const input = new Map([['olive', OLIVE]]);
    withAllMentionKeys(input);
    expect(input.has('all')).toBe(false);
    expect(input.has('모두')).toBe(false);
  });
});

describe('expandAllMentions', () => {
  const members = [OLIVE, MILO, OWEN];

  it('expands the sentinel to every member except the sender', () => {
    const { userIds, hadAll } = expandAllMentions([ALL_MENTION_SENTINEL], members, OLIVE);
    expect(userIds).toEqual([MILO, OWEN]);
    expect(hadAll).toBe(true);
  });

  it('dedups when an explicit mention co-occurs with the all-token', () => {
    const { userIds } = expandAllMentions([ALL_MENTION_SENTINEL, MILO], members, OLIVE);
    expect(userIds).toEqual([MILO, OWEN]); // milo not duplicated
  });

  it('passes explicit mentions through unchanged when no all-token (self not filtered)', () => {
    const { userIds, hadAll } = expandAllMentions([OLIVE, MILO], members, OLIVE);
    expect(userIds).toEqual([OLIVE, MILO]); // explicit self stays
    expect(hadAll).toBe(false);
  });

  it('never emits the sentinel in the output', () => {
    const { userIds } = expandAllMentions([ALL_MENTION_SENTINEL], members, OLIVE);
    expect(userIds).not.toContain(ALL_MENTION_SENTINEL);
  });

  it('returns empty ids but hadAll=true for a solo project (sender is the only member)', () => {
    const { userIds, hadAll } = expandAllMentions([ALL_MENTION_SENTINEL], [OLIVE], OLIVE);
    expect(userIds).toEqual([]);
    expect(hadAll).toBe(true);
  });
});

// Integration: how scanMentions behaves once the reserved keys are mixed in.
describe('scanMentions with all-keys', () => {
  it('recognizes both @all and @모두 as the all-sentinel', () => {
    const map = withAllMentionKeys(new Map([['olive', OLIVE]]));
    expect(scanMentions('@all ship it', map).userIds).toEqual([ALL_MENTION_SENTINEL]);
    expect(scanMentions('@모두 배포', map).userIds).toEqual([ALL_MENTION_SENTINEL]);
  });

  it('does not match @모두야 (한글 letter continues the identifier)', () => {
    const map = withAllMentionKeys(new Map([['olive', OLIVE]]));
    expect(scanMentions('@모두야', map).userIds).toEqual([]);
  });

  it('a member whose handle contains the token wins over the token', () => {
    // '@모두팀' must mention the member, not everyone (longest-first + consumed span).
    const map = withAllMentionKeys(new Map([['모두팀', MILO]]));
    const result = scanMentions('@모두팀 회의', map);
    expect(result.userIds).toEqual([MILO]);
    expect(result.userIds).not.toContain(ALL_MENTION_SENTINEL);
  });

  it('collapses duplicate @모두 to one sentinel id but keeps both highlight ranges', () => {
    const map = withAllMentionKeys(new Map([['olive', OLIVE]]));
    const result = scanMentions('@모두 @모두', map);
    expect(result.userIds).toEqual([ALL_MENTION_SENTINEL]);
    expect(result.ranges).toHaveLength(2);
  });
});
