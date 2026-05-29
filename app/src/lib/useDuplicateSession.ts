import { useMutation, useQueryClient } from '@tanstack/react-query';
import { duplicateSession } from '@/api/sessions';
import { ApiError } from '@/api/client';
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
      // 423: agent lock held — typically the worker hasn't released after a
      // cancel yet (heartbeat-driven, up to ~30s). Tell the user to retry.
      if (err instanceof ApiError && err.status === 423) {
        showToast('세션이 사용 중입니다. 잠시 후 다시 시도해주세요.');
        return;
      }
      const msg = err instanceof Error ? err.message : String(err);
      showToast(`세션 복제 실패: ${msg}`);
    },
  });
}
