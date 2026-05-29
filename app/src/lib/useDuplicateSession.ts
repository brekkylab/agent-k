import { useMutation, useQueryClient } from '@tanstack/react-query';
import { duplicateSession } from '@/api/sessions';
import { useToastStore } from '@/components/Toast';
import type { Session } from '@/domain/types';

export function useDuplicateSession() {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  return useMutation({
    mutationFn: (sessionId: string) => duplicateSession(sessionId),
    onSuccess: async (session: Session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', session.projectId] });
      showToast('세션이 복제되었습니다');
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(`세션 복제 실패: ${msg}`);
    },
  });
}
