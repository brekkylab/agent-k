import type { IconName } from '../components/Icon';
import type { ShareMode } from './types';

// Static, language-independent metadata for share modes. User-facing labels
// live in `app/src/i18n/locales/{en,ko}/common.json` under the `share.*` keys;
// consumers should pull them via `useTranslation()` rather than reading them
// from here.

export const shareMeta: Record<ShareMode, { icon: IconName; className: string }> = {
  private: { icon: 'lock', className: 'private' },
  shared_readonly: { icon: 'eye', className: 'readonly' },
  shared_chat: { icon: 'message-square', className: 'chat' },
};
