import { describe, expect, it } from 'vitest';

import { scanMentions } from './mentionScan';

const candidates = (entries: Record<string, string>) => new Map(Object.entries(entries));

describe('scanMentions', () => {
  it('matches a mention at start, middle, and end of text', () => {
    const map = candidates({ jane: 'u1' });
    expect(scanMentions('@jane hello', map).userIds).toEqual(['u1']);
    expect(scanMentions('hello @jane hello', map).userIds).toEqual(['u1']);
    expect(scanMentions('hello @jane', map).userIds).toEqual(['u1']);
  });

  it('prefers the longest candidate when usernames overlap', () => {
    const map = candidates({ jeff: 'u1', jeffrey: 'u2' });
    expect(scanMentions('hi @jeffrey', map).userIds).toEqual(['u2']);
    expect(scanMentions('hi @jeff', map).userIds).toEqual(['u1']);
  });

  it('treats regex metacharacters in usernames literally', () => {
    const map = candidates({ 'a.b+c(': 'u1' });
    expect(scanMentions('ping @a.b+c( now', map).userIds).toEqual(['u1']);
    expect(scanMentions('ping @aXb+c( now', map).userIds).toEqual([]);
  });

  it('matches usernames containing spaces literally', () => {
    const map = candidates({ 'kim cs': 'u1' });
    expect(scanMentions('hey @kim cs check this', map).userIds).toEqual(['u1']);
  });

  it('matches Korean usernames', () => {
    const map = candidates({ 김철수: 'u1' });
    expect(scanMentions('@김철수 확인해주세요', map).userIds).toEqual(['u1']);
  });

  it('requires an identifier boundary after the match', () => {
    const map = candidates({ kim: 'u1' });
    // '-' continues an identifier, so this is someone else's handle.
    expect(scanMentions('@kim-cs', map).userIds).toEqual([]);
    // Sentence punctuation and end-of-text are boundaries.
    expect(scanMentions('@kim.', map).userIds).toEqual(['u1']);
    expect(scanMentions('@kim', map).userIds).toEqual(['u1']);
    expect(scanMentions('@kimchi', map).userIds).toEqual([]);
  });

  it("requires '@' at input start or after whitespace", () => {
    const map = candidates({ jeffrey: 'u1' });
    expect(scanMentions('john@jeffrey.com', map).userIds).toEqual([]);
  });

  it('does not report deleted or partially edited tokens', () => {
    const map = candidates({ jeffrey: 'u1' });
    expect(scanMentions('plain text', map).userIds).toEqual([]);
    expect(scanMentions('@jeffre', map).userIds).toEqual([]);
  });

  it('reports duplicate tokens for the same user once', () => {
    const map = candidates({ jane: 'u1' });
    expect(scanMentions('@jane and again @jane', map).userIds).toEqual(['u1']);
  });

  it('collects multiple distinct mentions with their usernames', () => {
    const map = candidates({ jane: 'u1', bob: 'u2' });
    const result = scanMentions('@jane please sync with @bob', map);
    expect(result.userIds).toEqual(expect.arrayContaining(['u1', 'u2']));
    expect(result.usernames).toEqual(expect.arrayContaining(['jane', 'bob']));
    expect(result.userIds).toHaveLength(2);
  });

  it('returns empty results for an empty candidate map', () => {
    expect(scanMentions('@jane hi', new Map()).userIds).toEqual([]);
  });
});
