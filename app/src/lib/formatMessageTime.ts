import dayjs from 'dayjs';
import 'dayjs/locale/ko';
dayjs.locale('ko');

export function formatMessageTime(iso: string | null | undefined): string {
  if (!iso) return '';
  const t = dayjs(iso);
  if (!t.isValid()) return '';
  const now = dayjs();
  return t.isSame(now, 'day') ? t.format('HH:mm') : t.format('MM/DD HH:mm');
}
