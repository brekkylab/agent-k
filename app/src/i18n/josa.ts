export type JosaPair = '을/를' | '이/가' | '으로/로' | '은/는' | '와/과';

const HANGUL_START = 0xac00;
const HANGUL_END = 0xd7a3;

const PAIR_MAP: Record<JosaPair, { withBatchim: string; without: string }> = {
  '을/를': { withBatchim: '을', without: '를' },
  '이/가': { withBatchim: '이', without: '가' },
  '은/는': { withBatchim: '은', without: '는' },
  '으로/로': { withBatchim: '으로', without: '로' },
  '와/과': { withBatchim: '과', without: '와' },
};

function hasBatchim(ch: string): boolean {
  if (!ch) return false;
  const code = ch.charCodeAt(0);
  if (code < HANGUL_START || code > HANGUL_END) return false;
  return (code - HANGUL_START) % 28 !== 0;
}

export function josa(noun: string, pair: JosaPair): string {
  const { withBatchim, without } = PAIR_MAP[pair];
  const last = noun.charAt(noun.length - 1);
  return noun + (hasBatchim(last) ? withBatchim : without);
}

/**
 * Localized noun decorator. Applies the Korean particle only when the active
 * language is ko; otherwise returns the noun unchanged. Centralizes the
 * `i18n.language === 'ko' ? josa(...) : noun` pattern that appears at every
 * filename-interpolation site.
 */
export function localizedNoun(noun: string, pair: JosaPair, lang: string): string {
  return lang === 'ko' ? josa(noun, pair) : noun;
}
