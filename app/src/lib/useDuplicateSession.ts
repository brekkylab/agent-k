import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { duplicateSession } from '@/api/sessions';
import { useToastStore } from '@/components/Toast';
import type { Session } from '@/domain/types';

type UseDuplicateSessionOptions = {
  navigateOnSuccess?: boolean;
};

export function useDuplicateSession({ navigateOnSuccess = false }: UseDuplicateSessionOptions = {}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const showToast = useToastStore((s) => s.show);

  return useMutation({
    mutationFn: (sessionId: string) => duplicateSession(sessionId),
    onSuccess: async (session: Session) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', session.projectId] });
      showToast('세션이 복제되었습니다');
      if (navigateOnSuccess) {
        navigate({
          to: '/projects/$projectId/sessions/$sessionId',
          params: { projectId: session.projectId, sessionId: session.id },
        });
      }
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(`세션 복제 실패: ${msg}`);
    },
  });
}
