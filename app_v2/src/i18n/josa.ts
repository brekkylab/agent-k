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

// Jongseong index 8 is ㄹ (리을). The '으로/로' particle treats ㄹ as
// non-batchim, so 서울 + 으로/로 → '서울로' (not '서울으로'); same for 파일로,
// 마실로, etc. Other pairs follow the standard batchim/no-batchim split.
const RIEUL_JONGSEONG = 8;

function jongseongIndex(ch: string): number {
  if (!ch) return 0;
  const code = ch.charCodeAt(0);
  if (code < HANGUL_START || code > HANGUL_END) return 0;
  return (code - HANGUL_START) % 28;
}

function hasBatchim(ch: string, pair: JosaPair): boolean {
  const jong = jongseongIndex(ch);
  if (jong === 0) return false;
  if (pair === '으로/로' && jong === RIEUL_JONGSEONG) return false;
  return true;
}

export function josa(noun: string, pair: JosaPair): string {
  const { withBatchim, without } = PAIR_MAP[pair];
  const last = noun.charAt(noun.length - 1);
  return noun + (hasBatchim(last, pair) ? withBatchim : without);
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
