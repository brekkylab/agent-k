import { describe, expect, it } from 'vitest';

import { josa } from '../josa';

describe('josa', () => {
  describe('받침 있는 명사', () => {
    it.each([
      ['사진집', '을/를', '사진집을'],
      ['문서', '은/는', '문서는'],
      ['보고서', '으로/로', '보고서로'],
      ['컴퓨터', '와/과', '컴퓨터와'],
      ['집', '이/가', '집이'],
    ])('%s + %s = %s', (noun, pair, expected) => {
      expect(josa(noun, pair as Parameters<typeof josa>[1])).toBe(expected);
    });
  });

  describe('받침 없는 명사', () => {
    it.each([
      ['보고서', '을/를', '보고서를'],
      ['커피', '은/는', '커피는'],
      ['학교', '으로/로', '학교로'],
      ['친구', '와/과', '친구와'],
      ['나무', '이/가', '나무가'],
    ])('%s + %s = %s', (noun, pair, expected) => {
      expect(josa(noun, pair as Parameters<typeof josa>[1])).toBe(expected);
    });
  });

  describe('한글 외 문자로 끝나는 경우 (받침 없음으로 간주)', () => {
    it.each([
      ['report.pdf', '을/를', 'report.pdf를'],
      ['file', '을/를', 'file를'],
      ['v1.0', '을/를', 'v1.0를'],
      ['', '을/를', '를'],
    ])('%s + %s = %s', (noun, pair, expected) => {
      expect(josa(noun, pair as Parameters<typeof josa>[1])).toBe(expected);
    });
  });
});
