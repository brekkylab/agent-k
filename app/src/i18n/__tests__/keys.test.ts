import { describe, expect, it } from 'vitest';

import enCommon from '../locales/en/common.json';
import koCommon from '../locales/ko/common.json';
import enErrors from '../locales/en/errors.json';
import koErrors from '../locales/ko/errors.json';

type JsonShape = Record<string, unknown>;

function collectKeys(obj: JsonShape, prefix = ''): string[] {
  const keys: string[] = [];
  for (const [k, v] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${k}` : k;
    if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
      keys.push(...collectKeys(v as JsonShape, path));
    } else {
      keys.push(path);
    }
  }
  return keys.sort();
}

describe('translation key parity', () => {
  it('common: en and ko expose the same keys', () => {
    expect(collectKeys(enCommon)).toEqual(collectKeys(koCommon));
  });

  it('errors: en and ko expose the same keys', () => {
    expect(collectKeys(enErrors)).toEqual(collectKeys(koErrors));
  });
});
