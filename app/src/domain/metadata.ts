import type { IconName } from '../components/Icon';
import type { SessionIntent, ShareMode } from './types';

// Static, language-independent metadata for session intents and share modes.
// User-facing labels live in `app/src/i18n/locales/{en,ko}/common.json` under
// the `intent.*` and `share.*` keys; consumers should pull them via
// `useTranslation()` rather than reading them from here.
export const intentMeta: Record<SessionIntent, { icon: IconName }> = {
  general: { icon: 'general' },
  analysis: { icon: 'analysis' },
  brainstorm: { icon: 'brainstorm' },
  writing: { icon: 'writing' },
  recap: { icon: 'recap' },
};

export const shareMeta: Record<ShareMode, { icon: IconName; className: string }> = {
  private: { icon: 'lock', className: 'private' },
  shared_readonly: { icon: 'eye', className: 'readonly' },
  shared_chat: { icon: 'message-square', className: 'chat' },
};
