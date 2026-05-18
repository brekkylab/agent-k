import type { IconName } from '../components/Icon';
import type { SessionIntent, ShareMode } from './types';

export const intentMeta: Record<SessionIntent, { label: string; icon: IconName; note: string }> = {
  general: { label: '일반', icon: 'general', note: '자유롭게 시작' },
  analysis: { label: '분석', icon: 'analysis', note: '자료 기반 판단' },
  brainstorm: { label: '브레인스토밍', icon: 'brainstorm', note: '옵션 탐색' },
  writing: { label: '작성', icon: 'writing', note: '초안 생성' },
  recap: { label: '정리', icon: 'recap', note: '결정/요약' },
};

export const shareMeta: Record<ShareMode, { label: string; shortLabel: string; icon: IconName; className: string; desc: string }> = {
  private: { label: '비공개', shortLabel: '비공개', icon: 'lock', className: 'private', desc: '나만 봐요' },
  shared_readonly: { label: '읽기 공유', shortLabel: '읽기', icon: 'eye', className: 'readonly', desc: '팀은 읽을 수 있어요' },
  shared_chat: { label: '함께 대화', shortLabel: '대화', icon: 'message-square', className: 'chat', desc: '팀과 AI가 함께 답해요' },
};
