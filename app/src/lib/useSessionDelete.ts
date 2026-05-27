// Shared session-delete mutation: delete + cache invalidation + the 403 copy, in
// one place. Callers pass onDeleted for their own follow-up (close a dialog,
// navigate away if the deleted session was the active one, etc).

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { deleteSession } from '@/api/sessions';
import { useToastStore } from '@/components/Toast';
import { shortSessionId } from '@/lib/sessionId';
import { ApiError } from '@/api/client';

export function useSessionDelete(
  projectRef: string,
  opts?: { onDeleted?: (deletedId: string) => void },
) {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  return useMutation({
    mutationFn: (sessionId: string) => deleteSession(sessionId),
    onSuccess: async (_, deletedId) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectRef] });
      // deletedId is the full UUID; invalidate both full-UUID and prefix-based keys.
      await queryClient.invalidateQueries({ queryKey: ['session', deletedId] });
      await queryClient.invalidateQueries({ queryKey: ['session', shortSessionId(deletedId)] });
      showToast('세션이 삭제되었습니다');
      opts?.onDeleted?.(deletedId);
    },
    onError: (err) => {
      const msg = err instanceof ApiError
        ? (err.status === 403 ? '삭제 권한이 없습니다 (creator 또는 project owner만 가능)' : err.message)
        : err instanceof Error ? err.message : 'delete failed';
      showToast(`세션 삭제 실패: ${msg}`);
    },
  });
}
