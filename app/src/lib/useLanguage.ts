import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation } from '@tanstack/react-query';

import { updateMe } from '@/api/auth';
import { LANGUAGE_STORAGE_KEY, SUPPORTED_LANGUAGES, type SupportedLanguage } from '@/i18n';
import { useAuthStore } from '@/stores/auth';

function normalize(lng: string): SupportedLanguage {
  return SUPPORTED_LANGUAGES.includes(lng as SupportedLanguage)
    ? (lng as SupportedLanguage)
    : 'en';
}

export function useLanguage() {
  const { i18n } = useTranslation();
  const currentUser = useAuthStore((s) => s.currentUser);
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);

  const mutation = useMutation({
    mutationFn: (lang: SupportedLanguage) => updateMe({ preferredLanguage: lang }),
    onSuccess: (user) => setCurrentUser(user),
    onError: (err) => {
      // Backend sync is a nice-to-have; the local UI already changed via
      // i18n.changeLanguage + localStorage cache. Surface the failure to
      // logs so it's debuggable, but don't propagate (otherwise the
      // toggle button would throw inside React Query's error boundary).
      console.warn('[i18n] failed to sync preferred_language to backend', err);
    },
  });
  const { mutate } = mutation;

  const setLanguage = useCallback(
    (next: SupportedLanguage) => {
      i18n.changeLanguage(next).catch((err) => {
        console.warn('[i18n] changeLanguage failed', err);
      });
      try {
        localStorage.setItem(LANGUAGE_STORAGE_KEY, next);
      } catch {
        // localStorage may be unavailable (private mode, SSR) — UI still updates.
      }
      if (currentUser) {
        mutate(next);
      }
    },
    [i18n, currentUser, mutate],
  );

  return {
    lang: normalize(i18n.resolvedLanguage ?? i18n.language),
    setLanguage,
    isUpdating: mutation.isPending,
  };
}
