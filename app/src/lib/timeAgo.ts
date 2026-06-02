import dayjs from 'dayjs';
import relativeTime from 'dayjs/plugin/relativeTime';
import 'dayjs/locale/ko';
import i18n from '@/i18n';

dayjs.extend(relativeTime);

function syncDayjsLocale(lng: string): void {
  // dayjs ships `en` by default; we import `ko` above. Any other language
  // falls back to `en`.
  dayjs.locale(lng === 'ko' ? 'ko' : 'en');
}

syncDayjsLocale(i18n.language || 'en');
i18n.on('languageChanged', syncDayjsLocale);

export function timeAgo(iso: string | null | undefined): string {
  if (!iso) return '';
  return dayjs(iso).fromNow();
}
