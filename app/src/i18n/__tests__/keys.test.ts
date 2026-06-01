import { describe, expect, it } from 'vitest';

import enAuth from '../locales/en/auth.json';
import koAuth from '../locales/ko/auth.json';
import enCommon from '../locales/en/common.json';
import koCommon from '../locales/ko/common.json';
import enDialogs from '../locales/en/dialogs.json';
import koDialogs from '../locales/ko/dialogs.json';
import enErrors from '../locales/en/errors.json';
import koErrors from '../locales/ko/errors.json';
import enFiles from '../locales/en/files.json';
import koFiles from '../locales/ko/files.json';
import enMembers from '../locales/en/members.json';
import koMembers from '../locales/ko/members.json';
import enProject from '../locales/en/project.json';
import koProject from '../locales/ko/project.json';
import enSession from '../locales/en/session.json';
import koSession from '../locales/ko/session.json';

import type { ShareMode } from '@/domain/types';

type JsonShape = Record<string, unknown>;

function getNested(obj: JsonShape, path: string): unknown {
  return path.split('.').reduce<unknown>(
    (acc, key) => (acc && typeof acc === 'object' ? (acc as JsonShape)[key] : undefined),
    obj,
  );
}

const SHARE_MODES: ShareMode[] = ['private', 'shared_readonly', 'shared_chat'];

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

const NAMESPACES: Array<{ name: string; en: JsonShape; ko: JsonShape }> = [
  { name: 'auth', en: enAuth, ko: koAuth },
  { name: 'common', en: enCommon, ko: koCommon },
  { name: 'dialogs', en: enDialogs, ko: koDialogs },
  { name: 'errors', en: enErrors, ko: koErrors },
  { name: 'files', en: enFiles, ko: koFiles },
  { name: 'members', en: enMembers, ko: koMembers },
  { name: 'project', en: enProject, ko: koProject },
  { name: 'session', en: enSession, ko: koSession },
];

describe('translation key parity', () => {
  it.each(NAMESPACES)('$name: en and ko expose the same keys', ({ en, ko }) => {
    expect(collectKeys(en)).toEqual(collectKeys(ko));
  });
});

// Dynamic key callsites like `t(\`intent.${intent}.label\`)` are not caught
// by the parity test alone — if the enum gains a value, both locale files
// might still match but the new key is missing from both. These exhaustive
// checks ensure every enum value is materialized in both locales.
describe('dynamic key exhaustiveness', () => {
  describe.each(SHARE_MODES)('share.%s', (mode) => {
    it.each(['label', 'short_label', 'desc'] as const)('en/ko define common.share.%s.%s', (field) => {
      const path = `share.${mode}.${field}`;
      expect(typeof getNested(enCommon, path)).toBe('string');
      expect(typeof getNested(koCommon, path)).toBe('string');
    });
  });
});
