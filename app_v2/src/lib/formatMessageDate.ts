import dayjs from 'dayjs';
import i18n from '@/i18n';

// `lng` is passed by callers (from useTranslation) so the component re-renders
// on language change; falls back to the global i18n language otherwise.
function isKorean(lng?: string): boolean {
  return (lng ?? i18n.language ?? 'en').startsWith('ko');
}

/**
 * Date only (no time), localized:
 *   ko → "7월 17일"   en → "Jul 17"
 * Empty string for missing/invalid input.
 */
export function formatMessageDate(iso: string | null | undefined, lng?: string): string {
  if (!iso) return '';
  const t = dayjs(iso);
  if (!t.isValid()) return '';
  return isKorean(lng) ? t.format('M월 D일') : t.format('MMM D');
}

/**
 * Fuller date for the hover tooltip, still no time:
 *   ko → "2026년 7월 17일"   en → "Jul 17, 2026"
 */
export function formatMessageDateFull(iso: string | null | undefined, lng?: string): string {
  if (!iso) return '';
  const t = dayjs(iso);
  if (!t.isValid()) return '';
  return isKorean(lng) ? t.format('YYYY년 M월 D일') : t.format('MMM D, YYYY');
}
