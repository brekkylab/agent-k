import type { IconName } from '../components/Icon';
import type { ShareMode } from './types';

export const shareMeta: Record<ShareMode, { label: string; shortLabel: string; icon: IconName; className: string; desc: string }> = {
  private: { label: '비공개', shortLabel: '비공개', icon: 'lock', className: 'private', desc: '나만 봐요' },
  shared_readonly: { label: '읽기 공유', shortLabel: '읽기', icon: 'eye', className: 'readonly', desc: '팀은 읽을 수 있어요' },
  shared_chat: { label: '함께 대화', shortLabel: '대화', icon: 'message-square', className: 'chat', desc: '팀과 AI가 함께 답해요' },
};
