import dayjs from 'dayjs';

export function formatMessageTime(iso: string | null | undefined): string {
  if (!iso) return '';
  const t = dayjs(iso);
  if (!t.isValid()) return '';
  const now = dayjs();
  return t.isSame(now, 'day') ? t.format('HH:mm') : t.format('MM/DD HH:mm');
}

export function formatMessageTimeFull(iso: string | null | undefined): string {
  if (!iso) return '';
  const t = dayjs(iso);
  if (!t.isValid()) return '';
  return t.format('YYYY. M. D. HH:mm:ss');
}
